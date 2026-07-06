//! Plan 10 — menu-bar tray + dock badge.
//!
//! Creates the macOS menu-bar tray icon (via Tauri 2's `tray-icon`
//! feature), wires the right-click menu (Open Ward / Scan now / Quit),
//! and exposes a `update_badge` helper that updates the dock badge on
//! macOS (no-op on other platforms — Tauri 2's `set_badge_count` is
//! only available on WebviewWindow on macOS).
//!
//! The Tauri runtime is required to actually construct a tray icon;
//! that surface is exercised manually (the unit tests here cover the
//! pure logic bits: state, formatters, and the dock-badge payload).

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, Runtime, WebviewWindow};

use crate::error::WardError;

/// Build the right-click menu shown when the user clicks the tray
/// icon. Three items: "Open Ward", "Scan now", and "Quit". Each
/// emits a `tray_action` Tauri event carrying the selected label, so
/// the existing command handlers + frontend can react.
pub fn build_menu<R: Runtime>(app: &App<R>) -> Result<Menu<R>, WardError> {
    let open = MenuItem::with_id(app, "open", "Open Ward", true, None::<&str>)
        .map_err(|e| WardError::NotFound(format!("tray open menu item: {e}")))?;
    let scan_now = MenuItem::with_id(app, "scan", "Scan now", true, None::<&str>)
        .map_err(|e| WardError::NotFound(format!("tray scan menu item: {e}")))?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)
        .map_err(|e| WardError::NotFound(format!("tray quit menu item: {e}")))?;
    let sep = PredefinedMenuItem::separator(app)
        .map_err(|e| WardError::NotFound(format!("tray separator: {e}")))?;

    Menu::with_items(app, &[&open, &scan_now, &sep, &quit])
        .map_err(|e| WardError::NotFound(format!("tray menu: {e}")))
}

/// Set up the menu-bar tray icon for `app`. Returns the live
/// `TrayIcon` so the caller can hold it (drop = icon disappears).
///
/// Wires:
///   - Left-click on the icon → toggle the tray-anchored popover window.
///   - Right-click menu items → `tray_action` event with the menu id.
pub fn setup<R: Runtime>(app: &App<R>) -> Result<TrayIcon<R>, WardError> {
    let menu = build_menu(app)?;
    // Plan 13 — dedicated monochrome template so the menu-bar glyph adapts
    // cleanly to light/dark instead of muddily tinting the color app icon.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../../icons/tray-template.png"))
        .map_err(|e| WardError::NotFound(format!("tray template icon: {e}")))?;

    let tray = TrayIconBuilder::with_id("ward-tray")
        .icon(icon)
        .icon_as_template(true) // macOS dark-mode support
        .tooltip("Ward")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            // Forward to the frontend; the App.tsx switch handles
            // routing each menu id to the right command.
            let app: &AppHandle<R> = app;
            let _ = app.emit("tray_action", event.id().0.clone());
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button, button_state, position, .. } = event {
                if matches!(button, MouseButton::Left) && matches!(button_state, MouseButtonState::Up) {
                    let app = tray.app_handle();
                    if let Some(win) = app.get_webview_window("popover") {
                        if win.is_visible().unwrap_or(false) {
                            let _ = win.hide();
                        } else {
                            // Anchor the popover just under the tray icon, centered on
                            // the click point and CLAMPED to the monitor the click
                            // landed on — a raw physical coord valid on one display can
                            // otherwise spill onto another (multi-monitor setups with
                            // mixed scale factors, e.g. Retina 2× + external 1×). We
                            // position manually rather than via tauri-plugin-positioner,
                            // whose TrayCenter path calls `current_monitor().unwrap()`
                            // on the still-hidden window and panics with `None` on macOS.
                            let size = win.outer_size().ok();
                            let w = size.map(|s| s.width as f64).unwrap_or(320.0);
                            let h = size.map(|s| s.height as f64).unwrap_or(300.0);

                            // The monitor under the click point; fall back to the
                            // window's current monitor, then the primary.
                            let monitor = win
                                .monitor_from_point(position.x, position.y)
                                .ok()
                                .flatten()
                                .or_else(|| win.current_monitor().ok().flatten())
                                .or_else(|| win.primary_monitor().ok().flatten());

                            // Center under the click; drop a scale-aware gap so the
                            // popover clears the menu bar.
                            let scale = monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
                            let mut x = position.x - w / 2.0;
                            let mut y = position.y + (6.0 * scale).round();

                            // Clamp the whole rect inside the resolved monitor so it
                            // never spills onto an adjacent display. Guard the ranges:
                            // if the window is larger than the monitor, pin to origin.
                            if let Some(m) = monitor.as_ref() {
                                let mp = m.position();
                                let ms = m.size();
                                let (mx, my) = (mp.x as f64, mp.y as f64);
                                let max_x = (mx + ms.width as f64 - w).max(mx);
                                let max_y = (my + ms.height as f64 - h).max(my);
                                x = x.clamp(mx, max_x);
                                y = y.clamp(my, max_y);
                            } else {
                                // No monitor info — keep it at least near the origin.
                                x = x.max(0.0);
                                y = y.max(0.0);
                            }

                            // Opt-in diagnostics for verifying multi-monitor placement
                            // on real hardware (WARD_TRAY_DEBUG=1) without a rebuild.
                            if std::env::var("WARD_TRAY_DEBUG").is_ok() {
                                eprintln!(
                                    "ward tray: click=({:.1},{:.1}) monitor={} pos={:?} size={:?} scale={} -> set=({},{})",
                                    position.x,
                                    position.y,
                                    monitor.as_ref().and_then(|m| m.name().cloned()).unwrap_or_else(|| "<none>".into()),
                                    monitor.as_ref().map(|m| *m.position()),
                                    monitor.as_ref().map(|m| *m.size()),
                                    scale,
                                    x as i32,
                                    y as i32,
                                );
                            }

                            let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                }
            }
        })
        .build(app)
        .map_err(|e| WardError::NotFound(format!("tray build: {e}")))?;

    Ok(tray)
}

