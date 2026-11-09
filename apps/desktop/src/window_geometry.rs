//! Persisted window geometry in logical (DPI-independent) coordinates.

use serde::{Deserialize, Serialize};

pub const MIN_WINDOW_WIDTH: f32 = 160.0;
pub const MIN_WINDOW_HEIGHT: f32 = 100.0;
pub const MAX_WINDOW_DIMENSION: f32 = 10_000.0;
pub const TITLE_BAR_HEIGHT: f32 = 32.0;
pub const TITLE_BAR_VISIBLE_WIDTH: f32 = 96.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogicalRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LogicalRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }
}

/// Geometry saved for one stable window ID in one profile.
/// Minimized state is deliberately absent; windows reopen in their normal state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedWindowGeometry {
    pub rect: LogicalRect,
    #[serde(default)]
    pub pixels_per_point: Option<f32>,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default = "default_visible")]
    pub visible: bool,
}

const fn default_visible() -> bool {
    true
}

/// Returns whether a persisted rectangle is finite and within supported bounds.
pub fn is_valid_rect(rect: LogicalRect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= MIN_WINDOW_WIDTH
        && rect.height >= MIN_WINDOW_HEIGHT
        && rect.width <= MAX_WINDOW_DIMENSION
        && rect.height <= MAX_WINDOW_DIMENSION
        && rect.right().is_finite()
        && rect.bottom().is_finite()
}

pub fn is_valid_saved_geometry(geometry: &SavedWindowGeometry) -> bool {
    is_valid_rect(geometry.rect)
        && geometry
            .pixels_per_point
            .is_none_or(|scale| scale.is_finite() && scale > 0.0 && scale <= 16.0)
}

/// Selects a safe logical rectangle, clamping size and position so the title bar
/// remains reachable in the monitor's logical work area. Invalid saved values use
/// the supplied default rectangle.
pub fn choose_visible_rect(
    saved: Option<LogicalRect>,
    default: LogicalRect,
    work_area: LogicalRect,
) -> LogicalRect {
    if !is_valid_rect(work_area) {
        return saved
            .filter(|rect| is_valid_rect(*rect))
            .or_else(|| is_valid_rect(default).then_some(default))
            .unwrap_or(LogicalRect::new(0.0, 0.0, 800.0, 600.0));
    }
    let mut rect = saved.filter(|r| is_valid_rect(*r)).unwrap_or(default);
    if !is_valid_rect(rect) {
        rect = LogicalRect::new(work_area.x, work_area.y, 800.0, 600.0);
    }

    rect.width = rect
        .width
        .min(work_area.width)
        .max(MIN_WINDOW_WIDTH.min(work_area.width));
    rect.height = rect
        .height
        .min(work_area.height)
        .max(MIN_WINDOW_HEIGHT.min(work_area.height));

    let title_width = TITLE_BAR_VISIBLE_WIDTH.min(rect.width).min(work_area.width);
    let title_height = TITLE_BAR_HEIGHT.min(rect.height).min(work_area.height);
    rect.x = rect.x.clamp(
        work_area.x - rect.width + title_width,
        work_area.right() - title_width,
    );
    rect.y = rect.y.clamp(work_area.y, work_area.bottom() - title_height);
    rect
}

/// Chooses a work area for a saved window, then makes the rectangle reachable.
/// The areas and saved rectangle must use the same logical coordinate space.
/// If the saved rectangle overlaps a monitor, its largest-overlap monitor wins;
/// otherwise the nearest monitor to the saved title-bar anchor wins.
///
/// An empty/invalid area list falls back to `choose_visible_rect`'s existing
/// behavior using `default` as the work area.
pub fn choose_visible_rect_on_monitors(
    saved: Option<LogicalRect>,
    default: LogicalRect,
    work_areas: &[LogicalRect],
) -> LogicalRect {
    let areas: Vec<_> = work_areas
        .iter()
        .copied()
        .filter(|a| is_valid_rect(*a))
        .collect();
    if areas.is_empty() {
        return choose_visible_rect(saved, default, LogicalRect::new(0.0, 0.0, 0.0, 0.0));
    }

    let reference = saved.filter(|r| is_valid_rect(*r)).unwrap_or(default);
    let anchor_x = reference.x + TITLE_BAR_VISIBLE_WIDTH.min(reference.width) * 0.5;
    let anchor_y = reference.y + TITLE_BAR_HEIGHT.min(reference.height) * 0.5;
    let chosen = areas
        .iter()
        .max_by(|a, b| {
            let overlap_a = intersection_area(reference, **a);
            let overlap_b = intersection_area(reference, **b);
            overlap_a.total_cmp(&overlap_b).then_with(|| {
                let da = distance_to_rect(anchor_x, anchor_y, **a);
                let db = distance_to_rect(anchor_x, anchor_y, **b);
                db.total_cmp(&da)
            })
        })
        .copied()
        .unwrap();
    choose_visible_rect(saved, default, chosen)
}

