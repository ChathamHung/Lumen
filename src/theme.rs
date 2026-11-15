//! Colours, fonts and shared textures.
//!
//! Two palettes are registered at once (light and dark). egui picks whichever
//! one matches the active theme, so switching between "follow system", light
//! and dark is instant and needs no extra bookkeeping.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Theme, Visuals,
};

use crate::settings::{BackgroundStyle, ThemePref};

/// Accent colour used for the dark theme.
pub const ACCENT_DARK: Color32 = Color32::from_rgb(0x5B, 0x9D, 0xFF);
/// Accent colour used for the light theme (slightly deeper for contrast).
pub const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x2F, 0x6F, 0xED);

/// Applies the user's theme preference together with the Lumen palettes.
pub fn apply(ctx: &egui::Context, preference: ThemePref) {
    // Start from egui's dark style: it is the base for spacing and text styles
    // in both themes, only the colours differ.
    let mut base = (*ctx.style_of(Theme::Dark)).clone();
    base.text_styles = text_styles();
    base.spacing.item_spacing = egui::vec2(8.0, 6.0);
    base.spacing.button_padding = egui::vec2(10.0, 5.0);
    base.visuals.window_corner_radius = CornerRadius::same(10);
    base.visuals.menu_corner_radius = CornerRadius::same(8);

    let mut dark = base.clone();
    dark.visuals = dark_visuals();
    ctx.set_style_of(Theme::Dark, dark);

    let mut light = base;
    light.visuals = light_visuals();
    ctx.set_style_of(Theme::Light, light);

    ctx.set_theme(match preference {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Light => egui::ThemePreference::Light,
        ThemePref::Dark => egui::ThemePreference::Dark,
    });
}

/// Slightly larger default text, which reads better for a media application.
fn text_styles() -> std::collections::BTreeMap<TextStyle, FontId> {
    [
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Heading, FontId::proportional(19.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into()
}

fn dark_visuals() -> Visuals {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(0x16, 0x18, 0x1C);
    visuals.window_fill = Color32::from_rgb(0x1C, 0x1F, 0x25);
    visuals.faint_bg_color = Color32::from_rgb(0x22, 0x26, 0x2D);
    visuals.extreme_bg_color = Color32::from_rgb(0x0E, 0x10, 0x13);
    visuals.code_bg_color = Color32::from_rgb(0x11, 0x13, 0x17);
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(0x2E, 0x33, 0x3B));
    visuals.selection.bg_fill = ACCENT_DARK.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_DARK);
    visuals.hyperlink_color = ACCENT_DARK;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = Color32::from_rgb(0x1A, 0x1D, 0x22);
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x2A, 0x2F, 0x37));
    widgets.inactive.bg_fill = Color32::from_rgb(0x24, 0x28, 0x30);
    widgets.inactive.weak_bg_fill = Color32::from_rgb(0x24, 0x28, 0x30);
    widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x2E, 0x34, 0x3C));
    widgets.hovered.bg_fill = Color32::from_rgb(0x2E, 0x34, 0x3E);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2E, 0x34, 0x3E);
    widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x44, 0x4C, 0x58));
    widgets.active.bg_fill = Color32::from_rgb(0x38, 0x3F, 0x4B);
    widgets.active.weak_bg_fill = Color32::from_rgb(0x38, 0x3F, 0x4B);
    visuals
}

fn light_visuals() -> Visuals {
    let mut visuals = Visuals::light();
    visuals.panel_fill = Color32::from_rgb(0xF7, 0xF8, 0xFA);
    visuals.window_fill = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    visuals.faint_bg_color = Color32::from_rgb(0xEE, 0xF0, 0xF3);
    visuals.extreme_bg_color = Color32::from_rgb(0xFF, 0xFF, 0xFF);
    visuals.code_bg_color = Color32::from_rgb(0xF2, 0xF4, 0xF7);
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(0xD8, 0xDD, 0xE4));
    visuals.selection.bg_fill = ACCENT_LIGHT.gamma_multiply(0.28);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_LIGHT);
    visuals.hyperlink_color = ACCENT_LIGHT;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = Color32::from_rgb(0xF2, 0xF4, 0xF7);
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xDD, 0xE2, 0xE8));
    widgets.inactive.bg_fill = Color32::from_rgb(0xEC, 0xEF, 0xF3);
    widgets.inactive.weak_bg_fill = Color32::from_rgb(0xEC, 0xEF, 0xF3);
    widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xD8, 0xDD, 0xE4));
    widgets.hovered.bg_fill = Color32::from_rgb(0xE2, 0xE7, 0xEE);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(0xE2, 0xE7, 0xEE);
    widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xC2, 0xCA, 0xD4));
    widgets.active.bg_fill = Color32::from_rgb(0xD5, 0xDC, 0xE5);
    widgets.active.weak_bg_fill = Color32::from_rgb(0xD5, 0xDC, 0xE5);
    visuals
}

