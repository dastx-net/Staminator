//! Monitor work areas in egui's logical (points) coordinates.
//!
//! Windows reports monitor rectangles in physical screen pixels. Pass the
//! current window's `pixels_per_point` to match egui-winit's physical-to-point
//! conversion. With mixed-DPI monitors this gives a coordinate space scaled by
//! the current window, not monitor-specific logical coordinates; callers should
//! use one window's scale consistently for all areas in a selection operation.

use crate::window_geometry::LogicalRect;

/// Lists monitor work areas in egui points using the current window's scale.
///
/// This mirrors egui-winit's conversion for the current window. It does not
/// query each monitor's DPI or normalize mixed-DPI virtual desktop origins.
///
/// Returns an empty list when the scale is invalid or on non-Windows systems.
/// Negative origins are preserved for monitors placed left of or above the
/// primary monitor.
#[cfg(windows)]
pub fn monitor_work_areas(pixels_per_point: f32) -> Vec<LogicalRect> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORENUMPROC, MONITORINFO,
    };

    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return Vec::new();
    }

    struct Context {
        scale: f32,
        areas: Vec<LogicalRect>,
    }

    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _dc: HDC,
        _monitor_rect: *mut RECT,
        data: isize,
    ) -> i32 {
        // SAFETY: EnumDisplayMonitors invokes this callback synchronously with
        // the address of the live Context supplied below.
        let context = unsafe { &mut *(data as *mut Context) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rcWork: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dwFlags: 0,
        };
        // SAFETY: `monitor` is supplied by EnumDisplayMonitors and `info` is a
        // valid writable MONITORINFO with cbSize initialized.
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return 1;
        }
        let r = info.rcWork;
        if r.right <= r.left || r.bottom <= r.top {
            return 1;
        }
        context.areas.push(physical_rect_to_logical(
            r.left,
            r.top,
            r.right,
            r.bottom,
            context.scale,
        ));
        1
    }

    let mut context = Context {
        scale: pixels_per_point,
        areas: Vec::new(),
    };
    let callback: MONITORENUMPROC = Some(collect);
    // SAFETY: callback only uses the valid Context pointer during this
    // synchronous enumeration. Null DC and clipping rectangle request all
    // monitors.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            callback,
            &mut context as *mut Context as isize,
        );
    }
    context.areas
}

/// Non-Windows platforms do not expose native monitor geometry through this
/// adapter yet.
#[cfg(not(windows))]
pub fn monitor_work_areas(_pixels_per_point: f32) -> Vec<LogicalRect> {
    Vec::new()
}

#[cfg(any(windows, test))]
fn physical_rect_to_logical(
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    scale: f32,
) -> LogicalRect {
    let scale = scale.recip();
    LogicalRect::new(
        left as f32 * scale,
        top as f32 * scale,
        (right - left) as f32 * scale,
        (bottom - top) as f32 * scale,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_physical_rect_including_negative_origin() {
        let rect = physical_rect_to_logical(-1920, 0, 0, 1080, 2.0);
        assert_eq!(rect, LogicalRect::new(-960.0, 0.0, 960.0, 540.0));
    }
}
