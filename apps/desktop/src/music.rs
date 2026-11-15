//! Background music playback from the application's `Music` folder.
//!
//! `MusicService` owns a worker thread. Its public methods only enqueue commands,
//! so directory scans, decoding, and output-device work never block the UI.

use std::{
    collections::HashSet,
    fs::File,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

enum Command {
    Enabled(bool),
    Volume(f32),
    Refresh,
    Stop,
}

/// Asynchronous background music controller.
///
/// Playback starts automatically when the service is created. Use `set_enabled`
/// to mute/pause while retaining the current position, and `set_volume` for the
/// independent music level. `refresh` rescans the folder and clears failed-file
/// exclusions. Worker errors are retrieved with `try_error`.
pub struct MusicService {
    commands: Sender<Command>,
    errors: Receiver<String>,
}

impl MusicService {
    pub fn new(music_directory: impl Into<PathBuf>, enabled: bool, volume: f32) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (error_tx, errors) = mpsc::channel();
        let directory = music_directory.into();
        thread::Builder::new()
            .name("staminator-music".into())
            .spawn(move || {
                music_worker(
                    command_rx,
                    error_tx,
                    directory,
                    enabled,
                    normalize_volume(volume),
                )
            })
            .expect("failed to start music worker");
        Self { commands, errors }
    }

    /// Enable or mute playback. Muting pauses the current track without losing position.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.send(Command::Enabled(enabled))
    }

    /// Set music volume from 0.0 to 1.0, independent from sound effects.
    pub fn set_volume(&self, volume: f32) -> Result<(), String> {
        self.send(Command::Volume(normalize_volume(volume)))
    }

    /// Rescan supported WAV and MP3 files. The current track is allowed to finish.
    pub fn refresh(&self) -> Result<(), String> {
        self.send(Command::Refresh)
    }

    /// Stop playback and terminate the worker when the service is dropped.
    pub fn stop(&self) -> Result<(), String> {
        self.send(Command::Stop)
    }

    pub fn try_error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "music worker has stopped".to_owned())
    }
}

fn normalize_volume(volume: f32) -> f32 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

struct Playlist {
    directory: PathBuf,
    files: Vec<PathBuf>,
    order: Vec<usize>,
    cursor: usize,
    failed: HashSet<PathBuf>,
    random_state: u64,
}

impl Playlist {
    fn new(directory: PathBuf) -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Self {
            directory,
            files: Vec::new(),
            order: Vec::new(),
            cursor: 0,
            failed: HashSet::new(),
            random_state: seed | 1,
        }
    }

    fn refresh(&mut self) -> std::io::Result<()> {
        let files = crate::audio_resources::AudioResources::files(&self.directory)?;
        if files != self.files {
            self.files = files;
            self.failed.clear();
            self.new_round();
        } else {
            // An explicit refresh also permits retrying files that failed earlier.
            self.failed.clear();
            self.new_round();
        }
        Ok(())
    }

    fn new_round(&mut self) {
        self.order = (0..self.files.len()).collect();
        // Fisher-Yates using a per-service xorshift PRNG.
        for end in (1..self.order.len()).rev() {
            self.random_state ^= self.random_state << 13;
            self.random_state ^= self.random_state >> 7;
            self.random_state ^= self.random_state << 17;
            self.order
                .swap(end, (self.random_state as usize) % (end + 1));
        }
        self.cursor = 0;
    }

    fn next(&mut self) -> Option<PathBuf> {
        while !self.files.is_empty() {
            if self.cursor >= self.order.len() {
                self.new_round();
            }
            let path = self.files[self.order[self.cursor]].clone();
            self.cursor += 1;
            if !self.failed.contains(&path) {
                return Some(path);
            }
            // If this whole round contained only bad files, do not retry forever.
            if self.cursor >= self.order.len()
                && self.files.iter().all(|file| self.failed.contains(file))
            {
                return None;
            }
        }
        None
    }

    fn mark_failed(&mut self, path: PathBuf) {
        self.failed.insert(path);
    }
}