/// Colour painted behind the photo.
pub fn canvas_color(style: BackgroundStyle, dark: bool) -> Color32 {
    match style {
        BackgroundStyle::Dark => Color32::from_rgb(0x0B, 0x0C, 0x0E),
        BackgroundStyle::Light => Color32::from_rgb(0xE8, 0xEA, 0xEE),
        BackgroundStyle::Theme => {
            if dark {
                Color32::from_rgb(0x12, 0x14, 0x17)
            } else {
                Color32::from_rgb(0xF2, 0xF4, 0xF7)
            }
        }
        BackgroundStyle::Checkerboard => {
            if dark {
                Color32::from_rgb(0x0B, 0x0C, 0x0E)
            } else {
                Color32::from_rgb(0xE8, 0xEA, 0xEE)
            }
        }
    }
}

/// Builds the 32x32 transparency checkerboard once and keeps it in the context.
/// The texture is white/grey so a tint colour can adapt it to both themes.
/// Checkerboard used behind photos with transparency.
///
/// The two shades are deliberately close together: the pattern should hint that
/// the area is transparent, not fight with the photo for attention. A separate
/// texture is built for each theme so the cells always sit close to the canvas
/// colour.
pub fn checker_texture(ctx: &egui::Context, dark: bool) -> egui::TextureHandle {
    const SIZE: usize = 64;
    const CELL: usize = 32;

    let (a, b) = if dark { (0x26u8, 0x1Fu8) } else { (0xFFu8, 0xF6u8) };

    let mut pixels = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let value = if ((x / CELL) + (y / CELL)) % 2 == 0 { a } else { b };
            let index = (y * SIZE + x) * 4;
            pixels[index] = value;
            pixels[index + 1] = value;
            pixels[index + 2] = value;
            pixels[index + 3] = 255;
        }
    }

    let image = egui::ColorImage::from_rgba_unmultiplied([SIZE, SIZE], &pixels);
    ctx.load_texture(
        if dark { "lumen.checker.dark" } else { "lumen.checker.light" },
        image,
        egui::TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Nearest,
            wrap_mode: egui::TextureWrapMode::Repeat,
            mipmap_mode: None,
        },
    )
}

/// Installs a CJK-capable system font as a fallback so Chinese text renders
/// correctly. egui's built-in fonts only cover Latin.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    if let Some((bytes, index)) = load_cjk_font() {
        let name = "lumen-cjk".to_owned();
        fonts
            .font_data
            .insert(name.clone(), std::sync::Arc::new(FontData::from_blob(std::sync::Arc::new(bytes), index)));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(list) = fonts.families.get_mut(&family) {
                // Appended, not prepended: Latin text keeps the crisp default font.
                list.push(name.clone());
            }
        }
    } else {
        crate::diag::log("fonts: no CJK system font found, Chinese text may not render");
    }

    ctx.set_fonts(fonts);
}

/// Candidate font files, in order of preference. All of them ship with Windows.
#[cfg(windows)]
const FONT_CANDIDATES: &[(&str, u32)] = &[
    ("C:\\Windows\\Fonts\\msyh.ttc", 0),   // Microsoft YaHei
    ("C:\\Windows\\Fonts\\msyhl.ttc", 0),  // Microsoft YaHei Light
    ("C:\\Windows\\Fonts\\Deng.ttf", 0),   // DengXian
    ("C:\\Windows\\Fonts\\simhei.ttf", 0), // SimHei
    ("C:\\Windows\\Fonts\\simsun.ttc", 0), // SimSun
];

#[cfg(not(windows))]
const FONT_CANDIDATES: &[(&str, u32)] = &[
    ("/System/Library/Fonts/PingFang.ttc", 0),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
];

/// Loads the first usable CJK font. `LUMEN_FONT=/path/to/font.ttf` overrides
/// the search, which also makes it possible to run with a custom typeface.
fn load_cjk_font() -> Option<(Vec<u8>, u32)> {
    if let Ok(custom) = std::env::var("LUMEN_FONT") {
        if let Ok(bytes) = std::fs::read(&custom) {
            if is_sfnt(&bytes) {
                return Some((bytes, 0));
            }
        }
        crate::diag::log(&format!("fonts: LUMEN_FONT={custom} is not a usable font"));
    }

    for (path, index) in FONT_CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if is_sfnt(&bytes) {
            return Some((bytes, *index));
        }
    }
    None
}

/// Sanity check on the first four bytes of an sfnt file.
fn is_sfnt(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    let tag = &bytes[0..4];
    tag == b"ttcf"
        || tag == b"OTTO"
        || tag == b"true"
        || tag == b"typ1"
        || tag == [0x00, 0x01, 0x00, 0x00]
}
