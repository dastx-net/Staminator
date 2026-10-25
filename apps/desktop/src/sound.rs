//! Small wrapper for the standard Windows system sounds.

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

        // MessageBeep reports failure only when Windows cannot play the tone;
        // sound playback is best-effort and has no useful recovery action here.
        unsafe {
            MessageBeep(tone);
        }
    }
}