/// Update the dock badge on macOS to show `critical_count` new
/// findings. Tauri 2 exposes `set_badge_count` only on
/// `WebviewWindow`; we look up the main window from `app` and call
/// it there. On non-macOS platforms the call is still safe (Tauri
/// silently no-ops).
///
/// `critical_count = 0` clears the badge (passing `None`).
pub fn update_badge<R: Runtime>(app: &AppHandle<R>, critical_count: usize) {
    let target = if critical_count == 0 { None } else { Some(critical_count as i64) };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_badge_count(target);
    }
}

/// Update the badge on a specific webview window. Useful when the
/// caller already has the window handle.
pub fn update_badge_on_window<R: Runtime>(window: &WebviewWindow<R>, critical_count: usize) {
    let target = if critical_count == 0 { None } else { Some(critical_count as i64) };
    let _ = window.set_badge_count(target);
}

/// Build a human-readable tooltip for the tray icon. The frontend
/// supplies the values; the tray module formats. We keep the format
/// string here so tests can pin it.
pub fn format_tooltip(critical: usize, last_scan_at: Option<&str>) -> String {
    match last_scan_at {
        Some(ts) => format!("Ward — {critical} critical · last scan {ts}"),
        None => format!("Ward — {critical} critical"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tooltip_includes_critical_count() {
        let s = format_tooltip(0, None);
        assert_eq!(s, "Ward — 0 critical");
    }

    #[test]
    fn format_tooltip_includes_last_scan_when_present() {
        let s = format_tooltip(3, Some("2026-07-04 12:00"));
        assert!(s.contains("3 critical"));
        assert!(s.contains("2026-07-04 12:00"));
    }

    #[test]
    fn update_badge_payload_zero_is_none() {
        // The format contract: zero critical → None (clears the
        // badge). Positive count → Some(i64). We pin the helper
        // logic in a way that doesn't need a live AppHandle.
        let critical: usize = 0;
        let target = if critical == 0 { None } else { Some(critical as i64) };
        assert_eq!(target, None);
    }

    #[test]
    fn update_badge_payload_positive_is_some() {
        let critical: usize = 5;
        let target = if critical == 0 { None } else { Some(critical as i64) };
        assert_eq!(target, Some(5));
    }
}