trait Playback {
    fn start(&mut self, path: &Path, volume: f32) -> Result<(), PlaybackError>;
    fn ended(&self) -> bool;
    fn set_volume(&mut self, volume: f32);
    fn set_paused(&mut self, paused: bool);
    fn stop(&mut self);
}

#[derive(Debug)]
enum PlaybackError {
    File(String),
    Device(String),
}

struct RodioPlayback {
    stream: Option<rodio::OutputStream>,
    sink: Option<rodio::Sink>,
    volume: f32,
    paused: bool,
}

impl RodioPlayback {
    fn new() -> Self {
        Self {
            stream: None,
            sink: None,
            volume: 1.0,
            paused: false,
        }
    }
}

impl Playback for RodioPlayback {
    fn start(&mut self, path: &Path, volume: f32) -> Result<(), PlaybackError> {
        let file = File::open(path)
            .map_err(|e| PlaybackError::File(format!("cannot open {}: {e}", path.display())))?;
        let decoder = rodio::Decoder::try_from(file)
            .map_err(|e| PlaybackError::File(format!("cannot decode {}: {e}", path.display())))?;
        if self.stream.is_none() {
            let stream = rodio::OutputStreamBuilder::open_default_stream()
                .map_err(|e| PlaybackError::Device(format!("cannot open audio output: {e}")))?;
            self.stream = Some(stream);
        }
        // Stopping a Sink is terminal. Recreate it on the next start while
        // reusing the live output stream; otherwise reuse the empty Sink.
        if self.sink.is_none() {
            self.sink = Some(rodio::Sink::connect_new(
                self.stream.as_ref().unwrap().mixer(),
            ));
        }
        let sink = self.sink.as_ref().unwrap();
        sink.set_volume(volume);
        sink.append(decoder);
        if self.paused {
            sink.pause();
        }
        Ok(())
    }

    fn ended(&self) -> bool {
        self.sink.as_ref().is_none_or(rodio::Sink::empty)
    }
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }
    }
    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if let Some(sink) = &self.sink {
            if paused {
                sink.pause();
            } else {
                sink.play();
            }
        }
    }
    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
    }
}

fn music_worker(
    commands: Receiver<Command>,
    errors: Sender<String>,
    directory: PathBuf,
    enabled: bool,
    volume: f32,
) {
    run_worker(
        commands,
        errors,
        directory,
        enabled,
        volume,
        RodioPlayback::new(),
    );
}

