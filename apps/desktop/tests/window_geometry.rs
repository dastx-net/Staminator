#[path = "../src/window_geometry.rs"]
mod window_geometry;
#[path = "../src/window_monitors.rs"]
mod window_monitors;

#[test]
fn stored_geometry_is_validated_before_use() {
    use window_geometry::{LogicalRect, SavedWindowGeometry, is_valid_saved_geometry};

    let mut saved = SavedWindowGeometry {
        pixels_per_point: None,
        rect: LogicalRect::new(20.0, 30.0, 900.0, 500.0),
        maximized: false,
        visible: true,
    };
    assert!(is_valid_saved_geometry(&saved));
    saved.rect.width = f32::NAN;
    assert!(!is_valid_saved_geometry(&saved));
}

#[test]
fn invalid_scale_does_not_query_monitors() {
    assert!(window_monitors::monitor_work_areas(0.0).is_empty());
}

#[test]
fn chooses_monitor_and_recovers_offscreen_saved_window() {
    use window_geometry::{
        LogicalRect, TITLE_BAR_HEIGHT, TITLE_BAR_VISIBLE_WIDTH, choose_visible_rect_on_monitors,
    };

    let primary = LogicalRect::new(0.0, 0.0, 1600.0, 900.0);
    let secondary = LogicalRect::new(-1200.0, 0.0, 1200.0, 900.0);
    let saved = LogicalRect::new(-300.0, 100.0, 600.0, 400.0);
    assert_eq!(
        choose_visible_rect_on_monitors(Some(saved), primary, &[primary, secondary]),
        saved
    );

    let offscreen = LogicalRect::new(9000.0, 3000.0, 600.0, 400.0);
    let restored = choose_visible_rect_on_monitors(Some(offscreen), primary, &[primary, secondary]);
    assert_eq!(restored.x, primary.right() - TITLE_BAR_VISIBLE_WIDTH);
    assert_eq!(restored.y, primary.bottom() - TITLE_BAR_HEIGHT);
}
