//! Parsing and bounds for per-window content scaling shortcuts.
//!
//! Call [`shortcut_action`] on events belonging to the focused content window. When it
//! returns `Some`, remove that event from any training/input queue before forwarding it.

use eframe::egui;

pub const MIN_SCALE: f32 = 0.5;
pub const MAX_SCALE: f32 = 3.0;
pub const SCALE_STEP: f32 = 0.1;
pub const DEFAULT_SCALE: f32 = 1.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScaleAction {
    Increase,
    Decrease,
    Reset,
}

/// Finds a Ctrl scaling shortcut in a raw egui 0.37 event.
///
/// Plus recognizes both logical `Plus` and `Equals` (common Shift+= mapping), along
/// with physical `Plus`/`Equals` for layouts whose logical key differs. Minus and
/// zero recognize the corresponding logical or physical keys. Wheel direction follows
/// egui's content-motion convention: positive Y zooms in.
pub fn shortcut_action(event: &egui::Event) -> Option<ScaleAction> {
    match event {
        egui::Event::Key {
            key,
            physical_key,
            pressed: true,
            repeat: _,
            modifiers,
        } if modifiers.ctrl && !modifiers.alt => {
            let matches = |keys: &[egui::Key]| {
                keys.contains(key) || physical_key.is_some_and(|physical| keys.contains(&physical))
            };
            if matches(&[egui::Key::Plus, egui::Key::Equals]) {
                Some(ScaleAction::Increase)
            } else if matches(&[egui::Key::Minus]) {
                Some(ScaleAction::Decrease)
            } else if matches(&[egui::Key::Num0]) {
                Some(ScaleAction::Reset)
            } else {
                None
            }
        }
        egui::Event::MouseWheel {
            delta, modifiers, ..
        } if modifiers.ctrl && !modifiers.alt => {
            if delta.y > 0.0 {
                Some(ScaleAction::Increase)
            } else if delta.y < 0.0 {
                Some(ScaleAction::Decrease)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Applies one discrete shortcut step and clamps to the supported range.
pub fn apply_action(scale: f32, action: ScaleAction) -> f32 {
    if action == ScaleAction::Reset {
        return DEFAULT_SCALE;
    }
    let scale = if scale.is_finite() {
        scale
    } else {
        DEFAULT_SCALE
    };
    let next = match action {
        ScaleAction::Increase => scale + SCALE_STEP,
        ScaleAction::Decrease => scale - SCALE_STEP,
        ScaleAction::Reset => unreachable!(),
    };
    ((next * 10.0).round() / 10.0).clamp(MIN_SCALE, MAX_SCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(
        key: egui::Key,
        physical_key: Option<egui::Key>,
        pressed: bool,
        repeat: bool,
    ) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key,
            pressed,
            repeat,
            modifiers: egui::Modifiers::CTRL,
        }
    }

    #[test]
    fn recognizes_logical_and_physical_shortcuts() {
        assert_eq!(
            shortcut_action(&key(egui::Key::Plus, None, true, false)),
            Some(ScaleAction::Increase)
        );
        assert_eq!(
            shortcut_action(&key(egui::Key::Equals, None, true, false)),
            Some(ScaleAction::Increase)
        );
        assert_eq!(
            shortcut_action(&key(egui::Key::A, Some(egui::Key::Minus), true, false)),
            Some(ScaleAction::Decrease)
        );
        assert_eq!(
            shortcut_action(&key(egui::Key::Num0, None, true, false)),
            Some(ScaleAction::Reset)
        );
    }

    #[test]
    fn ignores_release_and_unmodified_keys() {
        assert_eq!(
            shortcut_action(&key(egui::Key::Plus, None, false, false)),
            None
        );
        assert_eq!(
            shortcut_action(&key(egui::Key::Plus, None, true, true)),
            Some(ScaleAction::Increase)
        );
        let mut event = key(egui::Key::Plus, None, true, false);
        if let egui::Event::Key { modifiers, .. } = &mut event {
            *modifiers = egui::Modifiers::NONE;
        }
        assert_eq!(shortcut_action(&event), None);
    }

    #[test]
    fn wheel_uses_ctrl_and_content_direction() {
        let event = |y, ctrl| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: if ctrl {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            },
        };
        assert_eq!(
            shortcut_action(&event(-1.0, true)),
            Some(ScaleAction::Decrease)
        );
        assert_eq!(
            shortcut_action(&event(1.0, true)),
            Some(ScaleAction::Increase)
        );
        assert_eq!(shortcut_action(&event(1.0, false)), None);
        assert_eq!(shortcut_action(&event(0.0, true)), None);
    }

    #[test]
    fn applies_steps_reset_and_bounds() {
        assert_eq!(apply_action(1.0, ScaleAction::Increase), 1.1);
        assert_eq!(apply_action(1.0, ScaleAction::Decrease), 0.9);
        assert_eq!(apply_action(2.8, ScaleAction::Increase), 2.9);
        assert_eq!(apply_action(2.99, ScaleAction::Increase), 3.0);
        assert_eq!(apply_action(0.51, ScaleAction::Decrease), 0.5);
        assert_eq!(apply_action(f32::NAN, ScaleAction::Increase), 1.1);
        assert_eq!(apply_action(2.0, ScaleAction::Reset), 1.0);
    }
}
