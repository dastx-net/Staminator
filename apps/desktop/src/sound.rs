//! Background playback of audio files and standard system sounds.

use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

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
    use rodio::{OutputStreamBuilder, Sink};

    let mut stream = None;
    let mut sink: Option<Sink> = None;
    while let Ok(command) = commands.recv() {
        match command {
            Command::Stop => {
                if let Some(current) = sink.take() {
                    current.stop();
                }
            }
            Command::ResetOutput => {
                if let Some(current) = sink.take() {
                    current.stop();
                }
                stream = None;
            }
            Command::Play(path) => {
                let result = decode_file(&path);
                match result {
                    Ok(source) => {
                        if stream.is_none() {
                            match OutputStreamBuilder::open_default_stream() {
                                Ok(output) => stream = Some(output),
                                Err(error) => {
                                    let _ =
                                        errors.send(format!("audio output unavailable: {error}"));
                                    continue;
                                }
                            }
                        }
                        if let Some(current) = sink.take() {
                            current.stop();
                        }
                        let next = Sink::connect_new(stream.as_ref().unwrap().mixer());
                        next.append(source);
                        sink = Some(next);
                    }
                    Err(error) => {
                        let _ = errors.send(error);
                    }
                }
            }
        }
    }
    drop(sink);
    drop(stream);
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
        assert_eq!(service.try_error(), None);
        tx.send("device unavailable".into()).unwrap();
        assert_eq!(service.try_error().as_deref(), Some("device unavailable"));
        drop(rx);
        assert!(service.play_file("example.mp3").is_err());
        assert!(service.stop().is_err());
        assert!(service.reset_output().is_err());
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
