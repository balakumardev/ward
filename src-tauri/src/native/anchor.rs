//! Multi-monitor anchor math for the tray-anchored glance popover (Plan 15).
//!
//! Pulled out of `tray.rs` into a pure, unit-testable function because the
//! placement is coordinate-space-sensitive and was silently wrong on
//! multi-monitor Retina setups: the `TrayIconEvent::Click` position arrives in
//! PHYSICAL coordinates uniformly scaled by the *primary* display's factor,
//! while macOS's `monitor_from_point` hit-tests display bounds in LOGICAL
//! points. Feeding the physical point straight in mis-resolved the monitor
//! (usually to `None` → `current_monitor()` `None` on a hidden window →
//! `primary_monitor()`), so the popover always opened on the primary display
//! instead of the one the user clicked the menu-bar icon on.
//!
//! This function does the hit-test itself in logical space and returns the
//! logical top-left to place the window at (via `set_position(LogicalPosition)`),
//! clamped to the resolved monitor so the popover never spills onto a neighbor.

/// A monitor's geometry as reported by Tauri's `Monitor` — physical position +
/// size plus its own scale factor. Kept Tauri-free so the placement math is
/// pure and testable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorRect {
    pub phys_x: f64,
    pub phys_y: f64,
    pub phys_w: f64,
    pub phys_h: f64,
    pub scale: f64,
}

/// The logical top-left to place the popover window at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalAnchor {
    pub x: f64,
    pub y: f64,
}

impl MonitorRect {
    /// This monitor's bounds in LOGICAL points. Tauri reports `position()` /
    /// `size()` in physical pixels (`logical × this monitor's own scale`), so we
    /// divide by the monitor's own scale to recover the logical bounds — the
    /// space macOS lays displays out in and hit-tests clicks against.
    fn logical_rect(&self) -> (f64, f64, f64, f64) {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            self.phys_x / s,
            self.phys_y / s,
            self.phys_w / s,
            self.phys_h / s,
        )
    }
}

/// Compute where to place the popover for a tray click.
///
/// * `click_phys_x/y` — the `TrayIconEvent::Click` position: physical coords,
///   uniformly scaled by the PRIMARY display's factor (macOS quirk).
/// * `primary_scale`  — the primary monitor's scale factor.
/// * `monitors`       — all monitors (from `available_monitors()`).
/// * `win_w/h_logical`— the popover window's logical size.
/// * `gap_logical`    — logical gap dropped below the menu bar.
///
/// Returns the logical top-left, clamped to the monitor under the click.
pub fn popover_anchor(
    click_phys_x: f64,
    click_phys_y: f64,
    primary_scale: f64,
    monitors: &[MonitorRect],
    win_w_logical: f64,
    win_h_logical: f64,
    gap_logical: f64,
) -> LogicalAnchor {
    // Recover the true LOGICAL click point. The tray click is `logical ×
    // primary_scale`, a uniform scaling by the primary display's factor.
    let ps = if primary_scale > 0.0 { primary_scale } else { 1.0 };
    let cx = click_phys_x / ps;
    let cy = click_phys_y / ps;

    // The monitor whose LOGICAL bounds contain the click; else the first
    // monitor (keeps the popover on-screen even for an out-of-bounds click).
    let selected = monitors
        .iter()
        .find(|m| {
            let (lx, ly, lw, lh) = m.logical_rect();
            cx >= lx && cx < lx + lw && cy >= ly && cy < ly + lh
        })
        .or_else(|| monitors.first());

    // Center horizontally under the click; drop below the menu bar.
    let mut x = cx - win_w_logical / 2.0;
    let mut y = cy + gap_logical;

    // Clamp the whole window inside the resolved monitor so it never spills
    // onto a neighbor. If the window is larger than the monitor, pin to origin.
    if let Some(m) = selected {
        let (lx, ly, lw, lh) = m.logical_rect();
        let max_x = (lx + lw - win_w_logical).max(lx);
        let max_y = (ly + lh - win_h_logical).max(ly);
        x = x.clamp(lx, max_x);
        y = y.clamp(ly, max_y);
    }

    LogicalAnchor { x, y }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 0.01, "expected {b}, got {a}");
    }

    #[test]
    fn centers_under_click_on_single_retina_monitor() {
        // One Retina display: 1440×900 logical (2880×1800 physical @ 2×).
        let mons = [MonitorRect { phys_x: 0.0, phys_y: 0.0, phys_w: 2880.0, phys_h: 1800.0, scale: 2.0 }];
        // Click the menu bar at logical (720, 12) => physical (1440, 24) @ primary 2×.
        let a = popover_anchor(1440.0, 24.0, 2.0, &mons, 320.0, 300.0, 6.0);
        approx(a.x, 560.0); // 720 - 320/2
        approx(a.y, 18.0); // 12 + 6
    }

    #[test]
    fn places_on_secondary_monitor_the_click_landed_on() {
        // Primary Retina 1440×900 @2× at origin; secondary non-Retina 1920×1080
        // @1× to the right (macOS logical origin 1440,0 => physical 1440,0 @1×).
        let mons = [
            MonitorRect { phys_x: 0.0, phys_y: 0.0, phys_w: 2880.0, phys_h: 1800.0, scale: 2.0 },
            MonitorRect { phys_x: 1440.0, phys_y: 0.0, phys_w: 1920.0, phys_h: 1080.0, scale: 1.0 },
        ];
        // Click the secondary menu bar at logical (2400, 8) => physical (4800, 16)
        // because the tray click is uniformly scaled by the PRIMARY 2×.
        let a = popover_anchor(4800.0, 16.0, 2.0, &mons, 320.0, 300.0, 6.0);
        // Must land on the secondary display (x >= 1440), NOT clamped to primary.
        assert!(a.x >= 1440.0, "popover should open on the secondary display, got x={}", a.x);
        approx(a.x, 2240.0); // 2400 - 160
        approx(a.y, 14.0); // 8 + 6
    }

    #[test]
    fn clamps_within_monitor_right_edge() {
        let mons = [MonitorRect { phys_x: 0.0, phys_y: 0.0, phys_w: 1440.0, phys_h: 900.0, scale: 1.0 }];
        // Click near the right edge; a centered 320-wide window would overflow.
        let a = popover_anchor(1430.0, 10.0, 1.0, &mons, 320.0, 300.0, 6.0);
        approx(a.x, 1120.0); // clamped to 1440 - 320
        approx(a.y, 16.0);
    }

    #[test]
    fn falls_back_to_first_monitor_when_click_outside_all() {
        let mons = [MonitorRect { phys_x: 0.0, phys_y: 0.0, phys_w: 1440.0, phys_h: 900.0, scale: 1.0 }];
        let a = popover_anchor(5000.0, 5000.0, 1.0, &mons, 320.0, 300.0, 6.0);
        // Clamped onto the only monitor so it stays visible.
        approx(a.x, 1120.0);
        approx(a.y, 600.0); // 900 - 300
    }

    #[test]
    fn best_effort_when_no_monitors() {
        let a = popover_anchor(400.0, 20.0, 1.0, &[], 320.0, 300.0, 6.0);
        approx(a.x, 240.0); // 400 - 160, unclamped
        approx(a.y, 26.0); // 20 + 6
    }
}