fn run_worker(
    commands: Receiver<Command>,
    errors: Sender<String>,
    directory: PathBuf,
    mut enabled: bool,
    mut volume: f32,
    mut playback: impl Playback,
) {
    let mut playlist = Playlist::new(directory);
    playback.set_volume(volume);
    playback.set_paused(!enabled);
    let mut current: Option<PathBuf> = None;
    let mut pending: Option<PathBuf> = None;
    let mut retry_at: Option<Instant> = None;
    let mut device_error_reported = false;
    let mut scan_error_reported = false;
    loop {
        match commands.recv_timeout(Duration::from_millis(40)) {
            Ok(Command::Stop) => {
                playback.stop();
                break;
            }
            Ok(Command::Enabled(value)) => {
                enabled = value;
                playback.set_paused(!enabled);
            }
            Ok(Command::Volume(value)) => {
                volume = value;
                playback.set_volume(volume);
            }
            Ok(Command::Refresh) => {
                scan_error_reported = false;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                playback.stop();
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if current.is_some() && !playback.ended() {
            continue;
        }
        current = None;
        if !enabled {
            continue;
        }
        if retry_at.is_some_and(|retry| Instant::now() < retry) {
            continue;
        }
        if !scan_error_reported {
            match playlist.refresh() {
                Ok(()) => {
                    scan_error_reported = true;
                    if pending
                        .as_ref()
                        .is_some_and(|path| !playlist.files.contains(path))
                    {
                        pending = None;
                        retry_at = None;
                        device_error_reported = false;
                    }
                }
                Err(error) => {
                    let _ = errors.send(format!(
                        "cannot scan music directory {}: {error}",
                        playlist.directory.display()
                    ));
                    scan_error_reported = true;
                    continue;
                }
            }
        }
        while let Some(path) = pending.take().or_else(|| playlist.next()) {
            match playback.start(&path, volume) {
                Ok(()) => {
                    retry_at = None;
                    device_error_reported = false;
                    current = Some(path);
                    break;
                }
                Err(PlaybackError::File(error)) => {
                    playlist.mark_failed(path);
                    let _ = errors.send(error);
                }
                Err(PlaybackError::Device(error)) => {
                    pending = Some(path);
                    retry_at = Some(Instant::now() + Duration::from_secs(1));
                    if !device_error_reported {
                        let _ = errors.send(error);
                        device_error_reported = true;
                    }
                    break;
                }
            }
        }
        // Refresh on the next loop only on explicit request, not every poll.
        if current.is_none() {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, Sender},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn folder() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "staminator-music-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn shuffled_round_contains_every_track_once_then_starts_new_round() {
        let dir = folder();
        let mut playlist = Playlist::new(dir.clone());
        playlist.files = (0..5).map(|i| dir.join(format!("{i}.mp3"))).collect();
        playlist.new_round();
        let first: Vec<_> = (0..5).map(|_| playlist.next().unwrap()).collect();
        assert_eq!(first.iter().collect::<HashSet<_>>().len(), 5);
        let second: Vec<_> = (0..5).map(|_| playlist.next().unwrap()).collect();
        assert_eq!(second.iter().collect::<HashSet<_>>().len(), 5);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn volume_normalization_clamps_finite_values_and_defaults_non_finite_to_full() {
        assert_eq!(normalize_volume(-1.0), 0.0);
        assert_eq!(normalize_volume(0.4), 0.4);
        assert_eq!(normalize_volume(2.0), 1.0);
        assert_eq!(normalize_volume(f32::NAN), 1.0);
        assert_eq!(normalize_volume(f32::INFINITY), 1.0);
        assert_eq!(normalize_volume(f32::NEG_INFINITY), 1.0);
    }

    #[test]
    fn refresh_resets_failed_track_even_when_file_list_is_unchanged() {
        let dir = folder();
        let track = dir.join("track.wav");
        std::fs::write(&track, b"track").unwrap();
        let mut playlist = Playlist::new(dir.clone());
        playlist.refresh().unwrap();
        playlist.mark_failed(track.clone());
        assert_eq!(playlist.next(), None);
        playlist.refresh().unwrap();
        assert_eq!(playlist.next(), Some(track));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rodio_adapter_controls_and_file_errors_do_not_open_output_device() {
        let mut playback = RodioPlayback::new();
        assert!(playback.ended());
        playback.set_volume(0.25);
        playback.set_paused(true);
        assert_eq!(playback.volume, 0.25);
        assert!(playback.paused);
        playback.set_paused(false);
        playback.stop();
        assert!(!playback.paused);

        let missing = folder().join("missing.wav");
        let open_error = playback.start(&missing, 0.25).unwrap_err();
        assert!(
            matches!(open_error, PlaybackError::File(message) if message.contains("cannot open"))
        );
        let invalid = folder();
        let invalid_file = invalid.join("invalid.wav");
        std::fs::write(&invalid_file, b"not an audio stream").unwrap();
        let decode_error = playback.start(&invalid_file, 0.25).unwrap_err();
        assert!(
            matches!(decode_error, PlaybackError::File(message) if message.contains("cannot decode"))
        );
        std::fs::remove_dir_all(invalid).unwrap();
    }

    #[derive(Default)]
    struct FakeState {
        started: Vec<PathBuf>,
        ended: bool,
        paused: bool,
        volume: f32,
        attempts: Vec<PathBuf>,
        corrupt: HashSet<PathBuf>,
        device_failures: HashMap<PathBuf, usize>,
        stopped: bool,
    }
    struct FakePlayback {
        state: Arc<Mutex<FakeState>>,
        started: Sender<PathBuf>,
    }
    impl Playback for FakePlayback {
        fn start(&mut self, path: &Path, volume: f32) -> Result<(), PlaybackError> {
            let mut state = self.state.lock().unwrap();
            state.attempts.push(path.to_owned());
            if let Some(failures) = state.device_failures.get_mut(path)
                && *failures > 0
            {
                *failures -= 1;
                return Err(PlaybackError::Device("fake output unavailable".to_owned()));
            }
            if state.corrupt.contains(path)
                || std::fs::read(path).is_ok_and(|contents| contents == b"bad")
            {
                return Err(PlaybackError::File(format!(
                    "cannot decode {}",
                    path.display()
                )));
            }
            state.started.push(path.to_owned());
            state.volume = volume;
            state.ended = false;
            drop(state);
            let _ = self.started.send(path.to_owned());
            Ok(())
        }
        fn ended(&self) -> bool {
            self.state.lock().unwrap().ended
        }
        fn set_volume(&mut self, volume: f32) {
            self.state.lock().unwrap().volume = volume;
        }
        fn set_paused(&mut self, paused: bool) {
            self.state.lock().unwrap().paused = paused;
        }
        fn stop(&mut self) {
            self.state.lock().unwrap().stopped = true;
        }
    }

    type FakeWorker = (
        Sender<Command>,
        Receiver<String>,
        Receiver<PathBuf>,
        Arc<Mutex<FakeState>>,
        thread::JoinHandle<()>,
    );

    fn worker(dir: PathBuf, enabled: bool) -> FakeWorker {
        worker_with_state(dir, enabled, Arc::new(Mutex::new(FakeState::default())))
    }

    fn worker_with_state(dir: PathBuf, enabled: bool, state: Arc<Mutex<FakeState>>) -> FakeWorker {
        let (started_tx, started_rx) = mpsc::channel();
        let (commands_tx, commands_rx) = mpsc::channel();
        let (errors_tx, errors_rx) = mpsc::channel();
        let fake = FakePlayback {
            state: state.clone(),
            started: started_tx,
        };
        let worker =
            thread::spawn(move || run_worker(commands_rx, errors_tx, dir, enabled, 0.4, fake));
        (commands_tx, errors_rx, started_rx, state, worker)
    }

    fn wait_started(started: &Receiver<PathBuf>) -> PathBuf {
        started
            .recv_timeout(Duration::from_secs(5))
            .expect("worker did not start a track")
    }

    fn finish_track(state: &Mutex<FakeState>) {
        state.lock().unwrap().ended = true;
    }

    #[test]
    fn fake_adapter_observes_independent_controls_and_end_detection() {
        let dir = folder();
        for i in 0..2 {
            std::fs::write(dir.join(format!("{i}.wav")), b"x").unwrap();
        }
        let state = Arc::new(Mutex::new(FakeState::default()));
        let (started_tx, started_rx) = mpsc::channel();
        let (tx, rx) = mpsc::channel();
        let (etx, _erx) = mpsc::channel();
        let fake = FakePlayback {
            state: state.clone(),
            started: started_tx,
        };
        let worker_dir = dir.clone();
        let worker = thread::spawn(move || run_worker(rx, etx, worker_dir, true, 1.0, fake));
        tx.send(Command::Volume(0.3)).unwrap();
        wait_started(&started_rx);
        tx.send(Command::Enabled(false)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !state.lock().unwrap().paused {
            assert!(
                std::time::Instant::now() < deadline,
                "pause command was not applied"
            );
            thread::yield_now();
        }
        assert!(state.lock().unwrap().paused);
        finish_track(&state);
        tx.send(Command::Enabled(true)).unwrap();
        wait_started(&started_rx);
        assert_eq!(state.lock().unwrap().volume, 0.3);
        tx.send(Command::Stop).unwrap();
        worker.join().unwrap();
        assert!(state.lock().unwrap().stopped);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn disabled_start_waits_until_enabled() {
        let dir = folder();
        std::fs::write(dir.join("track.wav"), b"track").unwrap();
        let (commands, _errors, started, state, worker) = worker(dir.clone(), false);
        assert!(started.recv_timeout(Duration::from_millis(120)).is_err());
        commands.send(Command::Enabled(true)).unwrap();
        assert_eq!(wait_started(&started), dir.join("track.wav"));
        commands.send(Command::Stop).unwrap();
        worker.join().unwrap();
        assert!(state.lock().unwrap().stopped);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_file_is_skipped_and_reported_once() {
        let dir = folder();
        let bad = dir.join("bad.wav");
        let good = dir.join("good.wav");
        std::fs::write(&bad, b"bad").unwrap();
        std::fs::write(&good, b"good").unwrap();
        let (commands, errors, started, state, worker) = worker(dir.clone(), true);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut bad_error_seen = false;
        let mut good_starts = 0;
        while !(bad_error_seen && good_starts >= 3) {
            assert!(
                std::time::Instant::now() < deadline,
                "bad track was not skipped"
            );
            while let Ok(error) = errors.try_recv() {
                assert!(error.contains("bad.wav"), "unexpected error: {error}");
                assert!(!bad_error_seen, "worker repeated the decode error");
                bad_error_seen = true;
            }
            match started.recv_timeout(Duration::from_millis(10)) {
                Ok(path) => {
                    assert_eq!(path, good);
                    good_starts += 1;
                    // Finishing the playable track lets the worker reach the
                    // damaged one even when shuffle selected the good file first.
                    finish_track(&state);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => thread::yield_now(),
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("fake playback worker stopped"),
            }
        }
        // A bad track can only be attempted once in the current cycle.
        assert_eq!(
            state
                .lock()
                .unwrap()
                .attempts
                .iter()
                .filter(|path| *path == &bad)
                .count(),
            1
        );
        assert!(
            errors.recv_timeout(Duration::from_millis(120)).is_err(),
            "worker repeated the decode error"
        );
        commands.send(Command::Stop).unwrap();
        worker.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refresh_adds_files_and_tracks_continue_across_multiple_rounds() {
        let dir = folder();
        let first = dir.join("first.wav");
        let second = dir.join("second.wav");
        std::fs::write(&first, b"first").unwrap();
        let (commands, _errors, started, state, worker) = worker(dir.clone(), true);
        assert_eq!(wait_started(&started), first);
        std::fs::write(&second, b"second").unwrap();
        commands.send(Command::Refresh).unwrap();
        // The currently playing track belongs to the old list. Its completion
        // triggers the scan, so the next start is the first item of a new round.
        finish_track(&state);
        let mut refreshed_rounds = vec![wait_started(&started)];
        for _ in 0..3 {
            finish_track(&state);
            refreshed_rounds.push(wait_started(&started));
        }
        assert!(
            refreshed_rounds.contains(&second),
            "refresh did not add the new file"
        );
        // Sequence begins at the round boundary established by the rescan.
        // Each pair is one full round, including the transition to the next.
        for chunk in refreshed_rounds.as_chunks::<2>().0 {
            assert_eq!(chunk.iter().collect::<HashSet<_>>().len(), 2);
        }
        commands.send(Command::Stop).unwrap();
        worker.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scan_error_is_reported_once_and_refresh_recovers_after_directory_appears() {
        let dir = folder();
        std::fs::remove_dir_all(&dir).unwrap();
        let (commands, errors, started, state, worker) = worker(dir.clone(), true);
        let error = errors
            .recv_timeout(Duration::from_secs(2))
            .expect("missing music folder error was not reported");
        assert!(error.contains("cannot scan music directory"));
        assert!(error.contains(&dir.display().to_string()));
        std::fs::create_dir_all(&dir).unwrap();
        let track = dir.join("appeared.wav");
        std::fs::write(&track, b"valid fake track").unwrap();
        commands.send(Command::Refresh).unwrap();
        assert_eq!(wait_started(&started), track);
        commands.send(Command::Stop).unwrap();
        worker.join().unwrap();
        assert!(state.lock().unwrap().stopped);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn all_failed_tracks_wait_for_refresh_and_can_be_retried() {
        let dir = folder();
        let track = dir.join("repaired.wav");
        std::fs::write(&track, b"bad").unwrap();
        let (commands, errors, started, state, worker) = worker(dir.clone(), true);
        let error = errors
            .recv_timeout(Duration::from_secs(2))
            .expect("decode error was not reported");
        assert!(error.contains("repaired.wav"));
        assert!(started.recv_timeout(Duration::from_millis(120)).is_err());
        assert_eq!(state.lock().unwrap().attempts, vec![track.clone()]);

        // Replacing a failed file does not trigger a retry loop. Explicit refresh
        // clears the failed-file set and discovers the repaired content.
        std::fs::write(&track, b"repaired content").unwrap();
        commands.send(Command::Refresh).unwrap();
        assert_eq!(wait_started(&started), track);
        assert_eq!(state.lock().unwrap().attempts.len(), 2);
        commands.send(Command::Stop).unwrap();
        worker.join().unwrap();
        assert!(state.lock().unwrap().stopped);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn disconnected_command_channel_stops_playback_adapter() {
        let dir = folder();
        let state = Arc::new(Mutex::new(FakeState::default()));
        let (_started_tx, started_rx) = mpsc::channel();
        let (commands_tx, commands_rx) = mpsc::channel();
        let (errors_tx, _errors_rx) = mpsc::channel();
        let fake = FakePlayback {
            state: state.clone(),
            started: _started_tx,
        };
        let thread_dir = dir.clone();
        let worker =
            thread::spawn(move || run_worker(commands_rx, errors_tx, thread_dir, true, 0.4, fake));
        drop(commands_tx);
        worker.join().unwrap();
        assert!(state.lock().unwrap().stopped);
        drop(started_rx);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn device_error_retries_same_track_without_advancing_and_reports_once_per_outage() {
        let dir = folder();
        let first = dir.join("first.wav");
        let second = dir.join("second.wav");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        let mut initial_state = FakeState::default();
        initial_state.device_failures.insert(first.clone(), 2);
        initial_state.device_failures.insert(second.clone(), 2);
        let (commands, errors, started, state, worker) =
            worker_with_state(dir.clone(), true, Arc::new(Mutex::new(initial_state)));

        let first_error = errors
            .recv_timeout(Duration::from_secs(2))
            .expect("first device error was not reported");
        assert!(first_error.contains("output unavailable"));
        let first_success = wait_started(&started);
        assert!(
            state.lock().unwrap().attempts[..3]
                .iter()
                .all(|path| path == &first_success)
        );
        assert!(first_success == first || first_success == second);

        finish_track(&state);
        let second_error = errors
            .recv_timeout(Duration::from_secs(2))
            .expect("second device outage was not reported");
        assert!(second_error.contains("output unavailable"));
        let second_success = wait_started(&started);
        assert_ne!(second_success, first_success);
        let attempts = state.lock().unwrap().attempts.clone();
        assert!(attempts[3..6].iter().all(|path| path == &second_success));
        assert!(
            errors.try_recv().is_err(),
            "device outage was reported repeatedly"
        );

        commands.send(Command::Stop).unwrap();
        worker.join().unwrap();
        assert!(state.lock().unwrap().stopped);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refresh_drops_pending_device_retry_when_file_was_removed() {
        let dir = folder();
        let removed = dir.join("removed.wav");
        let replacement = dir.join("replacement.wav");
        std::fs::write(&removed, b"removed").unwrap();
        let mut initial_state = FakeState::default();
        initial_state
            .device_failures
            .insert(removed.clone(), usize::MAX);
        let (commands, errors, started, state, worker) =
            worker_with_state(dir.clone(), true, Arc::new(Mutex::new(initial_state)));
        let error = errors
            .recv_timeout(Duration::from_secs(2))
            .expect("device error was not reported");
        assert!(error.contains("output unavailable"));
        assert_eq!(state.lock().unwrap().attempts, vec![removed.clone()]);

        std::fs::remove_file(&removed).unwrap();
        std::fs::write(&replacement, b"replacement").unwrap();
        commands.send(Command::Refresh).unwrap();
        assert_eq!(wait_started(&started), replacement);
        assert_eq!(state.lock().unwrap().attempts, vec![removed, replacement]);
        assert!(
            errors.try_recv().is_err(),
            "device outage was reported repeatedly"
        );
        commands.send(Command::Stop).unwrap();
        worker.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
