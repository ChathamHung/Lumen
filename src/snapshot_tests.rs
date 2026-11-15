//! Offscreen UI snapshots, used while developing to check layout, fonts,
//! icons and colours without needing a visible window.
//!
//! This module is only compiled for `cargo test`, so none of it ends up in the
//! shipped executable. Run `cargo test -- --nocapture` and look at `shots/`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui_kittest::Harness;

use crate::app::LumenApp;
use crate::i18n::Lang;
use crate::settings::{Settings, ThemePref};

/// Where the rendered PNG files are written.
const SHOTS_DIR: &str = "shots";

/// Build a harness that drives the real application at the given window size.
fn harness(
    settings: Settings,
    lang: Lang,
    path: Option<&str>,
    size: [f32; 2],
) -> Harness<'static, LumenApp> {
    // The application writes its settings and window state to the config
    // directory; tests must not overwrite the real one, so everything is
    // redirected to a temporary folder. Every test sets the same value, so the
    // shared process-wide variable cannot make them disagree.
    let config = std::env::temp_dir().join("lumen-test-config");
    let _ = std::fs::create_dir_all(&config);
    std::env::set_var("LUMEN_CONFIG_DIR", &config);

    let path = path.map(PathBuf::from);
    // The harness forces a theme of its own, so mirror the requested one here.
    let egui_theme = match settings.theme {
        ThemePref::Light => egui::Theme::Light,
        ThemePref::Dark | ThemePref::System => egui::Theme::Dark,
    };
    Harness::builder()
        .with_size(egui::Vec2::new(size[0], size[1]))
        .with_theme(egui_theme)
        .wgpu()
        .build_eframe(move |cc| LumenApp::new(cc, settings, lang, path, Instant::now()))
}

/// Advance a few frames, give the background workers real time to finish, then
/// advance again so the decoded image and thumbnails are on screen.
fn settle(harness: &mut Harness<'_, LumenApp>) {
    harness.run_steps(4);
    std::thread::sleep(Duration::from_millis(900));
    harness.run_steps(4);
    std::thread::sleep(Duration::from_millis(200));
    harness.run_steps(2);
}

/// Render the current frame to `shots/<name>.png`.
fn save(harness: &mut Harness<'_, LumenApp>, name: &str) {
    let image = harness.render().expect("offscreen render failed");
    std::fs::create_dir_all(SHOTS_DIR).expect("could not create the shots folder");
    image
        .save(format!("{SHOTS_DIR}/{name}.png"))
        .expect("could not save the snapshot");
    println!("wrote {SHOTS_DIR}/{name}.png ({}x{})", image.width(), image.height());
}

fn settings(theme: ThemePref, language: crate::settings::LanguagePref) -> Settings {
    Settings {
        theme,
        language,
        ..Settings::default()
    }
}

const PHOTO: &str = "test-images/alpha-gradient.png";
const FOLDER: &str = "test-images";

