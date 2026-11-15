//! Lumen - a fast, portable photo viewer built with Rust and egui.
//!
//! The startup path is deliberately short: read a small JSON file, install the
//! fonts, create the window and paint the first frame. Image decoding,
//! thumbnail generation and file metadata all happen on background threads, so
//! the window is on screen long before the first photo is ready.
//!
//! Module map (each file is self-contained and safe to edit on its own):
//!   app.rs          - application state, layout and interaction
//!   viewer.rs       - zoom / pan / rotation math and the image canvas painter
//!   image_loader.rs - background decoding, EXIF orientation, texture upload
//!   psd.rs          - the flattened-picture reader for Photoshop documents
//!   library.rs      - folder scanning, natural sort, navigation
//!   settings.rs     - persistent settings plus import/export
//!   window_state.rs - remembered window size, position and maximised state
//!   i18n.rs         - English / Chinese strings and system language detection
//!   theme.rs        - light and dark palettes, fonts, checkerboard texture
//!   icons.rs        - vector icons and the window icon, all drawn in code
//!   platform.rs     - Windows file associations and Explorer integration
//!   ui.rs           - small reusable widgets
//!   diag.rs         - crash / diagnostic log

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod diag;
mod i18n;
mod icons;
mod image_loader;
mod library;
mod platform;
mod psd;
mod settings;
mod theme;
mod ui;
mod viewer;
mod window_state;

// Offscreen screenshot helpers. Only built for `cargo test`, never shipped.
#[cfg(test)]
mod snapshot_tests;

use std::time::Instant;

/// Which rendering backend the window uses. Shown in the settings' About
/// section; keep it in sync with the `renderer` field below.
const RENDERER_NAME: &str = "OpenGL (glow)";

/// Where the source lives. Linked from the About section of the settings.
pub const REPOSITORY_URL: &str = "https://github.com/ChathamHung/Lumen";

fn main() -> eframe::Result {
    // Start the clock before anything else so the reported startup time is honest.
    let started = Instant::now();
    diag::install_panic_hook();
    diag::install_logger();

    // Headless helpers, useful for deployment scripts and automated tests:
    //   lumen.exe --register     write the file associations for this user
    //   lumen.exe --unregister   remove them again
    // Both only touch HKEY_CURRENT_USER and exit immediately.
    if let Some(flag) = std::env::args_os().nth(1).and_then(|arg| arg.to_str().map(str::to_owned)) {
        if flag == "--register" || flag == "--unregister" {
            let result = if flag == "--register" {
                platform::register()
            } else {
                platform::unregister()
            };
            match result {
                Ok(()) => {
                    diag::log(&format!("{flag}: ok"));
                    std::process::exit(0);
                }
                Err(error) => {
                    diag::log(&format!("{flag}: {error}"));
                    std::process::exit(1);
                }
            }
        }
    }

    let settings = settings::Settings::load();
    let lang = i18n::Lang::resolve(settings.language);

    // The shell passes a path when Lumen is launched through a file association.
    let initial_path = std::env::args_os().nth(1).map(std::path::PathBuf::from);

    // OpenGL first (it is by far the fastest to start), then DirectX 12 if the
    // GL context could not be created. LUMEN_RENDERER=wgpu / =glow overrides
    // the order, which is useful on machines with a broken driver.
    let forced = std::env::var("LUMEN_RENDERER").unwrap_or_default().to_lowercase();
    let first = if forced == "wgpu" {
        eframe::Renderer::Wgpu
    } else {
        eframe::Renderer::Glow
    };

    let result = run_with(first, settings.clone(), lang, initial_path.clone(), started);

    let result = match result {
        Ok(()) => Ok(()),
        Err(error) => {
            diag::log(&format!(
                "renderer {first:?} failed ({error}); retrying with the other backend"
            ));
            let fallback = if first == eframe::Renderer::Glow {
                eframe::Renderer::Wgpu
            } else {
                eframe::Renderer::Glow
            };
            run_with(fallback, settings, lang, initial_path, started)
        }
    };

    if let Err(error) = &result {
        diag::log(&format!("fatal: could not create the window: {error}"));
    }
    result
}

/// Creates the window with the given backend and runs the app until it closes.
fn run_with(
    renderer: eframe::Renderer,
    settings: settings::Settings,
    lang: i18n::Lang,
    initial_path: Option<std::path::PathBuf>,
    started: Instant,
) -> eframe::Result {
    // Restore the geometry of the previous run. On the very first launch there
    // is no file yet and the defaults apply: maximised, which is how photos
    // want to be looked at.
    let window = window_state::WindowState::load();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Lumen")
        .with_app_id("lumen")
        .with_inner_size([window.width, window.height])
        .with_min_inner_size([window_state::MIN_WIDTH, window_state::MIN_HEIGHT])
        .with_drag_and_drop(true)
        .with_icon(icons::app_icon());

    if let Some(position) = window.position() {
        viewport = viewport.with_position(position);
    }
    if window.maximized {
        viewport = viewport.with_maximized(true);
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer,
        // A restored position is authoritative; only centre when there is none.
        centered: window.position().is_none(),
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        "Lumen",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::LumenApp::new(
                cc,
                settings,
                lang,
                initial_path,
                started,
            )))
        }),
    )
}
