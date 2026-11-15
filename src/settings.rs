//! Persistent user settings, plus JSON import/export.
//!
//! The file lives in `%APPDATA%\Lumen\settings.json`. If a file called
//! `lumen.portable` sits next to `lumen.exe`, the settings are stored there
//! instead so the whole application travels on a USB stick.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Bumped whenever the on-disk layout changes. Old files still load because
/// every field falls back to its default value.
///
/// Version 2 turned the pixel grid and the cursor highlight off by default, so
/// files written by version 1 are migrated instead of keeping the old values.
pub const SETTINGS_VERSION: u32 = 2;

/// Which language the interface uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LanguagePref {
    #[default]
    System,
    English,
    Chinese,
}

/// Which colour theme the interface uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

/// Background behind the photo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BackgroundStyle {
    /// Classic transparency checkerboard.
    Checkerboard,
    /// Flat colour taken from the active theme.
    #[default]
    Theme,
    Dark,
    Light,
}

/// Order in which the images of a folder are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortOrder {
    #[default]
    NameAscending,
    NameDescending,
    NewestFirst,
    LargestFirst,
}

impl SortOrder {
    /// Human readable name used by the log and by future sort implementations.
    pub fn is_metadata_based(self) -> bool {
        matches!(self, Self::NewestFirst | Self::LargestFirst)
    }
}

/// Everything that is persisted between runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,

    // ----- Appearance -----
    pub language: LanguagePref,
    pub theme: ThemePref,
    pub background: BackgroundStyle,

    // ----- Behaviour -----
    pub sort_order: SortOrder,
    /// Re-open the last folder on the next launch.
    pub remember_session: bool,
    pub last_folder: Option<PathBuf>,
    pub last_file: Option<String>,

    /// Open new images fitted to the window instead of at 100%.
    pub open_fitted: bool,
    /// Multiplier for wheel zoom. 1.0 feels natural on most mice.
    pub zoom_sensitivity: f32,
    /// Draw a grid over the image once it is magnified enough for single
    /// pixels to be visible. Off by default: most people want to look at the
    /// picture, not at its pixels.
    pub pixel_grid: bool,
    /// Outline the pixel under the cursor while the image is magnified.
    /// Off by default for the same reason; the status bar still reports the
    /// colour and position of the pixel.
    pub hover_highlight: bool,
    /// Seconds between images while the slideshow runs.
    pub slideshow_interval: f32,
    pub slideshow_loop: bool,

    // ----- Panels -----
    pub show_toolbar: bool,
    pub show_filmstrip: bool,
    pub show_info: bool,
    pub show_status_bar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            language: LanguagePref::System,
            theme: ThemePref::System,
            background: BackgroundStyle::Theme,
            sort_order: SortOrder::NameAscending,
            remember_session: true,
            last_folder: None,
            last_file: None,
            open_fitted: true,
            zoom_sensitivity: 1.0,
            pixel_grid: false,
            hover_highlight: false,
            slideshow_interval: 4.0,
            slideshow_loop: true,
            show_toolbar: true,
            show_filmstrip: true,
            show_info: false,
            show_status_bar: true,
        }
    }
}

impl Settings {
    /// Brings a file written by an older version up to date.
    ///
    /// The only change so far is the default of the two pixel helpers, which
    /// are opt-in from version 2 on. Applying the new default to old files is
    /// what makes "off by default" actually visible on a machine that has run
    /// an earlier build.
    fn migrate(&mut self) {
        if self.version < 2 {
            self.pixel_grid = false;
            self.hover_highlight = false;
        }
    }

    /// Clamps values that a hand-edited or imported file could contain.
    pub fn validate(&mut self) {
        self.version = SETTINGS_VERSION;
        self.zoom_sensitivity = clamp_finite(self.zoom_sensitivity, 0.2, 3.0, 1.0);
        self.slideshow_interval = clamp_finite(self.slideshow_interval, 0.5, 120.0, 4.0);
    }

