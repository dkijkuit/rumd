mod app;
mod document;
mod icons;
mod search;
mod source;
mod toc;
mod viewer;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);
    let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var_os("WAYLAND_SOCKET").is_some();
    if let Some(scale) = x11_scale_pin(
        on_wayland,
        std::env::var("WINIT_X11_SCALE_FACTOR").ok().as_deref(),
    ) {
        // SAFETY: single-threaded startup, before eframe spawns anything.
        unsafe { std::env::set_var("WINIT_X11_SCALE_FACTOR", scale) };
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([900.0, 700.0]),
        event_loop_builder: backend_hook(),
        ..Default::default()
    };
    eframe::run_native(
        "rumd",
        options,
        Box::new(move |cc| {
            let mut app = app::App::new(path);
            app.set_prefs_file(app::prefs_path());
            app.load_prefs();
            app.apply_zoom_to_ctx(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

/// Force the X11 windowing backend on Linux.
///
/// winit 0.30 (pinned by eframe) has no drag-and-drop implementation in its
/// Wayland backend, so file drops from native Wayland apps never arrive.
/// The X11 backend implements XDND, and XWayland bridges drops from Wayland
/// sources, so running as an X11 client makes drag-and-drop work everywhere.
#[cfg(target_os = "linux")]
fn backend_hook() -> Option<eframe::EventLoopBuilderHook> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    Some(Box::new(|builder| {
        builder.with_x11();
    }))
}

#[cfg(not(target_os = "linux"))]
fn backend_hook() -> Option<eframe::EventLoopBuilderHook> {
    None
}

/// The scale factor to pin for the forced X11 backend, if any.
///
/// Under XWayland, winit's DPI resolution order is Xft.dpi (absent on most
/// Wayland sessions), then a RandR physical-size guess. That guess ignores
/// the compositor's own scale, and with Hyprland's default
/// `xwayland:force_zero_scaling = true` the app renders 1:1 physical pixels,
/// so the guess makes the UI 1.5-2x too big. Pin scale 1.0 and let the user
/// zoom (Ctrl+= / Ctrl+- / Ctrl+0, persisted in prefs). An explicit
/// `WINIT_X11_SCALE_FACTOR` from the user wins. Real X11 sessions keep
/// winit's own logic.
fn x11_scale_pin(on_wayland: bool, user_override: Option<&str>) -> Option<String> {
    if on_wayland && user_override.is_none() {
        Some("1".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// winit 0.30 has no Wayland drag-and-drop implementation, so on Linux
    /// the X11 backend must be forced or dropped files never arrive.
    #[test]
    fn forces_x11_backend_on_linux() {
        let hook = backend_hook();
        if cfg!(target_os = "linux") {
            let hook = hook.expect("X11 backend must be forced on Linux");
            let mut builder = winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event();
            hook(&mut builder);
        } else {
            assert!(hook.is_none());
        }
    }

    /// Under XWayland, winit's RandR/EDID DPI guess ignores the compositor
    /// scale (Hyprland's force_zero_scaling renders X11 clients 1:1), making
    /// the UI 1.5-2x too big. Pin scale 1.0 and let the user zoom instead.
    #[test]
    fn pins_scale_under_xwayland_only() {
        assert_eq!(x11_scale_pin(true, None).as_deref(), Some("1"));
        assert_eq!(
            x11_scale_pin(false, None),
            None,
            "real X11 keeps winit's own DPI logic"
        );
        assert_eq!(
            x11_scale_pin(true, Some("2")),
            None,
            "an explicit WINIT_X11_SCALE_FACTOR wins"
        );
    }
}
