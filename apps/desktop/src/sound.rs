//! Background playback of audio files and standard system sounds.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

type SoundSnapshot = Arc<Mutex<Option<Result<Vec<PathBuf>, String>>>>;

/// Background service for user-selected WAV and MP3 files.
///
/// Commands are queued to a dedicated thread, so calls never wait for decoding
/// or audio output initialization. Errors are reported asynchronously by
/// [`SoundService::try_error`].
pub struct SoundService {
    commands: Sender<Command>,
    errors: Receiver<String>,
}

enum Command {
    Play(PathBuf),
    SetVolume(f32),
    PlayError {
        directory: PathBuf,
        snapshot: SoundSnapshot,
        system_player: fn(SoundEvent),
    },
    SystemTone(SoundEvent, fn(SoundEvent)),
    Stop,
    ResetOutput,
}

impl SoundService {
    /// Start the audio worker. Device initialization happens when a file is played.
    pub fn new() -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (error_tx, errors) = mpsc::channel();
        thread::Builder::new()
            .name("staminator-audio".into())
            .spawn(move || audio_worker(command_rx, error_tx))
            .expect("failed to start audio worker");
        Self { commands, errors }
    }

    /// Queue a WAV or MP3 file for playback. Unsupported or unreadable files
    /// are reported later through [`Self::try_error`].
    pub fn play_file(&self, path: impl AsRef<Path>) -> Result<(), String> {
        self.commands
            .send(Command::Play(path.as_ref().to_owned()))
            .map_err(|_| "audio worker has stopped".to_owned())
    }

    /// Set playback volume for subsequent and current user sounds (clamped to 0..=1).
    pub fn set_volume(&self, volume: f32) -> Result<(), String> {
        self.commands
            .send(Command::SetVolume(normalize_volume(volume)))
            .map_err(|_| "audio worker has stopped".to_owned())
    }

    fn play_error(
        &self,
        directory: PathBuf,
        snapshot: SoundSnapshot,
        system_player: fn(SoundEvent),
    ) -> Result<(), String> {
        self.commands
            .send(Command::PlayError {
                directory,
                snapshot,
                system_player,
            })
            .map_err(|_| "audio worker has stopped".to_owned())
    }

    fn system_tone(&self, event: SoundEvent, system_player: fn(SoundEvent)) -> Result<(), String> {
        self.commands
            .send(Command::SystemTone(event, system_player))
            .map_err(|_| "audio worker has stopped".to_owned())
    }

    /// Stop the current user-file playback.
    pub fn stop(&self) -> Result<(), String> {
        self.commands
            .send(Command::Stop)
            .map_err(|_| "audio worker has stopped".to_owned())
    }

    /// Stop playback and reopen the output device on the next play command.
    pub fn reset_output(&self) -> Result<(), String> {
        self.commands
            .send(Command::ResetOutput)
            .map_err(|_| "audio worker has stopped".to_owned())
    }

    /// Return one asynchronous worker or playback error, if available.
    pub fn try_error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }
}

impl Default for SoundService {
    fn default() -> Self {
        Self::new()
    }
}

fn audio_worker(commands: Receiver<Command>, errors: Sender<String>) {
    let mut stream = None;
    let mut sink: Option<rodio::Sink> = None;
    let mut volume = 1.0_f32;
    let mut cached_directory: Option<PathBuf> = None;
    let mut cached_files: Result<Vec<PathBuf>, String> = Ok(Vec::new());
    let mut fallback_allowed = false;
    let mut next_file = 0;
    while let Ok(command) = commands.recv() {
        match command {
            Command::Stop => {
                if let Some(current) = sink.take() {
                    current.stop();
                }
            }
            Command::SetVolume(next_volume) => {
                volume = normalize_volume(next_volume);
                if let Some(current) = sink.as_ref() {
                    current.set_volume(volume);
                }
            }
            Command::ResetOutput => {
                if let Some(current) = sink.take() {
                    current.stop();
                }
                stream = None;
            }
            Command::SystemTone(event, system_player) => system_player(event),
            Command::PlayError {
                directory,
                snapshot,
                system_player,
            } => {
                // Directory enumeration and sound selection stay on this worker,
                // so a slow or unavailable filesystem cannot delay keyboard input.
                let (files, can_fallback) = load_sound_files(&directory);
                if cached_directory.as_ref() != Some(&directory)
                    || cached_files != files
                    || fallback_allowed != can_fallback
                {
                    cached_directory = Some(directory);
                    cached_files = files;
                    fallback_allowed = can_fallback;
                    next_file = 0;
                }
                if let Ok(mut current) = snapshot.lock() {
                    *current = Some(cached_files.clone());
                }
                match choose_error_sound(&cached_files, fallback_allowed, &mut next_file) {
                    ErrorSoundChoice::SystemFallback => system_player(SoundEvent::Error),
                    ErrorSoundChoice::UserFile(path) => {
                        play_file_on_output(path, &mut stream, &mut sink, volume, &errors);
                    }
                    ErrorSoundChoice::Suppressed => {}
                }
            }
            Command::Play(path) => {
                play_file_on_output(path, &mut stream, &mut sink, volume, &errors);
            }
        }
    }
    drop(sink);
    drop(stream);
}