fn intersection_area(a: LogicalRect, b: LogicalRect) -> f32 {
    (a.right().min(b.right()) - a.x.max(b.x)).max(0.0)
        * (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0)
}

fn distance_to_rect(x: f32, y: f32, rect: LogicalRect) -> f32 {
    let dx = if x < rect.x {
        rect.x - x
    } else if x > rect.right() {
        x - rect.right()
    } else {
        0.0
    };
    let dy = if y < rect.y {
        rect.y - y
    } else if y > rect.bottom() {
        y - rect.bottom()
    } else {
        0.0
    };
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_finite_and_implausible_rectangles() {
        assert!(!is_valid_rect(LogicalRect::new(
            f32::NAN,
            0.0,
            400.0,
            300.0
        )));
        assert!(!is_valid_rect(LogicalRect::new(0.0, 0.0, 10.0, 300.0)));
        assert!(!is_valid_rect(LogicalRect::new(0.0, 0.0, 20_000.0, 300.0)));
        assert!(is_valid_rect(LogicalRect::new(-500.0, 20.0, 400.0, 300.0)));
    }

    #[test]
    fn relocates_offscreen_window_and_keeps_title_bar_reachable() {
        let area = LogicalRect::new(100.0, 50.0, 1200.0, 800.0);
        let rect = choose_visible_rect(
            Some(LogicalRect::new(5000.0, -900.0, 500.0, 400.0)),
            LogicalRect::new(200.0, 100.0, 700.0, 500.0),
            area,
        );
        assert_eq!(rect.x, area.right() - TITLE_BAR_VISIBLE_WIDTH);
        assert_eq!(rect.y, area.y);
        assert!(rect.x < area.right());
        assert!(rect.y + TITLE_BAR_HEIGHT <= area.bottom());
    }

    #[test]
    fn invalid_saved_rect_uses_default_and_size_fits_work_area() {
        let area = LogicalRect::new(0.0, 0.0, 500.0, 300.0);
        let rect = choose_visible_rect(
            Some(LogicalRect::new(0.0, 0.0, f32::INFINITY, 400.0)),
            LogicalRect::new(-20.0, 0.0, 900.0, 700.0),
            area,
        );
        assert_eq!(rect.width, 500.0);
        assert_eq!(rect.height, 300.0);
        assert_eq!(rect.x, -20.0);
        assert_eq!(rect.y, 0.0);
    }

    #[test]
    fn geometry_serializes_without_minimized_state() {
        let geometry = SavedWindowGeometry {
            rect: LogicalRect::new(12.0, 34.0, 800.0, 600.0),
            pixels_per_point: None,
            maximized: true,
            visible: false,
        };
        let json = serde_json::to_string(&geometry).unwrap();
        assert!(json.contains("maximized"));
        assert!(!json.contains("minimized"));
        assert_eq!(
            serde_json::from_str::<SavedWindowGeometry>(&json).unwrap(),
            geometry
        );
    }

    #[test]
    fn unavailable_work_area_preserves_valid_saved_geometry() {
        let saved = LogicalRect::new(-500.0, 30.0, 900.0, 500.0);
        assert_eq!(
            choose_visible_rect(
                Some(saved),
                LogicalRect::new(0.0, 0.0, 800.0, 600.0),
                LogicalRect::new(0.0, 0.0, 0.0, 0.0)
            ),
            saved
        );
    }

    #[test]
    fn selects_monitor_with_saved_window_overlap() {
        let left = LogicalRect::new(-1000.0, 0.0, 1000.0, 800.0);
        let right = LogicalRect::new(0.0, 0.0, 1200.0, 900.0);
        let saved = LogicalRect::new(-300.0, 80.0, 500.0, 400.0);
        assert_eq!(
            choose_visible_rect_on_monitors(Some(saved), right, &[left, right]),
            saved
        );
    }

    #[test]
    fn returns_offscreen_window_to_nearest_monitor() {
        let left = LogicalRect::new(-1200.0, 0.0, 1200.0, 900.0);
        let right = LogicalRect::new(0.0, 0.0, 1400.0, 900.0);
        let rect = choose_visible_rect_on_monitors(
            Some(LogicalRect::new(8000.0, 4000.0, 600.0, 400.0)),
            left,
            &[left, right],
        );
        assert_eq!(rect.x, right.right() - TITLE_BAR_VISIBLE_WIDTH);
        assert_eq!(rect.y, right.bottom() - TITLE_BAR_HEIGHT);
    }

    #[test]
    fn invalid_monitor_list_preserves_valid_geometry() {
        let default = LogicalRect::new(0.0, 0.0, 800.0, 600.0);
        let saved = LogicalRect::new(5000.0, 0.0, 400.0, 300.0);
        assert_eq!(
            choose_visible_rect_on_monitors(Some(saved), default, &[]),
            saved
        );
    }
}