#[test]
fn snapshot_dark_english_photo() {
    let mut harness = harness(
        settings(ThemePref::Dark, crate::settings::LanguagePref::English),
        Lang::En,
        Some(PHOTO),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    save(&mut harness, "ui-dark-english");
}

#[test]
fn snapshot_light_chinese_photo() {
    let mut harness = harness(
        settings(ThemePref::Light, crate::settings::LanguagePref::Chinese),
        Lang::Zh,
        Some(PHOTO),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    save(&mut harness, "ui-light-chinese");
}

#[test]
fn snapshot_dark_english_folder() {
    let mut harness = harness(
        settings(ThemePref::Dark, crate::settings::LanguagePref::English),
        Lang::En,
        Some(FOLDER),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    save(&mut harness, "ui-dark-english-folder");
}

#[test]
fn snapshot_settings_page_chinese() {
    let mut harness = harness(
        settings(ThemePref::Light, crate::settings::LanguagePref::Chinese),
        Lang::Zh,
        Some(PHOTO),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    // Ctrl+, opens the settings window, exactly like a real user would.
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Comma);
    harness.run_steps(6);
    save(&mut harness, "ui-settings-chinese");
}

#[test]
fn snapshot_settings_page_bottom() {
    let mut harness = harness(
        settings(ThemePref::Light, crate::settings::LanguagePref::Chinese),
        Lang::Zh,
        Some(PHOTO),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::Comma);
    harness.run_steps(4);
    // Scroll the settings page down so the lower sections are visible too.
    harness.hover_at(egui::pos2(560.0, 400.0));
    for _ in 0..6 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(2);
    }
    save(&mut harness, "ui-settings-chinese-bottom");
}

#[test]
fn snapshot_rotated_with_info() {
    let mut harness = harness(
        settings(ThemePref::Dark, crate::settings::LanguagePref::English),
        Lang::En,
        Some(PHOTO),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    // Actual size, rotate a quarter turn, then show the information overlay.
    harness.key_press(egui::Key::Num1);
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::R);
    harness.run_steps(2);
    harness.key_press(egui::Key::I);
    harness.run_steps(3);
    save(&mut harness, "ui-rotated-info");
}

#[test]
fn snapshot_slideshow_advances() {
    let mut harness = harness(
        Settings {
            theme: ThemePref::Dark,
            language: crate::settings::LanguagePref::English,
            // Shortest allowed interval, so the test only waits about a second.
            slideshow_interval: 0.5,
            ..Settings::default()
        },
        Lang::En,
        Some(FOLDER),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    harness.key_press(egui::Key::Space);
    harness.run_steps(2);
    // The slideshow is driven by real time, so let the clock run.
    std::thread::sleep(Duration::from_millis(1400));
    harness.run_steps(4);
    std::thread::sleep(Duration::from_millis(700));
    harness.run_steps(4);
    save(&mut harness, "ui-slideshow");
}

/// Builds a folder of small numbered images so the filmstrip overflows.
///
/// Each caller passes its own name: the tests run in parallel and one of them
/// deletes its folder at the end, so a shared path would make them race.
fn make_filmstrip_folder(name: &str, count: usize) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create folder");
    for index in 0..count {
        let mut image = image::RgbaImage::new(64, 48);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgba([
                (index * 7 % 256) as u8,
                (x * 4) as u8,
                (y * 5) as u8,
                255,
            ]);
        }
        image
            .save(dir.join(format!("img-{index:03}.png")))
            .expect("write test image");
    }
    dir
}

#[test]
fn filmstrip_scrolls_with_the_mouse_wheel() {
    let dir = make_filmstrip_folder("lumen-filmstrip-wheel", 30);
    let mut harness = harness(
        Settings::default(),
        Lang::En,
        Some(dir.to_str().unwrap()),
        [900.0, 600.0],
    );
    settle(&mut harness);

    // Hover first and let the hover highlight settle, so the only difference
    // between the two renders below can be the scroll position.
    harness.hover_at(egui::pos2(450.0, 524.0));
    harness.run_steps(4);
    let before = harness.render().expect("render");

    // Turn a plain (vertical) mouse wheel over the strip.
    for _ in 0..4 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(3);
    }
    harness.run_steps(6);

    let after = harness.render().expect("render");
    assert_ne!(
        before.as_raw(),
        after.as_raw(),
        "the filmstrip did not scroll when the mouse wheel turned"
    );

    save(&mut harness, "ui-filmstrip-scrolled");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_pixel_view() {
    let mut harness = harness(
        Settings {
            theme: ThemePref::Dark,
            language: crate::settings::LanguagePref::English,
            // Both pixel helpers are opt-in; this snapshot documents them on.
            pixel_grid: true,
            hover_highlight: true,
            ..Settings::default()
        },
        Lang::En,
        Some("test-images/pixel-grid.png"),
        [1120.0, 760.0],
    );
    settle(&mut harness);

    // Zoom in until single pixels are large, then point at the middle pixel so
    // the coordinate and colour readout has something to show.
    for _ in 0..11 {
        harness.key_press(egui::Key::Plus);
        harness.run_steps(1);
    }
    harness.hover_at(egui::pos2(560.0, 342.0));
    harness.run_steps(4);
    save(&mut harness, "ui-pixel-view");
}

#[test]
fn animated_gif_actually_plays() {
    let mut harness = harness(
        Settings {
            theme: ThemePref::Dark,
            language: crate::settings::LanguagePref::English,
            show_info: true,
            ..Settings::default()
        },
        Lang::En,
        Some("test-images/animation.gif"),
        [900.0, 600.0],
    );
    settle(&mut harness);

    let first = harness.render().expect("render");
    save(&mut harness, "ui-animation-a");

    // The frames are 120 ms apart, so this must land on a different one.
    std::thread::sleep(Duration::from_millis(220));
    harness.run_steps(3);
    let second = harness.render().expect("render");
    save(&mut harness, "ui-animation-b");

    assert_ne!(
        first.as_raw(),
        second.as_raw(),
        "the GIF did not advance to another frame"
    );
}

#[test]
fn snapshot_shortcuts_page_english() {
    let mut harness = harness(
        settings(ThemePref::Dark, crate::settings::LanguagePref::English),
        Lang::En,
        Some(PHOTO),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    harness.key_press(egui::Key::F1);
    harness.run_steps(6);
    save(&mut harness, "ui-shortcuts-english");
}

/// Extent of the photo around the centre of the window, found by walking out
/// from the middle until the canvas background starts. Keeping the test free of
/// layout constants means it still works if a panel changes height.
fn photo_bounds(image: &image::RgbaImage) -> (u32, u32, u32, u32) {
    let (width, height) = image.dimensions();
    // A pixel just inside the canvas but outside the centred photo.
    let background = *image.get_pixel(4, height / 2);
    let differs = |x: u32, y: u32| *image.get_pixel(x, y) != background;
    let (cx, cy) = (width / 2, height / 2);

    let mut left = cx;
    while left > 0 && differs(left - 1, cy) {
        left -= 1;
    }
    let mut right = cx;
    while right + 1 < width && differs(right + 1, cy) {
        right += 1;
    }
    let mut top = cy;
    while top > 0 && differs(cx, top - 1) {
        top -= 1;
    }
    let mut bottom = cy;
    while bottom + 1 < height && differs(cx, bottom + 1) {
        bottom += 1;
    }
    (left, top, right, bottom)
}

/// Share of pixels along one line that survive a mirror, allowing a little
/// slack for texture filtering.
fn mirrored_share(
    original: &image::RgbaImage,
    flipped: &image::RgbaImage,
    line: impl Iterator<Item = ((u32, u32), (u32, u32))>,
) -> (usize, usize) {
    let mut matching = 0;
    let mut compared = 0;
    for ((ox, oy), (fx, fy)) in line {
        let before = original.get_pixel(ox, oy);
        let after = flipped.get_pixel(fx, fy);
        if before
            .0
            .iter()
            .zip(after.0.iter())
            .all(|(a, b)| a.abs_diff(*b) <= 8)
        {
            matching += 1;
        }
        compared += 1;
    }
    (matching, compared)
}

#[test]
fn flipping_mirrors_the_picture_and_is_reversible() {
    let mut harness = harness(
        settings(ThemePref::Dark, crate::settings::LanguagePref::English),
        Lang::En,
        // An opaque photo, so every step outwards from the centre stays
        // inside the picture and the bounds are reliable.
        Some("test-images/wallpaper-1.jpg"),
        [900.0, 600.0],
    );
    settle(&mut harness);
    let original = harness.render().expect("render");
    let (left, top, right, bottom) = photo_bounds(&original);
    assert!(
        right > left + 50 && bottom > top + 50,
        "the photo was not found on screen"
    );

    harness.key_press(egui::Key::H);
    harness.run_steps(3);
    let flipped = harness.render().expect("render");
    save(&mut harness, "ui-flipped-horizontal");

    // A horizontal mirror means "column x now shows what column right-(x-left)
    // showed". Comparing the middle row proves a real mirror happened, rather
    // than just the highlighted toolbar button changing the picture.
    let middle = (top + bottom) / 2;
    let (matching, compared) = mirrored_share(
        &original,
        &flipped,
        (left..=right).map(|x| ((left + right - x, middle), (x, middle))),
    );
    assert!(
        matching * 100 >= compared * 95,
        "only {matching} of {compared} pixels were mirrored horizontally"
    );

    harness.key_press(egui::Key::H);
    harness.run_steps(3);
    let restored = harness.render().expect("render");
    // Only the photo is compared: the filmstrip keeps filling in thumbnails in
    // the background, so comparing the whole window would be flaky.
    for y in top..=bottom {
        for x in left..=right {
            assert_eq!(
                original.get_pixel(x, y),
                restored.get_pixel(x, y),
                "pixel ({x}, {y}) was not restored"
            );
        }
    }

    harness.key_press(egui::Key::V);
    harness.run_steps(3);
    let flipped_vertically = harness.render().expect("render");
    save(&mut harness, "ui-flipped-vertical");

    let middle_column = (left + right) / 2;
    let (matching, compared) = mirrored_share(
        &original,
        &flipped_vertically,
        (top..=bottom).map(|y| ((middle_column, top + bottom - y), (middle_column, y))),
    );
    assert!(
        matching * 100 >= compared * 95,
        "only {matching} of {compared} pixels were mirrored vertically"
    );
}

/// The hover box is drawn over the pixel the status bar reports; the setting
/// has to be able to remove it entirely.
#[test]
fn the_hover_highlight_can_be_turned_off() {
    let build = |highlight: bool| {
        let mut harness = harness(
            Settings {
                theme: ThemePref::Dark,
                language: crate::settings::LanguagePref::English,
                hover_highlight: highlight,
                ..Settings::default()
            },
            Lang::En,
            Some("test-images/pixel-grid.png"),
            [1120.0, 760.0],
        );
        settle(&mut harness);
        for _ in 0..11 {
            harness.key_press(egui::Key::Plus);
            harness.run_steps(1);
        }
        harness.hover_at(egui::pos2(560.0, 342.0));
        harness.run_steps(4);
        harness
    };

    let mut with_box = build(true);
    let mut without = build(false);

    let a = with_box.render().expect("render");
    let b = without.render().expect("render");
    assert_ne!(
        a.as_raw(),
        b.as_raw(),
        "turning the hover highlight off changed nothing on screen"
    );
    save(&mut without, "ui-pixel-view-no-highlight");
}

#[test]
fn snapshot_svg() {
    let mut harness = harness(
        Settings {
            theme: ThemePref::Dark,
            language: crate::settings::LanguagePref::English,
            show_info: true,
            ..Settings::default()
        },
        Lang::En,
        Some("test-images/landscape.svg"),
        [1000.0, 720.0],
    );
    settle(&mut harness);
    // The first SVG with text pays for scanning the installed fonts once.
    std::thread::sleep(Duration::from_millis(1200));
    harness.run_steps(4);
    save(&mut harness, "ui-svg");
}

#[test]
fn snapshot_psd() {
    let mut harness = harness(
        Settings {
            theme: ThemePref::Dark,
            language: crate::settings::LanguagePref::English,
            show_info: true,
            ..Settings::default()
        },
        Lang::En,
        Some("test-images/artwork.psd"),
        [1000.0, 720.0],
    );
    settle(&mut harness);
    save(&mut harness, "ui-psd");
}

/// Scrolling far away from the selected image must still produce thumbnails.
///
/// The strip used to request only the neighbourhood of the selection, so after
/// a long scroll every visible slot showed a "…" placeholder forever.
#[test]
fn scrolling_far_loads_the_thumbnails_that_come_into_view() {
    let dir = make_filmstrip_folder("lumen-filmstrip-far", 60);
    let mut harness = harness(
        Settings::default(),
        Lang::En,
        Some(dir.to_str().unwrap()),
        [900.0, 600.0],
    );
    settle(&mut harness);

    // Nothing but the selection and its neighbours is loaded at this point.
    assert!(
        !harness.state().thumbnail_ready(59),
        "the last image should not be cached before scrolling"
    );

    // Turn the wheel far past the end of the strip; the selection must not move.
    harness.hover_at(egui::pos2(450.0, 524.0));
    for _ in 0..40 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(1);
    }
    // Give the decode threads time to finish the newly visible ones.
    std::thread::sleep(Duration::from_millis(900));
    harness.run_steps(6);

    assert_eq!(
        harness.state().selected_index(),
        0,
        "the selection must not move"
    );
    assert!(
        harness.state().thumbnail_ready(59),
        "the thumbnails at the far end of the strip were never requested"
    );
    save(&mut harness, "ui-filmstrip-scrolled-far");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pixel helpers are off by default: a fresh install must show a clean
/// picture, and the shortcuts must be able to turn them on.
#[test]
fn the_pixel_helpers_are_off_by_default() {
    let defaults = Settings::default();
    assert!(!defaults.pixel_grid, "the pixel grid must start hidden");
    assert!(
        !defaults.hover_highlight,
        "the cursor highlight must start hidden"
    );
    assert!(defaults.show_toolbar, "the toolbar must be visible at first");
}

#[test]
fn the_toolbar_can_be_hidden_with_ctrl_t() {
    let mut harness = harness(
        settings(ThemePref::Dark, crate::settings::LanguagePref::English),
        Lang::En,
        Some(PHOTO),
        [900.0, 600.0],
    );
    settle(&mut harness);
    let with_toolbar = harness.state().canvas_height();

    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::T);
    harness.run_steps(3);
    let without_toolbar = harness.state().canvas_height();
    assert!(
        without_toolbar > with_toolbar + 40.0,
        "hiding the toolbar did not give the photo more room ({with_toolbar} -> {without_toolbar})"
    );
    save(&mut harness, "ui-toolbar-hidden");

    // The filmstrip shortcut is plain T and must not have fired as well.
    harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::T);
    harness.run_steps(3);
    assert!(
        (harness.state().canvas_height() - with_toolbar).abs() < 0.5,
        "Ctrl+T did not bring the toolbar back"
    );
}

#[test]
fn snapshot_shortcuts_page_as_a_window() {
    let mut harness = harness(
        settings(ThemePref::Dark, crate::settings::LanguagePref::English),
        Lang::En,
        Some(PHOTO),
        [1120.0, 760.0],
    );
    settle(&mut harness);
    harness.key_press(egui::Key::F1);
    harness.run_steps(6);
    // The list is a window of its own now, so the photo behind it must still be
    // visible around it rather than being replaced by an in-app overlay.
    save(&mut harness, "ui-shortcuts-window");
}