fn normalize_volume(volume: f32) -> f32 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn play_file_on_output(
    path: PathBuf,
    stream: &mut Option<rodio::OutputStream>,
    sink: &mut Option<rodio::Sink>,
    volume: f32,
    errors: &Sender<String>,
) {
    match decode_file(&path) {
        Ok(source) => {
            if stream.is_none() {
                match rodio::OutputStreamBuilder::open_default_stream() {
                    Ok(output) => *stream = Some(output),
                    Err(error) => {
                        let _ = errors.send(format!("audio output unavailable: {error}"));
                        return;
                    }
                }
            }
            if let Some(current) = sink.take() {
                current.stop();
            }
            let next = rodio::Sink::connect_new(stream.as_ref().unwrap().mixer());
            next.set_volume(volume);
            next.append(source);
            *sink = Some(next);
        }
        Err(error) => {
            let _ = errors.send(error);
        }
    }
}

fn decode_file(path: &Path) -> Result<rodio::Decoder<std::io::BufReader<std::fs::File>>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    rodio::Decoder::try_from(file)
        .map_err(|error| format!("cannot decode {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SYSTEM_TONES: AtomicUsize = AtomicUsize::new(0);
    static LATE_SYSTEM_TONES: AtomicUsize = AtomicUsize::new(0);
    static CHANGING_SYSTEM_TONES: AtomicUsize = AtomicUsize::new(0);
    static CALLBACK_THREAD: std::sync::OnceLock<Mutex<Option<thread::ThreadId>>> =
        std::sync::OnceLock::new();

    fn fake_system_tone(_: SoundEvent) {
        SYSTEM_TONES.fetch_add(1, Ordering::Relaxed);
    }

    fn fake_late_system_tone(_: SoundEvent) {
        LATE_SYSTEM_TONES.fetch_add(1, Ordering::Relaxed);
    }

    fn fake_changing_system_tone(_: SoundEvent) {
        CHANGING_SYSTEM_TONES.fetch_add(1, Ordering::Relaxed);
    }

    fn record_system_tone_thread(_: SoundEvent) {
        *CALLBACK_THREAD
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(thread::current().id());
    }

    fn wait_for_count(counter: &AtomicUsize, expected: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while counter.load(Ordering::Relaxed) < expected {
            assert!(
                std::time::Instant::now() < deadline,
                "callback was not called"
            );
            thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn wait_for_files(effects: &SoundEffects) -> Result<Vec<PathBuf>, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Ok(files) = effects.files() {
                return Ok(files);
            }
            if std::time::Instant::now() >= deadline {
                return effects.files();
            }
            thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn wait_for_file_error(effects: &SoundEffects) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Err(error) = effects.files()
                && error != "sound file list has not been scanned yet"
            {
                return error;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "directory scan did not report its error"
            );
            thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn temp_path(label: &str, extension: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "staminator-{label}-{}-{nonce}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    fn commands_and_errors_are_transferred_without_an_audio_device() {
        let (commands, rx) = mpsc::channel();
        let (tx, errors) = mpsc::channel();
        let service = SoundService { commands, errors };
        service.play_file("example.wav").unwrap();
        service.stop().unwrap();
        service.reset_output().unwrap();
        assert!(
            matches!(rx.recv().unwrap(), Command::Play(path) if path == Path::new("example.wav"))
        );
        assert!(matches!(rx.recv().unwrap(), Command::Stop));
        assert!(matches!(rx.recv().unwrap(), Command::ResetOutput));
        service.set_volume(0.4).unwrap();
        assert!(
            matches!(rx.recv().unwrap(), Command::SetVolume(v) if (v - 0.4).abs() < f32::EPSILON)
        );
        assert_eq!(service.try_error(), None);
        tx.send("device unavailable".into()).unwrap();
        assert_eq!(service.try_error().as_deref(), Some("device unavailable"));
        drop(rx);
        assert!(service.play_file("example.mp3").is_err());
        assert!(service.stop().is_err());
        assert!(service.reset_output().is_err());
        assert!(service.set_volume(0.4).is_err());
    }

    #[test]
    fn volume_is_clamped_and_non_finite_values_restore_full_volume() {
        assert_eq!(normalize_volume(-1.0), 0.0);
        assert_eq!(normalize_volume(2.0), 1.0);
        assert_eq!(normalize_volume(f32::NAN), 1.0);
        assert_eq!(normalize_volume(f32::INFINITY), 1.0);
    }

    #[test]
    fn error_sound_selection_uses_cached_files_then_falls_back_only_for_empty_list() {
        let directory = temp_path("sound-dir", "folder");
        let files = Ok(vec![directory.join("one.wav"), directory.join("two.mp3")]);
        let mut index = 0;
        assert!(
            matches!(choose_error_sound(&files, false, &mut index), ErrorSoundChoice::UserFile(path) if path.ends_with("one.wav"))
        );
        assert!(
            matches!(choose_error_sound(&files, false, &mut index), ErrorSoundChoice::UserFile(path) if path.ends_with("two.mp3"))
        );
        assert!(
            matches!(choose_error_sound(&files, false, &mut index), ErrorSoundChoice::UserFile(path) if path.ends_with("one.wav"))
        );
        assert!(matches!(
            choose_error_sound(&Ok(vec![]), true, &mut index),
            ErrorSoundChoice::SystemFallback
        ));
        assert!(matches!(
            choose_error_sound(&Ok(vec![]), false, &mut index),
            ErrorSoundChoice::Suppressed
        ));
        assert!(matches!(
            choose_error_sound(&Err("read failed".into()), true, &mut index),
            ErrorSoundChoice::Suppressed
        ));
    }

    #[test]
    fn only_a_truly_empty_directory_uses_fallback_and_system_player_is_injectable() {
        let empty = temp_path("empty-sound-dir", "folder");
        let populated = temp_path("populated-sound-dir", "folder");
        std::fs::create_dir(&empty).unwrap();
        std::fs::create_dir(&populated).unwrap();
        std::fs::write(populated.join("readme.txt"), b"unsupported").unwrap();
        SYSTEM_TONES.store(0, Ordering::Relaxed);

        let mut empty_effects = SoundEffects::with_system_player(&empty, fake_system_tone);
        empty_effects.play(SoundEvent::Error);
        wait_for_count(&SYSTEM_TONES, 1);
        assert_eq!(SYSTEM_TONES.load(Ordering::Relaxed), 1);
        empty_effects.set_enabled(false);
        empty_effects.play(SoundEvent::Start);
        assert_eq!(SYSTEM_TONES.load(Ordering::Relaxed), 1);
        empty_effects.set_enabled(true);
        empty_effects.set_volume(0.0).unwrap();
        empty_effects.play(SoundEvent::Error);
        assert_eq!(SYSTEM_TONES.load(Ordering::Relaxed), 1);

        let mut populated_effects = SoundEffects::with_system_player(&populated, fake_system_tone);
        populated_effects.play(SoundEvent::Error);
        assert_eq!(SYSTEM_TONES.load(Ordering::Relaxed), 1);
        populated_effects.play(SoundEvent::Start);
        wait_for_count(&SYSTEM_TONES, 2);
        assert_eq!(SYSTEM_TONES.load(Ordering::Relaxed), 2);

        let missing = temp_path("missing-sound-dir", "folder");
        let mut unreadable_effects = SoundEffects::with_system_player(&missing, fake_system_tone);
        unreadable_effects.play(SoundEvent::Error);
        assert!(wait_for_file_error(&unreadable_effects).contains("cannot read sound directory"));
        assert_eq!(SYSTEM_TONES.load(Ordering::Relaxed), 2);

        drop((empty_effects, populated_effects, unreadable_effects));
        std::fs::remove_dir_all(empty).unwrap();
        std::fs::remove_dir_all(populated).unwrap();
    }

    #[test]
    fn newly_added_supported_file_replaces_cached_empty_directory_fallback() {
        let directory = temp_path("late-sound-file", "folder");
        std::fs::create_dir(&directory).unwrap();
        LATE_SYSTEM_TONES.store(0, Ordering::Relaxed);
        let mut effects = SoundEffects::with_system_player(&directory, fake_late_system_tone);
        assert!(effects.files().is_err());

        let added = directory.join("effect.wav");
        std::fs::write(&added, b"invalid wave data").unwrap();
        effects.play(SoundEvent::Error);

        assert_eq!(wait_for_files(&effects).unwrap(), vec![added]);
        assert_eq!(LATE_SYSTEM_TONES.load(Ordering::Relaxed), 0);
        drop(effects);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn error_fallback_tracks_directory_changes_after_the_first_worker_scan() {
        let directory = temp_path("changing-sound-dir", "folder");
        std::fs::create_dir(&directory).unwrap();
        CHANGING_SYSTEM_TONES.store(0, Ordering::Relaxed);
        let mut effects = SoundEffects::with_system_player(&directory, fake_changing_system_tone);

        effects.play(SoundEvent::Error);
        wait_for_count(&CHANGING_SYSTEM_TONES, 1);
        effects.play(SoundEvent::Error);
        wait_for_count(&CHANGING_SYSTEM_TONES, 2);

        let unsupported = directory.join("readme.txt");
        std::fs::write(&unsupported, b"not an effect").unwrap();
        effects.play(SoundEvent::Error);
        effects.play(SoundEvent::Pause); // Worker queue barrier after the error scan.
        wait_for_count(&CHANGING_SYSTEM_TONES, 3);
        assert_eq!(CHANGING_SYSTEM_TONES.load(Ordering::Relaxed), 3);

        std::fs::remove_file(unsupported).unwrap();
        effects.play(SoundEvent::Error);
        effects.play(SoundEvent::Pause);
        wait_for_count(&CHANGING_SYSTEM_TONES, 5);
        drop(effects);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn refreshed_directory_plays_user_file_and_reports_decode_failure_asynchronously() {
        let original = temp_path("refresh-old-sound", "folder");
        let replacement = temp_path("refresh-new-sound", "folder");
        std::fs::create_dir(&original).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        let added = replacement.join("effect.wav");
        std::fs::write(&added, b"not a valid wave").unwrap();

        let mut effects = SoundEffects::with_system_player(&original, fake_late_system_tone);
        effects.refresh(&replacement);
        effects.play(SoundEvent::Error);
        assert_eq!(wait_for_files(&effects).unwrap(), vec![added.clone()]);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if let Some(error) = effects.try_error() {
                assert!(error.contains("cannot decode"), "{error}");
                assert!(error.contains(&added.display().to_string()));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "decode error not reported"
            );
            thread::sleep(std::time::Duration::from_millis(2));
        }

        drop(effects);
        std::fs::remove_dir_all(original).unwrap();
        std::fs::remove_dir_all(replacement).unwrap();
    }

    #[test]
    fn system_fallback_callback_runs_on_audio_worker() {
        let directory = temp_path("worker-fallback", "folder");
        std::fs::create_dir(&directory).unwrap();
        *CALLBACK_THREAD
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = None;
        let caller_thread = thread::current().id();
        let mut effects = SoundEffects::with_system_player(&directory, record_system_tone_thread);
        effects.play(SoundEvent::Error);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let callback_thread = *CALLBACK_THREAD
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap();
            if let Some(callback_thread) = callback_thread {
                assert_ne!(callback_thread, caller_thread);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "fallback was not called"
            );
            thread::sleep(std::time::Duration::from_millis(2));
        }
        drop(effects);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn decoder_reports_missing_and_invalid_files() {
        let missing = temp_path("missing", "wav");
        assert!(decode_file(&missing).err().unwrap().contains("cannot open"));
        let invalid = temp_path("invalid", "wav");
        std::fs::write(&invalid, b"not a wave file").unwrap();
        assert!(
            decode_file(&invalid)
                .err()
                .unwrap()
                .contains("cannot decode")
        );
        std::fs::remove_file(invalid).unwrap();
    }

    #[test]
    fn worker_reports_missing_and_invalid_files_without_opening_audio_device() {
        let missing = temp_path("worker-missing", "wav");
        let invalid = temp_path("worker-invalid", "wav");
        std::fs::write(&invalid, b"not a wave file").unwrap();

        let (commands, command_rx) = mpsc::channel();
        let (error_tx, errors) = mpsc::channel();
        let worker = thread::spawn(move || audio_worker(command_rx, error_tx));
        commands.send(Command::Play(missing.clone())).unwrap();
        commands.send(Command::SetVolume(0.25)).unwrap();
        commands.send(Command::Stop).unwrap();
        commands.send(Command::ResetOutput).unwrap();
        commands.send(Command::Play(invalid.clone())).unwrap();
        drop(commands);
        worker.join().unwrap();

        let missing_error = errors.recv().unwrap();
        assert!(missing_error.contains("cannot open"), "{missing_error}");
        assert!(missing_error.contains(&missing.display().to_string()));
        let invalid_error = errors.recv().unwrap();
        assert!(invalid_error.contains("cannot decode"), "{invalid_error}");
        assert!(invalid_error.contains(&invalid.display().to_string()));
        assert!(errors.try_recv().is_err());
        std::fs::remove_file(invalid).unwrap();
    }

    #[test]
    fn service_delivers_file_errors_asynchronously() {
        let service = SoundService::new();
        let path = temp_path("service-missing", "wav");
        service.play_file(&path).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(error) = service.try_error() {
                assert!(error.contains("cannot open"), "{error}");
                assert!(error.contains(&path.display().to_string()));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not report an error"
            );
            thread::sleep(std::time::Duration::from_millis(5));
        }
        service.stop().unwrap();
    }

    #[test]
    fn decoder_accepts_pcm_wave() {
        let path = temp_path("audio", "wav");
        let wave: [u8; 46] = [
            82, 73, 70, 70, 38, 0, 0, 0, 87, 65, 86, 69, 102, 109, 116, 32, 16, 0, 0, 0, 1, 0, 1,
            0, 68, 172, 0, 0, 136, 88, 1, 0, 2, 0, 16, 0, 100, 97, 116, 97, 2, 0, 0, 0, 0, 0,
        ];
        std::fs::write(&path, wave).unwrap();
        let result = decode_file(&path);
        assert!(result.is_ok());
        drop(result);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn decoder_accepts_mp3_frames() {
        let path = temp_path("audio", "mp3");
        // MPEG-1 Layer III, 128 kbps, 44.1 kHz, silent frames.
        let mut frames = vec![0_u8; 417 * 4];
        for frame in frames.as_chunks_mut::<417>().0 {
            frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
        }
        std::fs::write(&path, frames).unwrap();
        let mut decoded = decode_file(&path);
        assert!(decoded.is_ok(), "{:?}", decoded.as_ref().err());
        assert!(decoded.as_mut().unwrap().next().is_some());
        drop(decoded);
        std::fs::remove_file(path).unwrap();
    }
}

/// The training events that have an associated system sound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoundEvent {
    Error,
    Start,
    Pause,
}

/// Cached user sound effects from a `Sound` directory.
///
/// `Error` uses supported WAV/MP3 files in round-robin order. The system tone
/// is used only when the directory was read successfully and is physically
/// empty. Unsupported entries suppress fallback. A directory read error is
/// retained and suppresses fallback, since treating it as empty would be wrong.
pub struct SoundEffects {
    service: SoundService,
    directory: PathBuf,
    snapshot: SoundSnapshot,
    enabled: bool,
    volume: f32,
    system_player: fn(SoundEvent),
}

impl SoundEffects {
    pub fn new(directory: impl AsRef<Path>) -> Self {
        Self::with_system_player(directory, play)
    }

    /// Construct with a system tone callback, useful for hosts and tests that
    /// need to intercept fallback playback.
    pub fn with_system_player(directory: impl AsRef<Path>, system_player: fn(SoundEvent)) -> Self {
        Self {
            service: SoundService::new(),
            directory: directory.as_ref().to_owned(),
            snapshot: Arc::new(Mutex::new(None)),
            enabled: true,
            volume: 1.0,
            system_player,
        }
    }

    /// Refresh the cached file list after the directory contents change.
    pub fn refresh(&mut self, directory: impl AsRef<Path>) {
        self.directory = directory.as_ref().to_owned();
        if let Ok(mut snapshot) = self.snapshot.lock() {
            *snapshot = None;
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            let _ = self.service.stop();
        }
    }

    pub fn set_volume(&mut self, volume: f32) -> Result<(), String> {
        self.volume = normalize_volume(volume);
        self.service.set_volume(self.volume)
    }

    /// Play an event. User files currently replace the error tone; start and
    /// pause continue to use their standard system tones.
    pub fn play(&mut self, event: SoundEvent) {
        if !self.enabled || self.volume == 0.0 {
            return;
        }
        if event != SoundEvent::Error {
            let _ = self.service.system_tone(event, self.system_player);
            return;
        }
        let _ = self.service.play_error(
            self.directory.clone(),
            Arc::clone(&self.snapshot),
            self.system_player,
        );
    }

    /// Return the latest asynchronously discovered supported files, if a scan completed.
    pub fn files(&self) -> Result<Vec<PathBuf>, String> {
        let snapshot = self
            .snapshot
            .lock()
            .map_err(|_| "sound cache unavailable".to_owned())?;
        match snapshot.as_ref() {
            Some(Ok(files)) => Ok(files.clone()),
            Some(Err(error)) => Err(error.clone()),
            None => Err("sound file list has not been scanned yet".to_owned()),
        }
    }

    pub fn try_error(&self) -> Option<String> {
        self.service.try_error()
    }
}

enum ErrorSoundChoice {
    UserFile(PathBuf),
    SystemFallback,
    Suppressed,
}

fn load_sound_files(directory: &Path) -> (Result<Vec<PathBuf>, String>, bool) {
    let files = crate::audio_resources::AudioResources::files(directory).map_err(|error| {
        format!(
            "cannot read sound directory {}: {error}",
            directory.display()
        )
    });
    let fallback_allowed = match &files {
        Ok(files) if files.is_empty() => std::fs::read_dir(directory)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false),
        _ => false,
    };
    (files, fallback_allowed)
}

fn choose_error_sound(
    files: &Result<Vec<PathBuf>, String>,
    fallback_allowed: bool,
    next_file: &mut usize,
) -> ErrorSoundChoice {
    match files {
        Ok(files) if files.is_empty() && fallback_allowed => ErrorSoundChoice::SystemFallback,
        Ok(files) if files.is_empty() => ErrorSoundChoice::Suppressed,
        Ok(files) => {
            let path = files[*next_file % files.len()].clone();
            *next_file = (*next_file + 1) % files.len();
            ErrorSoundChoice::UserFile(path)
        }
        Err(_) => ErrorSoundChoice::Suppressed,
    }
}

/// Play the standard system tone associated with `event`.
///
/// This is a no-op on platforms other than Windows.
pub fn play(event: SoundEvent) {
    #[cfg(windows)]
    windows::play(event);

    #[cfg(not(windows))]
    let _ = event;
}

#[cfg(windows)]
mod windows {
    use super::SoundEvent;

    const MB_ICONHAND: u32 = 0x0000_0010;
    const MB_ICONEXCLAMATION: u32 = 0x0000_0030;
    const MB_ICONASTERISK: u32 = 0x0000_0040;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBeep(u_type: u32) -> i32;
    }

    pub(super) fn play(event: SoundEvent) {
        let tone = match event {
            SoundEvent::Error => MB_ICONHAND,
            SoundEvent::Start => MB_ICONASTERISK,
            SoundEvent::Pause => MB_ICONEXCLAMATION,
        };

        // SAFETY: MessageBeep accepts any u32 tone selector and has no pointer arguments.
        // MessageBeep reports failure only when Windows cannot play the tone;
        // sound playback is best-effort and has no useful recovery action here.
        unsafe {
            MessageBeep(tone);
        }
    }
}
