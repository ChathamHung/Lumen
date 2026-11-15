//! Remembers the size, position and maximised state of the main window.
//!
//! Kept in its own file next to `settings.json` so that importing or exporting
//! settings never moves somebody's window, and so that a corrupted window file
//! can never take the settings with it. A missing file simply means "first
//! launch", which opens the window maximised.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Smallest window the user can drag the frame down to.
pub const MIN_WIDTH: f32 = 640.0;
pub const MIN_HEIGHT: f32 = 420.0;

/// Size used when the window is first restored (un-maximised).
const DEFAULT_WIDTH: f32 = 1120.0;
const DEFAULT_HEIGHT: f32 = 760.0;

/// Positions outside this range are almost certainly stale: the monitor they
/// were measured on is gone. Restoring them would put the window off-screen.
const POSITION_LIMIT: f32 = 16_384.0;

/// Geometry of the main window, persisted between runs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    /// Inner size in logical points, used when the window is not maximised.
    pub width: f32,
    pub height: f32,
    /// Outer top-left corner, if it is known.
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            x: None,
            y: None,
            // Lumen is a photo viewer: it opens filling the screen, which is
            // how most people look at pictures.
            maximized: true,
        }
    }
}

impl WindowState {
    /// Rejects values a hand-edited file or a monitor change could produce.
    pub fn sanitize(&mut self) {
        self.width = sane_size(self.width, MIN_WIDTH, DEFAULT_WIDTH);
        self.height = sane_size(self.height, MIN_HEIGHT, DEFAULT_HEIGHT);
        self.x = self.x.filter(|value| value.is_finite() && value.abs() <= POSITION_LIMIT);
        self.y = self.y.filter(|value| value.is_finite() && value.abs() <= POSITION_LIMIT);
        // A position without both coordinates is unusable.
        if self.x.is_none() || self.y.is_none() {
            self.x = None;
            self.y = None;
        }
    }

    /// Top-left corner to restore, if there is a trustworthy one.
    pub fn position(&self) -> Option<[f32; 2]> {
        Some([self.x?, self.y?])
    }

    /// Reads the file, falling back to "maximised, default size".
    pub fn load() -> Self {
        let path = path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(mut state) => {
                state.sanitize();
                state
            }
            Err(error) => {
                crate::diag::log(&format!("window: ignoring unreadable file: {error}"));
                Self::default()
            }
        }
    }

    /// Writes the file atomically (temp file + rename), ignoring failures:
    /// losing the window geometry is never worth interrupting the user.
    pub fn save(&self) {
        let path = path();
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_err() {
                return;
            }
        }
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        let temp = path.with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
    }
}

fn sane_size(value: f32, min: f32, fallback: f32) -> f32 {
    if value.is_finite() && value >= min {
        value.min(POSITION_LIMIT)
    } else {
        fallback
    }
}

/// Full path of `window.json`, next to `settings.json`.
fn path() -> PathBuf {
    crate::settings::config_dir().join("window.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_a_maximised_window() {
        let state = WindowState::default();
        assert!(state.maximized);
        assert_eq!(state.width, DEFAULT_WIDTH);
        assert_eq!(state.position(), None);
    }

    #[test]
    fn sanitize_rejects_impossible_geometry() {
        let mut state = WindowState {
            width: 10.0,
            height: f32::NAN,
            x: Some(99_999.0),
            y: Some(200.0),
            maximized: false,
        };
        state.sanitize();
        assert_eq!(state.width, DEFAULT_WIDTH, "too narrow falls back to the default");
        assert_eq!(state.height, DEFAULT_HEIGHT, "NaN falls back to the default");
        assert_eq!(state.position(), None, "one bad coordinate drops both");
    }

    #[test]
    fn a_normal_geometry_survives_sanitize() {
        let mut state = WindowState {
            width: 1440.0,
            height: 900.0,
            x: Some(-1200.0),
            y: Some(40.0),
            maximized: false,
        };
        state.sanitize();
        assert_eq!((state.width, state.height), (1440.0, 900.0));
        assert_eq!(state.position(), Some([-1200.0, 40.0]));
    }

    #[test]
    fn round_trips_through_json() {
        let original = WindowState {
            width: 1000.0,
            height: 700.0,
            x: Some(20.0),
            y: Some(30.0),
            maximized: false,
        };
        let text = serde_json::to_string(&original).expect("serialise");
        let restored: WindowState = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(restored, original);
    }
}