    /// Reads the settings file, falling back to defaults on any problem.
    pub fn load() -> Self {
        let path = settings_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            // A missing file on first launch is completely normal.
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(mut settings) => {
                settings.migrate();
                settings.validate();
                settings
            }
            Err(error) => {
                crate::diag::log(&format!("settings: ignoring unreadable file: {error}"));
                // Keep the broken file around instead of silently overwriting it.
                let backup = path.with_extension("json.broken");
                let _ = std::fs::rename(&path, backup);
                Self::default()
            }
        }
    }

    /// Writes the settings atomically (temp file + rename).
    pub fn save(&self) -> std::io::Result<()> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, &path)
    }

    /// Writes a copy of the settings to any location the user picks.
    pub fn export_to(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// Reads a settings file chosen by the user and validates it.
    pub fn import_from(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let mut settings: Self = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        settings.migrate();
        settings.validate();
        Ok(settings)
    }
}

/// Directory that holds the settings, the log and the crash log.
///
/// Portable mode: a `lumen.portable` marker next to the executable redirects
/// everything to the executable's own folder.
pub fn config_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        // Tests point this at a temporary folder so that running them never
        // overwrites the settings of whoever is working on the project.
        if let Some(dir) = std::env::var_os("LUMEN_CONFIG_DIR") {
            return PathBuf::from(dir);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(folder) = exe.parent() {
                if folder.join("lumen.portable").exists() {
                    return folder.to_path_buf();
                }
            }
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Lumen")
    })
    .clone()
}

/// Full path of `settings.json`.
pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

fn clamp_finite(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_import_round_trip() {
        let folder = std::env::temp_dir().join("lumen-settings-test");
        std::fs::create_dir_all(&folder).expect("temp folder");
        let file = folder.join("exported.json");

        let original = Settings {
            language: LanguagePref::Chinese,
            theme: ThemePref::Dark,
            background: BackgroundStyle::Dark,
            zoom_sensitivity: 1.7,
            slideshow_interval: 9.5,
            ..Settings::default()
        };
        original.export_to(&file).expect("export should succeed");

        let restored = Settings::import_from(&file).expect("import should succeed");
        assert_eq!(restored.language, original.language);
        assert_eq!(restored.theme, original.theme);
        assert_eq!(restored.background, original.background);
        assert!((restored.zoom_sensitivity - 1.7).abs() < 1e-6);
        assert!((restored.slideshow_interval - 9.5).abs() < 1e-6);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn import_clamps_out_of_range_values() {
        let folder = std::env::temp_dir().join("lumen-settings-test");
        std::fs::create_dir_all(&folder).expect("temp folder");
        let file = folder.join("hand-edited.json");

        // A hand-edited file with impossible values must not crash the app.
        std::fs::write(
            &file,
            r#"{"version":0,"language":"Chinese","theme":"Dark","background":"Theme",
                "sort_order":"NameAscending","remember_session":true,"last_folder":null,
                "last_file":null,"open_fitted":true,"zoom_sensitivity":99.0,
                "slideshow_interval":0.0,"slideshow_loop":true,"show_filmstrip":true,
                "show_info":false,"show_status_bar":true}"#,
        )
        .expect("write");

        let settings = Settings::import_from(&file).expect("import should succeed");
        assert!((settings.zoom_sensitivity - 3.0).abs() < 1e-6);
        assert!((settings.slideshow_interval - 0.5).abs() < 1e-6);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn version_1_files_lose_the_pixel_helpers() {
        let folder = std::env::temp_dir().join("lumen-settings-test");
        std::fs::create_dir_all(&folder).expect("temp folder");
        let file = folder.join("old-version.json");

        // A file from before the defaults changed, with both helpers on.
        let old = Settings {
            version: 1,
            pixel_grid: true,
            hover_highlight: true,
            ..Settings::default()
        };
        old.export_to(&file).expect("export");

        let restored = Settings::import_from(&file).expect("import");
        assert!(!restored.pixel_grid, "the grid must be off after migrating");
        assert!(!restored.hover_highlight, "the highlight must be off after migrating");
        assert_eq!(restored.version, SETTINGS_VERSION);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn current_files_keep_deliberate_pixel_helper_choices() {
        let folder = std::env::temp_dir().join("lumen-settings-test");
        std::fs::create_dir_all(&folder).expect("temp folder");
        let file = folder.join("current-version.json");

        let current = Settings {
            pixel_grid: true,
            hover_highlight: true,
            ..Settings::default()
        };
        current.export_to(&file).expect("export");

        let restored = Settings::import_from(&file).expect("import");
        assert!(restored.pixel_grid, "a deliberate choice must survive");
        assert!(restored.hover_highlight, "a deliberate choice must survive");

        let _ = std::fs::remove_file(&file);
    }
}
