//! Application state, layout and interaction.
//!
//! The window is built from four panels:
//!   toolbar (top) -> filmstrip (bottom) -> status bar (bottom) -> canvas (centre)
//! plus two floating windows (settings, shortcuts) and a toast overlay.
//!
//! Nothing in here blocks: file dialogs are the only synchronous calls and they
//! are native modal windows, which is exactly what the user expects.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    Ui, Vec2,
};

use crate::i18n::{t, Lang, Text};
use crate::icons::Icon;
use crate::image_loader::{
    ImageLoader, LoadedImage, ThumbnailLoader, MAX_TEXTURE_EDGE,
};
use crate::library::{self, Library};
use crate::settings::{BackgroundStyle, LanguagePref, Settings, SortOrder, ThemePref};
use crate::viewer::{ViewState, ZoomMode};
use crate::window_state::WindowState;
use crate::{diag, icons, platform, theme, ui as widgets};

const TOOLBAR_HEIGHT: f32 = 48.0;
const STATUS_HEIGHT: f32 = 28.0;
const FILMSTRIP_HEIGHT: f32 = 96.0;
const THUMB_WIDTH: f32 = 84.0;
const THUMB_HEIGHT: f32 = 64.0;
const THUMB_GAP: f32 = 6.0;
const TOAST_SECONDS: f32 = 3.0;
const AUTOSAVE_DELAY: Duration = Duration::from_millis(800);
/// How many thumbnails on each side of the current image are warmed up when the
/// selection changes. The visible range is requested separately as the strip is
/// scrolled, so this only has to cover a screen or two.
const THUMB_WINDOW: usize = 12;

/// Physical screen pixels per image pixel needed before the pixel grid is
/// drawn. Below this the lines would merge into a flat grey tint.
const PIXEL_GRID_MIN_SIZE: f32 = 8.0;

/// Size of the settings window when it is first opened.
const SETTINGS_WIDTH: f32 = 620.0;
const SETTINGS_HEIGHT: f32 = 700.0;

/// Size of the keyboard shortcut window.
const SHORTCUTS_WIDTH: f32 = 440.0;
const SHORTCUTS_HEIGHT: f32 = 520.0;

/// What the canvas is currently showing.
enum ViewStatus {
    Empty,
    Loading {
        path: PathBuf,
        generation: u64,
        started: Instant,
    },
    Ready,
    Error {
        path: PathBuf,
        message: String,
    },
}

/// A short message shown at the bottom of the window.
struct Toast {
    text: String,
    created: Instant,
}

/// Deferred commands, executed after the frame has been drawn. Keeping them in
/// a list avoids borrowing conflicts and keeps blocking dialogs out of the
/// layout code.
enum Action {
    OpenFolder,
    OpenFile,
    ExportSettings,
    ImportSettings,
    ResetSettings,
    RegisterDefault,
    UnregisterDefault,
    OpenSettingsFolder,
    OpenShortcuts,
}

/// Thumbnail textures with a simple least-recently-used eviction policy.
#[derive(Default)]
struct ThumbCache {
    textures: HashMap<PathBuf, Option<egui::TextureHandle>>,
    order: VecDeque<PathBuf>,
}

impl ThumbCache {
    fn contains(&self, path: &Path) -> bool {
        self.textures.contains_key(path)
    }

    fn get(&self, path: &Path) -> Option<&Option<egui::TextureHandle>> {
        self.textures.get(path)
    }

    fn insert(&mut self, path: PathBuf, texture: Option<egui::TextureHandle>) {
        if self.textures.insert(path.clone(), texture).is_none() {
            self.order.push_back(path);
        }
        while self.order.len() > 400 {
            if let Some(oldest) = self.order.pop_front() {
                self.textures.remove(&oldest);
            }
        }
    }
}

pub struct LumenApp {
    settings: Settings,
    lang: Lang,
    started: Instant,
    startup_ms: Option<f32>,

    library: Library,
    view: ViewState,
    image: Option<LoadedImage>,
    status: ViewStatus,

    loader: ImageLoader,
    thumbs: ThumbnailLoader,
    thumb_cache: ThumbCache,

    settings_open: bool,
    /// Where the settings window was placed when it opened. Kept so the window
    /// does not jump back over the main window every frame while the user is
    /// dragging it somewhere else.
    settings_position: Option<egui::Pos2>,
    shortcuts_open: bool,
    /// Placement of the shortcut window, kept for the same reason as
    /// `settings_position`.
    shortcuts_position: Option<egui::Pos2>,
    toasts: Vec<Toast>,
    fullscreen: bool,
    title: String,
    default_app_registered: bool,

    slideshow_playing: bool,
    slideshow_next: Instant,

    checker: Option<(bool, egui::TextureHandle)>,
    last_canvas: Rect,

    settings_dirty: bool,
    dirty_since: Option<Instant>,
    pending_actions: Vec<Action>,
    pending_view_reset: bool,
    scroll_strip: bool,

    /// Geometry of the main window as last observed, plus the copy that is
    /// already on disk so an unchanged window does not rewrite the file.
    window: WindowState,
    window_saved: WindowState,
    window_save_at: Option<Instant>,
}

impl LumenApp {
    /// Whether the thumbnail of a file has been decoded and cached.
    ///
    /// Only used by the tests: the filmstrip bug it guards against was that
    /// scrolling far away from the selection left "…" placeholders forever,
    /// because nothing ever requested those thumbnails.
    #[cfg(test)]
    pub fn thumbnail_ready(&self, index: usize) -> bool {
        self.library
            .files
            .get(index)
            .is_some_and(|entry| matches!(self.thumb_cache.get(&entry.path), Some(Some(_))))
    }

    /// Index of the file the viewer is showing. Test-only accessor.
    #[cfg(test)]
    pub fn selected_index(&self) -> usize {
        self.library.index
    }

    /// Height of the image canvas. Test-only accessor, used to prove that
    /// hiding the toolbar gives the photo more room.
    #[cfg(test)]
    pub fn canvas_height(&self) -> f32 {
        self.last_canvas.height()
    }

    pub fn new(
        cc: &eframe::CreationContext<'_>,
        settings: Settings,
        lang: Lang,
        initial_path: Option<PathBuf>,
        started: Instant,
    ) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, settings.theme);

        let window = WindowState::load();
        let mut app = Self {
            settings,
            lang,
            started,
            startup_ms: None,
            library: Library::default(),
            view: ViewState::default(),
            image: None,
            status: ViewStatus::Empty,
            loader: ImageLoader::new(cc.egui_ctx.clone()),
            thumbs: ThumbnailLoader::new(cc.egui_ctx.clone()),
            thumb_cache: ThumbCache::default(),
            settings_open: false,
            settings_position: None,
            shortcuts_open: false,
            shortcuts_position: None,
            toasts: Vec::new(),
            fullscreen: false,
            title: String::new(),
            default_app_registered: platform::is_registered(),
            slideshow_playing: false,
            slideshow_next: Instant::now(),
            checker: None,
            last_canvas: Rect::NOTHING,
            settings_dirty: false,
            dirty_since: None,
            pending_actions: Vec::new(),
            pending_view_reset: false,
            scroll_strip: false,
            window,
            window_saved: window,
            window_save_at: None,
        };

        // An explicit path (file association / command line) wins, then the
        // remembered session, and otherwise the empty state is shown.
        let mut opened = false;
        if let Some(path) = initial_path {
            opened = app.open_path(&path);
        }
        if !opened && app.settings.remember_session {
            if let Some(folder) = app.settings.last_folder.clone() {
                if folder.is_dir() {
                    let name = app.settings.last_file.clone();
                    opened = app.open_folder(&folder, name.as_deref());
                }
            }
        }
        if !opened {
            app.settings.last_file = None;
        }

        app
    }

    // ---------------------------------------------------------------------
    // Opening files and folders
    // ---------------------------------------------------------------------

    fn open_path(&mut self, path: &Path) -> bool {
        if path.is_dir() {
            return self.open_folder(path, None);
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self.open_folder(parent, name.as_deref())
    }

    fn open_folder(&mut self, folder: &Path, select: Option<&str>) -> bool {
        match self.library.open(folder, self.settings.sort_order) {
            Ok(()) => {
                if let Some(name) = select {
                    self.library.select_by_name(name);
                }
                self.settings.last_folder = Some(folder.to_path_buf());
                self.start_loading_current();
                self.scroll_strip = true;
                self.mark_dirty();
                true
            }
            Err(error) => {
                self.toast(format!("{}: {error}", t(self.lang, Text::FolderFailed)));
                false
            }
        }
    }

    /// Starts decoding the currently selected file.
    fn start_loading_current(&mut self) {
        self.image = None;
        self.pending_view_reset = true;

        let Some(entry) = self.library.current() else {
            self.status = ViewStatus::Empty;
            return;
        };

        let path = entry.path.clone();
        self.settings.last_file = Some(entry.name.clone());
        let generation = self.loader.request(path.clone(), MAX_TEXTURE_EDGE);
        self.status = ViewStatus::Loading {
            path,
            generation,
            started: Instant::now(),
        };
        self.mark_dirty();
    }

    fn navigate(&mut self, delta: isize) {
        if self.library.step(delta) {
            self.start_loading_current();
        } else if !self.library.is_empty() {
            self.toast(t(self.lang, Text::NoMoreImages));
        }
    }

    // ---------------------------------------------------------------------
    // Background work
    // ---------------------------------------------------------------------

    fn poll_workers(&mut self, ctx: &egui::Context) {
        while let Some(response) = self.loader.poll() {
            let expected = match &self.status {
                ViewStatus::Loading { generation, .. } => Some(*generation),
                _ => None,
            };
            if expected != Some(response.generation) {
                continue;
            }

            let path = match &self.status {
                ViewStatus::Loading { path, .. } => path.clone(),
                _ => PathBuf::new(),
            };

            match response.result {
                Ok(decoded) => {
                    diag::log(&format!(
                        "loaded {} in {:.1} ms ({}x{})",
                        path.display(),
                        decoded.decode_ms,
                        decoded.width,
                        decoded.height
                    ));
                    self.image = Some(decoded.into_loaded(ctx));
                    self.status = ViewStatus::Ready;
                    self.pending_view_reset = true;
                    self.scroll_strip = true;
                }
                Err(message) => {
                    diag::log(&format!("load failed for {}: {message}", path.display()));
                    self.status = ViewStatus::Error { path, message };
                }
            }
        }

        while let Some(result) = self.thumbs.poll() {
            match result.result {
                Ok(thumb) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(thumb.size, &thumb.pixels);
                    let texture = ctx.load_texture(
                        format!("lumen.thumb:{}", thumb.path.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.thumb_cache.insert(thumb.path, Some(texture));
                }
                Err(_) => self.thumb_cache.insert(result.path, None),
            }
        }
    }

    // ---------------------------------------------------------------------
    // Input
    // ---------------------------------------------------------------------

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.first() {
            self.open_path(path);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Never steal keys from a focused text field.
        if ctx.egui_wants_keyboard_input() {
            return;
        }

        use egui::Key;
        let (
            next,
            previous,
            first,
            last,
            zoom_in,
            zoom_out,
            fit,
            actual,
            rotate_left,
            rotate_right,
            slideshow,
            info,
            strip,
            fullscreen,
            escape,
            open_settings,
            open_shortcuts,
            open_folder,
            open_file,
        ) = ctx.input(|input| {
            // `command` is Ctrl on Windows/Linux and Cmd on macOS. Ctrl is
            // accepted as well so the shortcuts keep working no matter how the
            // modifier state was produced (some input paths only set `ctrl`).
            let command = input.modifiers.command || input.modifiers.ctrl;
            (
                input.key_pressed(Key::ArrowRight) || input.key_pressed(Key::PageDown),
                input.key_pressed(Key::ArrowLeft) || input.key_pressed(Key::PageUp),
                input.key_pressed(Key::Home),
                input.key_pressed(Key::End),
                input.key_pressed(Key::Plus) || input.key_pressed(Key::Equals),
                input.key_pressed(Key::Minus),
                input.key_pressed(Key::Num0),
                input.key_pressed(Key::Num1),
                input.key_pressed(Key::R) && !input.modifiers.shift,
                input.key_pressed(Key::R) && input.modifiers.shift,
                input.key_pressed(Key::Space),
                input.key_pressed(Key::I) && !command,
                // Ctrl+T is the toolbar, so plain T must not fire with it.
                input.key_pressed(Key::T) && !command,
                input.key_pressed(Key::F11),
                input.key_pressed(Key::Escape),
                command && input.key_pressed(Key::Comma),
                input.key_pressed(Key::F1),
                command && input.key_pressed(Key::O) && !input.modifiers.shift,
                command && input.modifiers.shift && input.key_pressed(Key::O),
            )
        });

        if escape {
            if self.fullscreen {
                self.set_fullscreen(ctx, false);
            } else if self.settings_open {
                self.settings_open = false;
            } else if self.shortcuts_open {
                self.shortcuts_open = false;
            }
        }
        if open_settings {
            self.open_settings();
        }
        if open_shortcuts {
            self.open_shortcuts();
        }
        if open_folder {
            self.pending_actions.push(Action::OpenFolder);
        }
        if open_file {
            self.pending_actions.push(Action::OpenFile);
        }
        if fullscreen {
            self.set_fullscreen(ctx, !self.fullscreen);
        }
        if info {
            self.settings.show_info = !self.settings.show_info;
            self.mark_dirty();
        }
        if strip {
            self.settings.show_filmstrip = !self.settings.show_filmstrip;
            self.mark_dirty();
        }
        if slideshow {
            self.toggle_slideshow();
        }
        if next {
            self.navigate(1);
        }
        if previous {
            self.navigate(-1);
        }
        if first && self.library.first() {
            self.start_loading_current();
        }
        if last && self.library.last() {
            self.start_loading_current();
        }
        if zoom_in {
            self.zoom_keyboard(1.25);
        }
        if zoom_out {
            self.zoom_keyboard(0.8);
        }
        if fit {
            self.fit_to_window();
        }
        if actual {
            self.actual_size();
        }
        if rotate_left {
            self.rotate(-90);
        }
        if rotate_right {
            self.rotate(90);
        }

        // Secondary shortcuts, handled separately so the list above stays
        // readable. `plain` keeps them from stealing Ctrl+H/Ctrl+V style
        // combinations that other tools bind to other things.
        let (flip_horizontal, flip_vertical, toggle_grid, reset_view, toggle_toolbar) = ctx
            .input(|input| {
                let command = input.modifiers.command || input.modifiers.ctrl;
                let plain = !command && !input.modifiers.alt;
                (
                    plain && input.key_pressed(Key::H),
                    plain && input.key_pressed(Key::V),
                    plain && input.key_pressed(Key::G),
                    command && input.key_pressed(Key::Num0),
                    command && input.key_pressed(Key::T),
                )
            });
        if flip_horizontal {
            self.flip_horizontal();
        }
        if flip_vertical {
            self.flip_vertical();
        }
        if toggle_grid {
            self.settings.pixel_grid = !self.settings.pixel_grid;
            self.mark_dirty();
            let message = if self.settings.pixel_grid {
                t(self.lang, Text::PixelGrid)
            } else {
                t(self.lang, Text::PixelGridOff)
            };
            self.toast(message.to_owned());
        }
        if reset_view {
            self.reset_view();
        }
        if toggle_toolbar {
            self.settings.show_toolbar = !self.settings.show_toolbar;
            self.mark_dirty();
        }
    }

    fn toggle_slideshow(&mut self) {
        if self.library.len() < 2 {
            return;
        }
        self.slideshow_playing = !self.slideshow_playing;
        if self.slideshow_playing {
            self.slideshow_next =
                Instant::now() + Duration::from_secs_f32(self.settings.slideshow_interval);
        }
    }

    /// Advances an animated GIF or APNG. The texture is updated in place, so no
    /// new GPU texture is created while the animation runs.
    fn tick_animation(&mut self, ctx: &egui::Context) {
        let Some(image) = self.image.as_mut() else {
            return;
        };
        if !image.is_animated() {
            return;
        }
        if let Some(wait) = image.advance_frame(Instant::now()) {
            // Wake up exactly when the next frame is due.
            ctx.request_repaint_after(wait.max(Duration::from_millis(1)));
        }
    }

    fn tick_slideshow(&mut self, ctx: &egui::Context) {
        if !self.slideshow_playing {
            return;
        }
        if self.library.len() < 2 {
            self.slideshow_playing = false;
            return;
        }

        let now = Instant::now();
        if now >= self.slideshow_next {
            let moved = if self.settings.slideshow_loop {
                self.library.step_wrapped(1)
            } else if self.library.at_end() {
                false
            } else {
                self.library.step(1)
            };

            if moved {
                self.start_loading_current();
            } else {
                self.slideshow_playing = false;
            }
            self.slideshow_next = now + Duration::from_secs_f32(self.settings.slideshow_interval);
        }
        ctx.request_repaint_after(Duration::from_millis(120));
    }

    // ---------------------------------------------------------------------
    // View commands (shared by toolbar and keyboard)
    // ---------------------------------------------------------------------

    fn fit_to_window(&mut self) {
        let Some(image) = &self.image else { return };
        let size = image.size();
        self.view.mode = ZoomMode::Fit;
        self.view.zoom =
            ViewState::fit_zoom(self.last_canvas, ViewState::rotated_size(size, self.view.rotation));
        self.view.offset = Vec2::ZERO;
    }

    fn actual_size(&mut self) {
        let Some(image) = &self.image else { return };
        let size = image.size();
        self.view.zoom_to(1.0, None, self.last_canvas, size);
    }

    fn zoom_keyboard(&mut self, factor: f32) {
        let Some(image) = &self.image else { return };
        let size = image.size();
        self.view.zoom_by(factor, None, self.last_canvas, size);
    }

    fn rotate(&mut self, degrees: i32) {
        let Some(image) = &self.image else { return };
        let size = image.size();
        self.view.rotate(degrees, self.last_canvas, size);
    }

    fn flip_horizontal(&mut self) {
        let Some(image) = &self.image else { return };
        let size = image.size();
        self.view.flip_horizontal();
        self.view
            .clamp(self.last_canvas, ViewState::rotated_size(size, self.view.rotation));
    }

    fn flip_vertical(&mut self) {
        let Some(image) = &self.image else { return };
        let size = image.size();
        self.view.flip_vertical();
        self.view
            .clamp(self.last_canvas, ViewState::rotated_size(size, self.view.rotation));
    }

    /// Undoes rotation and mirroring and returns to the opening zoom.
    fn reset_view(&mut self) {
        let Some(image) = &self.image else { return };
        let size = image.size();
        self.view.reset_orientation();
        self.view
            .reset(self.last_canvas, size, self.settings.open_fitted);
    }

    fn set_fullscreen(&mut self, ctx: &egui::Context, fullscreen: bool) {
        self.fullscreen = fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
    }

    // ---------------------------------------------------------------------
    // Frame
    // ---------------------------------------------------------------------

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match self.library.current() {
            Some(entry) => format!(
                "{}  ·  {}  ·  Lumen",
                entry.name,
                format_position(self.library.index + 1, self.library.len())
            ),
            None => "Lumen".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Mirrors the real window geometry into `self.window` and writes it to
    /// disk a moment after it settles. Dragging or resizing produces a burst of
    /// frames, so the write is debounced instead of happening on every frame.
    fn track_window(&mut self, ctx: &egui::Context) {
        let (inner, outer, maximized) = ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.inner_rect, viewport.outer_rect, viewport.maximized)
        });

        let mut next = self.window;
        // While the window is maximised its rectangle is the whole screen,
        // which is not the size to restore when it is un-maximised again, so
        // the size and position are only recorded in the restored state.
        if maximized != Some(true) {
            if let Some(rect) = inner {
                next.width = rect.width();
                next.height = rect.height();
            }
            if let Some(rect) = outer {
                next.x = Some(rect.min.x);
                next.y = Some(rect.min.y);
            }
        }
        if let Some(maximized) = maximized {
            next.maximized = maximized;
        }

        if next != self.window {
            self.window = next;
            self.window_save_at = Some(Instant::now() + Duration::from_millis(700));
        }

        if self.window_save_at.is_some_and(|deadline| Instant::now() >= deadline) {
            self.window_save_at = None;
            if self.window != self.window_saved {
                self.window.save();
                self.window_saved = self.window;
            }
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        let lang = self.lang;
        let ctx = ui.ctx().clone();
        let has_images = !self.library.is_empty();
        let has_image = self.image.is_some();
        let can_go_back = self.library.index > 0;
        let can_go_forward = has_images && !self.library.at_end();

        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(6.0);

            if widgets::icon_button(ui, Icon::Folder, t(lang, Text::OpenFolder), false, true)
                .clicked()
            {
                self.pending_actions.push(Action::OpenFolder);
            }
            if widgets::icon_button(ui, Icon::Image, t(lang, Text::OpenFile), false, true).clicked()
            {
                self.pending_actions.push(Action::OpenFile);
            }

            widgets::toolbar_separator(ui);

            if widgets::icon_button(ui, Icon::ChevronLeft, t(lang, Text::Previous), false, can_go_back)
                .clicked()
            {
                self.navigate(-1);
            }
            if widgets::icon_button(ui, Icon::ChevronRight, t(lang, Text::Next), false, can_go_forward)
                .clicked()
            {
                self.navigate(1);
            }

            widgets::toolbar_separator(ui);

            if widgets::icon_button(ui, Icon::Minus, t(lang, Text::ZoomOut), false, has_image).clicked()
            {
                self.zoom_keyboard(0.8);
            }

            let zoom_text = if has_image {
                format!("{:.0}%", self.view.zoom * 100.0)
            } else {
                "—".to_owned()
            };
            ui.add_sized(
                [46.0, 22.0],
                egui::Label::new(RichText::new(zoom_text).size(12.5).weak()).selectable(false),
            );

            if widgets::icon_button(ui, Icon::Plus, t(lang, Text::ZoomIn), false, has_image).clicked()
            {
                self.zoom_keyboard(1.25);
            }
            if widgets::icon_button(
                ui,
                Icon::Fit,
                t(lang, Text::FitToWindow),
                self.view.mode == ZoomMode::Fit,
                has_image,
            )
            .clicked()
            {
                self.fit_to_window();
            }
            if widgets::icon_button(ui, Icon::OneToOne, t(lang, Text::ActualSize), false, has_image)
                .clicked()
            {
                self.actual_size();
            }

            widgets::toolbar_separator(ui);

            if widgets::icon_button(ui, Icon::RotateLeft, t(lang, Text::RotateLeft), false, has_image)
                .clicked()
            {
                self.rotate(-90);
            }
            if widgets::icon_button(ui, Icon::RotateRight, t(lang, Text::RotateRight), false, has_image)
                .clicked()
            {
                self.rotate(90);
            }
            if widgets::icon_button(
                ui,
                Icon::FlipHorizontal,
                t(lang, Text::FlipHorizontal),
                self.view.flip_x,
                has_image,
            )
            .clicked()
            {
                self.flip_horizontal();
            }
            if widgets::icon_button(
                ui,
                Icon::FlipVertical,
                t(lang, Text::FlipVertical),
                self.view.flip_y,
                has_image,
            )
            .clicked()
            {
                self.flip_vertical();
            }
            if widgets::icon_button(ui, Icon::Reset, t(lang, Text::ResetView), false, has_image)
                .clicked()
            {
                self.reset_view();
            }

            widgets::toolbar_separator(ui);

            let (slideshow_icon, slideshow_tip) = if self.slideshow_playing {
                (Icon::Pause, t(lang, Text::StopSlideshow))
            } else {
                (Icon::Play, t(lang, Text::StartSlideshow))
            };
            if widgets::icon_button(
                ui,
                slideshow_icon,
                slideshow_tip,
                self.slideshow_playing,
                has_images,
            )
            .clicked()
            {
                self.toggle_slideshow();
            }

            if widgets::icon_button(
                ui,
                Icon::Filmstrip,
                t(lang, Text::ToggleFilmstrip),
                self.settings.show_filmstrip,
                true,
            )
            .clicked()
            {
                self.settings.show_filmstrip = !self.settings.show_filmstrip;
                self.mark_dirty();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(8.0);

                if widgets::icon_button(
                    ui,
                    Icon::Settings,
                    t(lang, Text::SettingsTitle),
                    self.settings_open,
                    true,
                )
                .clicked()
                {
                    self.open_settings();
                }

                if widgets::icon_button(
                    ui,
                    Icon::Info,
                    t(lang, Text::FileInformation),
                    self.settings.show_info,
                    true,
                )
                .clicked()
                {
                    self.settings.show_info = !self.settings.show_info;
                    self.mark_dirty();
                }

                let (fullscreen_icon, fullscreen_tip) = if self.fullscreen {
                    (Icon::ExitFullscreen, t(lang, Text::ExitFullscreen))
                } else {
                    (Icon::Fullscreen, t(lang, Text::EnterFullscreen))
                };
                if widgets::icon_button(
                    ui,
                    fullscreen_icon,
                    fullscreen_tip,
                    self.fullscreen,
                    true,
                )
                .clicked()
                {
                    self.set_fullscreen(&ctx, !self.fullscreen);
                }
            });
        });
    }

    /// Image pixel under the mouse pointer, when it is over the photo and the
    /// photo is shown at 100% or larger (below that one screen pixel covers
    /// several image pixels, so a single-pixel readout would be misleading).
    fn hovered_pixel(&self, ctx: &egui::Context) -> Option<(u32, u32)> {
        if self.view.zoom < 1.0 {
            return None;
        }
        let image = self.image.as_ref()?;
        let pointer = ctx.pointer_hover_pos()?;
        if !self.last_canvas.contains(pointer) {
            return None;
        }
        self.view.screen_to_pixel(pointer, self.last_canvas, image.size())
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let lang = self.lang;
        // Pixel under the cursor, computed before the borrow below.
        let hovered = self.hovered_pixel(ui.ctx());
        ui.horizontal_centered(|ui| {
            ui.add_space(10.0);

            if let Some(image) = &self.image {
                let name = image
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.label(RichText::new(name).size(12.0));
                ui.label(
                    RichText::new(format_position(self.library.index + 1, self.library.len()))
                        .size(12.0)
                        .weak(),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!("{} × {}", image.width, image.height))
                        .size(12.0)
                        .weak(),
                );
                ui.label(
                    RichText::new(format_bytes(image.file_size))
                        .size(12.0)
                        .weak(),
                );
                ui.label(RichText::new(&image.format).size(12.0).weak());

                // Colour of the pixel under the cursor: the point of the pixel
                // grid is being able to read exact values off the image.
                if let Some((x, y)) = hovered {
                    if let Some([r, g, b, a]) = image.pixel_rgba(x, y) {
                        ui.separator();
                        ui.label(
                            RichText::new("■")
                                .size(12.0)
                                .color(Color32::from_rgb(r, g, b)),
                        );
                        let hex = if a == 255 {
                            format!("#{r:02X}{g:02X}{b:02X}")
                        } else {
                            format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
                        };
                        ui.label(RichText::new(hex).size(12.0).monospace().weak());
                        ui.label(RichText::new(format!("{x}, {y}")).size(12.0).weak());
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!("{}%", (self.view.zoom * 100.0).round()))
                            .size(12.0)
                            .weak(),
                    );
                    if self.view.mode == ZoomMode::Fit {
                        ui.label(RichText::new(t(lang, Text::Fit)).size(12.0).weak());
                    }
                });
            } else {
                // While the first photo of a folder is still decoding there is
                // no image yet, but saying "no image" would be misleading.
                let placeholder = match &self.status {
                    ViewStatus::Loading { path, .. } => Some((
                        file_name_of(path),
                        t(lang, Text::LoadingImage).to_owned(),
                    )),
                    ViewStatus::Error { path, .. } => {
                        Some((file_name_of(path), t(lang, Text::LoadFailed).to_owned()))
                    }
                    _ => None,
                };
                match placeholder {
                    Some((name, message)) => {
                        ui.label(RichText::new(name).size(12.0));
                        ui.label(RichText::new(message).size(12.0).weak());
                    }
                    None => {
                        ui.label(RichText::new(t(lang, Text::NoImage)).size(12.0).weak());
                    }
                }
            }
        });
    }

    fn filmstrip(&mut self, ui: &mut Ui) {
        let count = self.library.len();
        if count == 0 {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("—").weak());
            });
            return;
        }

        let step = THUMB_WIDTH + THUMB_GAP;
        let current = self.library.index;

        // Warm up the neighbourhood of the selection: the strip scrolls back to
        // it, so those are the thumbnails that are about to be on screen.
        for index in current.saturating_sub(THUMB_WINDOW)..(current + THUMB_WINDOW + 1).min(count) {
            let path = self.library.files[index].path.clone();
            if !self.thumb_cache.contains(&path) {
                self.thumbs.request(path);
            }
        }

        let want_scroll = self.scroll_strip;
        let mut clicked: Option<usize> = None;
        // Thumbnails that became visible by scrolling. They cannot be requested
        // inside the scroll area because that borrows the app, so they are
        // collected here and queued once the strip is laid out.
        let mut newly_visible: Vec<PathBuf> = Vec::new();

        // A horizontal scroll area ignores the vertical wheel by default, which
        // made the filmstrip impossible to scroll with a normal mouse. Turning
        // this on lets either wheel axis drive the strip.
        let previous_scroll_style = ui.style().always_scroll_the_only_direction;
        ui.style_mut().always_scroll_the_only_direction = true;

        egui::ScrollArea::horizontal()
            .auto_shrink([false, true])
            .show_viewport(ui, |ui, viewport| {
                let total_width = count as f32 * step;
                let (content, _) =
                    ui.allocate_exact_size(Vec2::new(total_width, THUMB_HEIGHT), Sense::hover());

                let first = ((viewport.min.x / step).floor().max(0.0) as usize).min(count);
                let last = (((viewport.max.x / step).ceil() as usize) + 1).min(count);

                for index in first..last {
                    let entry = &self.library.files[index];
                    let left = content.left() + index as f32 * step;
                    let thumb_rect = Rect::from_min_size(
                        egui::pos2(left, content.top()),
                        Vec2::new(THUMB_WIDTH, THUMB_HEIGHT),
                    );

                    let response =
                        ui.interact(thumb_rect, ui.scope_id().with(("thumb", index)), Sense::click());
                    if response.clicked() {
                        clicked = Some(index);
                    }

                    let selected = index == current;
                    let painter = ui.painter();
                    painter.rect_filled(
                        thumb_rect,
                        6.0,
                        if selected {
                            ui.visuals().selection.bg_fill
                        } else {
                            ui.visuals().faint_bg_color
                        },
                    );

                    match self.thumb_cache.get(&entry.path) {
                        Some(Some(texture)) => {
                            let size = texture.size_vec2();
                            let scale = ((THUMB_WIDTH - 8.0) / size.x).min((THUMB_HEIGHT - 8.0) / size.y);
                            let draw = Rect::from_center_size(thumb_rect.center(), size * scale);
                            painter.image(
                                texture.id(),
                                draw,
                                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        Some(None) => {
                            painter.text(
                                thumb_rect.center(),
                                Align2::CENTER_CENTER,
                                "!",
                                FontId::proportional(16.0),
                                ui.visuals().error_fg_color,
                            );
                        }
                        None => {
                            // Visible but not decoded yet: ask for it. The
                            // loader ignores duplicates, so repeating this
                            // every frame costs nothing.
                            newly_visible.push(entry.path.clone());
                            painter.text(
                                thumb_rect.center(),
                                Align2::CENTER_CENTER,
                                "…",
                                FontId::proportional(14.0),
                                ui.visuals().weak_text_color(),
                            );
                        }
                    }

                    if selected {
                        painter.rect_stroke(
                            thumb_rect,
                            6.0,
                            Stroke::new(2.0, ui.visuals().selection.stroke.color),
                            StrokeKind::Inside,
                        );
                        if want_scroll {
                            ui.scroll_to_rect(thumb_rect, Some(Align::Center));
                        }
                    } else if response.hovered() {
                        painter.rect_stroke(
                            thumb_rect,
                            6.0,
                            Stroke::new(1.0, ui.visuals().widgets.hovered.bg_stroke.color),
                            StrokeKind::Inside,
                        );
                    }
                }
            });

        ui.style_mut().always_scroll_the_only_direction = previous_scroll_style;
        self.scroll_strip = false;

        for path in newly_visible {
            self.thumbs.request(path);
        }

        if let Some(index) = clicked {
            if self.library.go_to(index) {
                self.start_loading_current();
            }
        }
    }

    fn canvas(&mut self, ui: &mut Ui) {
        let lang = self.lang;
        let dark = ui.visuals().dark_mode;
        let (rect, response) = ui.allocate_at_least(ui.available_size(), Sense::click_and_drag());
        self.last_canvas = rect;

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::canvas_color(self.settings.background, dark));

        if self.settings.background == BackgroundStyle::Checkerboard {
            let texture = self.checker_texture(ui.ctx(), dark);
            let uv = Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(rect.width() / 64.0, rect.height() / 64.0),
            );
            painter.image(texture.id(), rect, uv, Color32::WHITE);
        }

        let image_size = self.image.as_ref().map(|image| image.size());
        let Some(size) = image_size else {
            self.empty_state(ui, rect);
            return;
        };

        if self.pending_view_reset {
            self.view.reset(rect, size, self.settings.open_fitted);
            self.pending_view_reset = false;
        }
        self.view.update_fit(rect, size);
        self.view
            .interact(ui, &response, rect, size, self.settings.zoom_sensitivity);
        self.view
            .clamp(rect, ViewState::rotated_size(size, self.view.rotation));

        // Double click toggles between fit and 100%, like most viewers.
        if response.double_clicked() {
            if self.view.mode == ZoomMode::Fit {
                let anchor = response.interact_pointer_pos();
                self.view.zoom_to(1.0, anchor, rect, size);
            } else {
                self.fit_to_window();
            }
        }

        let hovered = self.hovered_pixel(ui.ctx());
        if let Some(image) = &self.image {
            self.view.paint(&painter, rect, image.texture.id(), size);

            // Pixel grid: only once one image pixel covers enough physical
            // screen pixels for a grid to be readable rather than a grey wash.
            let pixel_size = self.view.zoom * ui.ctx().pixels_per_point();
            if self.settings.pixel_grid && pixel_size >= PIXEL_GRID_MIN_SIZE {
                self.view.paint_pixel_grid(
                    &painter,
                    rect,
                    size,
                    Color32::from_rgba_unmultiplied(128, 128, 128, 110),
                );
            }

            // Outline the pixel the status bar is reporting, so it is obvious
            // which one the readout belongs to. Users who want an untouched
            // picture can switch this off in the settings.
            if self.settings.hover_highlight {
                if let Some((x, y)) = hovered {
                    let top_left = self.view.image_to_screen(x as f32, y as f32, rect, size);
                    let bottom_right =
                        self.view.image_to_screen(x as f32 + 1.0, y as f32 + 1.0, rect, size);
                    let outline = Rect::from_two_pos(top_left, bottom_right);
                    if outline.width() >= 3.0 {
                        painter.rect_stroke(
                            outline,
                            0.0,
                            Stroke::new(1.5, ui.visuals().selection.stroke.color),
                            StrokeKind::Outside,
                        );
                    }
                }
            }

            if self.settings.show_info {
                paint_info_overlay(
                    &painter,
                    rect,
                    image,
                    lang,
                    (self.library.index + 1, self.library.len()),
                    self.view.zoom,
                );
            }
        }

        // Right-click menu: reveal the file in Explorer or open its folder.
        let mut reveal = false;
        let mut open_parent = false;
        let mut show_toolbar = false;
        response.context_menu(|ui| {
            // A way back for anyone who hid the toolbar and forgot the
            // shortcut; it only appears while the toolbar is actually hidden.
            if !self.settings.show_toolbar && ui.button(t(lang, Text::ShowToolbar)).clicked() {
                show_toolbar = true;
                ui.close();
            }
            if ui.button(t(lang, Text::ShowInExplorer)).clicked() {
                reveal = true;
            }
            if ui.button(t(lang, Text::OpenContainingFolder)).clicked() {
                open_parent = true;
            }
        });
        if show_toolbar {
            self.settings.show_toolbar = true;
            self.mark_dirty();
        }
        if reveal {
            if let Some(image) = &self.image {
                platform::reveal_in_explorer(&image.path);
            }
        }
        if open_parent {
            if let Some(folder) = &self.library.folder {
                platform::open_folder(folder);
            }
        }

        response.on_hover_cursor(egui::CursorIcon::Grab);

        // Visual hint while a file is being dragged over the window.
        if ui.ctx().input(|input| !input.raw.hovered_files.is_empty()) {
            let overlay = rect;
            painter.rect_filled(overlay, 0.0, Color32::from_black_alpha(140));
            painter.text(
                overlay.center(),
                Align2::CENTER_CENTER,
                t(lang, Text::DropToOpen),
                FontId::proportional(22.0),
                Color32::WHITE,
            );
        }
    }

    fn empty_state(&mut self, ui: &mut Ui, rect: Rect) {
        let lang = self.lang;

        enum Kind {
            Empty,
            Loading(String, f32),
            Error(String, String),
        }

        let kind = match &self.status {
            ViewStatus::Loading { path, started, .. } => {
                Kind::Loading(file_name_of(path), started.elapsed().as_secs_f32())
            }
            ViewStatus::Error { path, message } => Kind::Error(file_name_of(path), message.clone()),
            _ => Kind::Empty,
        };

        let painter = ui.painter_at(rect);
        let center = rect.center();
        let mut y = center.y - 46.0;
        let mut open_folder = false;
        let mut next_image = false;

        match kind {
            Kind::Loading(name, elapsed) => {
                ui.put(
                    Rect::from_center_size(egui::pos2(center.x, y), Vec2::splat(34.0)),
                    egui::Spinner::new().size(28.0),
                );
                y += 42.0;
                painter.text(
                    egui::pos2(center.x, y),
                    Align2::CENTER_CENTER,
                    t(lang, Text::LoadingImage),
                    FontId::proportional(15.0),
                    ui.visuals().text_color(),
                );
                painter.text(
                    egui::pos2(center.x, y + 22.0),
                    Align2::CENTER_CENTER,
                    name,
                    FontId::proportional(12.0),
                    ui.visuals().weak_text_color(),
                );
                // Slow decodes (huge photos, network drives) get a live counter
                // so the window never looks frozen.
                if elapsed > 0.4 {
                    painter.text(
                        egui::pos2(center.x, y + 40.0),
                        Align2::CENTER_CENTER,
                        format!("{elapsed:.1} s"),
                        FontId::proportional(11.5),
                        ui.visuals().weak_text_color(),
                    );
                }
                ui.ctx().request_repaint();
            }
            Kind::Error(name, message) => {
                icons::paint(
                    &painter,
                    Rect::from_center_size(egui::pos2(center.x, y), Vec2::splat(44.0)),
                    Icon::Warning,
                    ui.visuals().warn_fg_color,
                    2.0,
                );
                y += 40.0;
                painter.text(
                    egui::pos2(center.x, y),
                    Align2::CENTER_CENTER,
                    t(lang, Text::LoadFailed),
                    FontId::proportional(15.0),
                    ui.visuals().text_color(),
                );
                painter.text(
                    egui::pos2(center.x, y + 20.0),
                    Align2::CENTER_CENTER,
                    truncate(&name, 60),
                    FontId::proportional(12.0),
                    ui.visuals().weak_text_color(),
                );
                painter.text(
                    egui::pos2(center.x, y + 38.0),
                    Align2::CENTER_CENTER,
                    truncate(&message, 70),
                    FontId::proportional(11.5),
                    ui.visuals().weak_text_color(),
                );
                let button = Rect::from_center_size(
                    egui::pos2(center.x, y + 78.0),
                    Vec2::new(150.0, 30.0),
                );
                if ui.put(button, egui::Button::new(t(lang, Text::Next))).clicked() {
                    next_image = true;
                }
            }
            Kind::Empty => {
                icons::paint(
                    &painter,
                    Rect::from_center_size(egui::pos2(center.x, y), Vec2::splat(52.0)),
                    Icon::Image,
                    ui.visuals().weak_text_color(),
                    1.8,
                );
                y += 48.0;
                let message = if self.library.folder.is_some() {
                    t(lang, Text::NoImagesInFolder)
                } else {
                    t(lang, Text::DropHint)
                };
                painter.text(
                    egui::pos2(center.x, y),
                    Align2::CENTER_CENTER,
                    message,
                    FontId::proportional(16.0),
                    ui.visuals().text_color(),
                );
                painter.text(
                    egui::pos2(center.x, y + 22.0),
                    Align2::CENTER_CENTER,
                    t(lang, Text::OrOpenFolder),
                    FontId::proportional(12.5),
                    ui.visuals().weak_text_color(),
                );
                let label = if self.library.folder.is_some() {
                    t(lang, Text::ChooseAnotherFolder)
                } else {
                    t(lang, Text::OpenFolder)
                };
                let button = Rect::from_center_size(
                    egui::pos2(center.x, y + 62.0),
                    Vec2::new(170.0, 32.0),
                );
                if ui.put(button, egui::Button::new(label)).clicked() {
                    open_folder = true;
                }
            }
        }

        if open_folder {
            self.pending_actions.push(Action::OpenFolder);
        }
        if next_image {
            self.navigate(1);
        }
    }

    fn checker_texture(&mut self, ctx: &egui::Context, dark: bool) -> egui::TextureHandle {
        if let Some((cached_dark, texture)) = &self.checker {
            if *cached_dark == dark {
                return texture.clone();
            }
        }
        let texture = theme::checker_texture(ctx, dark);
        self.checker = Some((dark, texture.clone()));
        texture
    }

    // ---------------------------------------------------------------------
    // Settings window
    // ---------------------------------------------------------------------

    /// The settings window.
    ///
    /// This opens a real window of its own: `show_viewport_immediate` asks the
    /// backend for a second OS window, which eframe provides. When the backend
    /// cannot create windows (the offscreen test harness, or a restricted
    /// platform) egui falls back to an embedded in-app window, and the very
    /// same code renders there, so there is only one layout to maintain.
    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        let lang = self.lang;
        let mut actions: Vec<Action> = Vec::new();
        let mut theme_changed = false;
        let mut language_changed = false;
        let mut sort_changed = false;
        let mut close = false;

        // Place the window over the middle of the main window the first time it
        // opens. The value is kept afterwards so that the builder — which is
        // reapplied every frame — cannot drag the window back while the user is
        // moving it.
        let settings_position = *self.settings_position.get_or_insert_with(|| {
            ctx.input(|input| input.viewport().outer_rect)
                .map(|parent| {
                    parent.center() - egui::vec2(SETTINGS_WIDTH, SETTINGS_HEIGHT) / 2.0
                })
                .unwrap_or_else(|| centered_pos(ctx, SETTINGS_WIDTH, SETTINGS_HEIGHT))
        });

        let builder = egui::ViewportBuilder::default()
            .with_title(t(lang, Text::Settings))
            .with_inner_size([SETTINGS_WIDTH, SETTINGS_HEIGHT])
            .with_min_inner_size([520.0, 420.0])
            .with_resizable(true)
            // A second window is a second taskbar button, so it needs the icon.
            .with_icon(icons::app_icon())
            .with_position(settings_position);

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("lumen.settings"),
            builder,
            |ui, _class| {
                // The window's own close button, or Escape while it has focus.
                // Keys arrive in the viewport that has focus, so this has to be
                // handled here rather than in `handle_keys`.
                let close_requested = ui.ctx().input(|input| input.viewport().close_requested())
                    || ui.input(|input| input.key_pressed(egui::Key::Escape));
                if close_requested {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    close = true;
                }

                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(egui::Margin::symmetric(14, 12)),
                    )
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                        // ----- Appearance -----
                        widgets::section(ui, t(lang, Text::Appearance));

                        widgets::setting_row(ui, t(lang, Text::Theme), "", |ui| {
                            let options = [
                                (ThemePref::System, t(lang, Text::ThemeSystem)),
                                (ThemePref::Light, t(lang, Text::ThemeLight)),
                                (ThemePref::Dark, t(lang, Text::ThemeDark)),
                            ];
                            if widgets::segmented(ui, &mut self.settings.theme, &options) {
                                theme_changed = true;
                            }
                        });

                        widgets::setting_row(ui, t(lang, Text::Background), "", |ui| {
                            let options = [
                                (BackgroundStyle::Checkerboard, t(lang, Text::BgCheckerboard)),
                                (BackgroundStyle::Theme, t(lang, Text::BgTheme)),
                                (BackgroundStyle::Dark, t(lang, Text::BgDark)),
                                (BackgroundStyle::Light, t(lang, Text::BgLight)),
                            ];
                            if widgets::dropdown(ui, "lumen.background", &mut self.settings.background, &options) {
                                self.mark_dirty();
                            }
                        });

                        if widgets::toggle(
                            ui,
                            t(lang, Text::ShowToolbar),
                            t(lang, Text::ShowToolbarHint),
                            &mut self.settings.show_toolbar,
                        ) {
                            self.mark_dirty();
                        }
                        if widgets::toggle(
                            ui,
                            t(lang, Text::ShowFilmstrip),
                            t(lang, Text::ShowFilmstripHint),
                            &mut self.settings.show_filmstrip,
                        ) {
                            self.mark_dirty();
                        }
                        if widgets::toggle(
                            ui,
                            t(lang, Text::ShowInfo),
                            t(lang, Text::ShowInfoHint),
                            &mut self.settings.show_info,
                        ) {
                            self.mark_dirty();
                        }
                        if widgets::toggle(
                            ui,
                            t(lang, Text::ShowStatusBar),
                            t(lang, Text::ShowStatusBarHint),
                            &mut self.settings.show_status_bar,
                        ) {
                            self.mark_dirty();
                        }

                        // ----- Viewing -----
                        widgets::section(ui, t(lang, Text::Viewing));

                        if widgets::toggle(
                            ui,
                            t(lang, Text::OpenFitted),
                            t(lang, Text::OpenFittedHint),
                            &mut self.settings.open_fitted,
                        ) {
                            self.mark_dirty();
                        }

                        widgets::setting_row(
                            ui,
                            t(lang, Text::ZoomSensitivity),
                            t(lang, Text::ZoomSensitivityHint),
                            |ui| {
                                ui.add(
                                    egui::Slider::new(&mut self.settings.zoom_sensitivity, 0.2..=3.0)
                                        .fixed_decimals(1)
                                        .suffix("×"),
                                );
                            },
                        );

                        if widgets::toggle(
                            ui,
                            t(lang, Text::PixelGrid),
                            t(lang, Text::PixelGridHint),
                            &mut self.settings.pixel_grid,
                        ) {
                            self.mark_dirty();
                        }

                        if widgets::toggle(
                            ui,
                            t(lang, Text::HoverHighlight),
                            t(lang, Text::HoverHighlightHint),
                            &mut self.settings.hover_highlight,
                        ) {
                            self.mark_dirty();
                        }

                        widgets::setting_row(ui, t(lang, Text::SlideshowInterval), "", |ui| {
                            ui.add(
                                egui::Slider::new(&mut self.settings.slideshow_interval, 1.0..=30.0)
                                    .fixed_decimals(1)
                                    .suffix(" s"),
                            );
                        });

                        if widgets::toggle(
                            ui,
                            t(lang, Text::LoopSlideshow),
                            "",
                            &mut self.settings.slideshow_loop,
                        ) {
                            self.mark_dirty();
                        }

                        // ----- Library -----
                        widgets::section(ui, t(lang, Text::Library));

                        widgets::setting_row(ui, t(lang, Text::SortOrder), "", |ui| {
                            let options = [
                                (SortOrder::NameAscending, t(lang, Text::SortNameAsc)),
                                (SortOrder::NameDescending, t(lang, Text::SortNameDesc)),
                                (SortOrder::NewestFirst, t(lang, Text::SortNewest)),
                                (SortOrder::LargestFirst, t(lang, Text::SortLargest)),
                            ];
                            if widgets::dropdown(ui, "lumen.sort", &mut self.settings.sort_order, &options) {
                                sort_changed = true;
                            }
                        });

                        if widgets::toggle(
                            ui,
                            t(lang, Text::RememberSession),
                            t(lang, Text::RememberSessionHint),
                            &mut self.settings.remember_session,
                        ) {
                            self.mark_dirty();
                        }

                        if ui.button(t(lang, Text::ForgetSession)).clicked() {
                            self.settings.last_folder = None;
                            self.settings.last_file = None;
                            self.mark_dirty();
                        }

                        // ----- Language -----
                        widgets::section(ui, t(lang, Text::Language));

                        widgets::setting_row(ui, t(lang, Text::Language), t(lang, Text::LanguageHint), |ui| {
                            let options = [
                                (LanguagePref::System, t(lang, Text::LangSystem)),
                                (LanguagePref::English, t(lang, Text::LangEnglish)),
                                (LanguagePref::Chinese, t(lang, Text::LangChinese)),
                            ];
                            if widgets::segmented(ui, &mut self.settings.language, &options) {
                                language_changed = true;
                            }
                        });

                        // ----- Default photo viewer -----
                        widgets::section(ui, t(lang, Text::DefaultViewer));
                        ui.label(
                            RichText::new(t(lang, Text::DefaultViewerHint))
                                .small()
                                .weak(),
                        );
                        ui.add_space(6.0);

                        if platform::is_supported() {
                            ui.horizontal(|ui| {
                                if self.default_app_registered {
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                                    icons::paint(
                                        ui.painter(),
                                        rect,
                                        Icon::Check,
                                        ui.visuals().hyperlink_color,
                                        1.6,
                                    );
                                }
                                ui.label(
                                    RichText::new(if self.default_app_registered {
                                        t(lang, Text::Registered)
                                    } else {
                                        t(lang, Text::NotRegistered)
                                    })
                                    .small(),
                                );
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui.button(t(lang, Text::RegisterDefault)).clicked() {
                                    actions.push(Action::RegisterDefault);
                                }
                                if self.default_app_registered
                                    && ui.button(t(lang, Text::UnregisterDefault)).clicked()
                                {
                                    actions.push(Action::UnregisterDefault);
                                }
                            });
                        } else {
                            ui.label(RichText::new(t(lang, Text::WindowsOnly)).small().weak());
                        }

                        // ----- Settings file -----
                        widgets::section(ui, t(lang, Text::ConfigFile));
                        ui.label(RichText::new(t(lang, Text::ConfigFileHint)).small().weak());
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(crate::settings::settings_path().display().to_string())
                                .monospace()
                                .size(11.5)
                                .weak(),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button(t(lang, Text::ExportSettings)).clicked() {
                                actions.push(Action::ExportSettings);
                            }
                            if ui.button(t(lang, Text::ImportSettings)).clicked() {
                                actions.push(Action::ImportSettings);
                            }
                            if ui.button(t(lang, Text::ResetDefaults)).clicked() {
                                actions.push(Action::ResetSettings);
                            }
                            if ui.button(t(lang, Text::OpenFolderAction)).clicked() {
                                actions.push(Action::OpenSettingsFolder);
                            }
                        });

                        // ----- About -----
                        widgets::section(ui, t(lang, Text::About));
                        ui.label(t(lang, Text::AboutBlurb));
                        ui.add_space(4.0);

                        egui::Grid::new("lumen.about")
                            .num_columns(2)
                            .spacing([20.0, 4.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new(t(lang, Text::Version)).small().weak());
                                ui.label(RichText::new(env!("CARGO_PKG_VERSION")).small());
                                ui.end_row();

                                ui.label(RichText::new(t(lang, Text::StartupTime)).small().weak());
                                ui.label(
                                    RichText::new(match self.startup_ms {
                                        Some(ms) => format!("{ms:.0} ms"),
                                        None => "—".to_owned(),
                                    })
                                    .small(),
                                );
                                ui.end_row();

                                ui.label(RichText::new(t(lang, Text::Renderer)).small().weak());
                                ui.label(RichText::new(crate::RENDERER_NAME).small());
                                ui.end_row();

                                // Clickable, because the whole point of the
                                // link is that nobody has to type it.
                                ui.label(RichText::new(t(lang, Text::Repository)).small().weak());
                                ui.hyperlink_to(
                                    RichText::new("github.com/ChathamHung/Lumen").small(),
                                    crate::REPOSITORY_URL,
                                );
                                ui.end_row();

                                ui.label(RichText::new(t(lang, Text::SupportedFormats)).small().weak());
                                ui.label(
                                    RichText::new(
                                        library::SUPPORTED_EXTENSIONS
                                            .iter()
                                            .map(|extension| extension.to_ascii_uppercase())
                                            .collect::<Vec<_>>()
                                            .join(" · "),
                                    )
                                    .small(),
                                );
                                ui.end_row();
                            });

                        ui.add_space(6.0);
                        if ui.button(t(lang, Text::Shortcuts)).clicked() {
                            actions.push(Action::OpenShortcuts);
                        }
                        ui.add_space(12.0);
                            });
                    });
            },
        );

        if theme_changed {
            theme::apply(ctx, self.settings.theme);
            self.mark_dirty();
        }
        if language_changed {
            self.lang = Lang::resolve(self.settings.language);
            self.mark_dirty();
        }
        if sort_changed {
            self.library.set_sort(self.settings.sort_order);
            self.mark_dirty();
        }

        self.settings_open = !close;
        self.pending_actions.extend(actions);
    }

    /// The keyboard shortcut list, in a window of its own just like settings.
    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        if !self.shortcuts_open {
            return;
        }
        let lang = self.lang;
        let mut close = false;

        // Placed once, like the settings window, so the builder cannot drag it
        // back while the user moves it.
        let position = *self.shortcuts_position.get_or_insert_with(|| {
            ctx.input(|input| input.viewport().outer_rect)
                .map(|parent| {
                    parent.center() - egui::vec2(SHORTCUTS_WIDTH, SHORTCUTS_HEIGHT) / 2.0
                })
                .unwrap_or_else(|| centered_pos(ctx, SHORTCUTS_WIDTH, SHORTCUTS_HEIGHT))
        });

        let builder = egui::ViewportBuilder::default()
            .with_title(t(lang, Text::ShortcutsTitle))
            .with_inner_size([SHORTCUTS_WIDTH, SHORTCUTS_HEIGHT])
            .with_resizable(false)
            .with_icon(icons::app_icon())
            .with_position(position);

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("lumen.shortcuts"),
            builder,
            |ui, _class| {
                let close_requested = ui.ctx().input(|input| input.viewport().close_requested())
                    || ui.input(|input| input.key_pressed(egui::Key::Escape));
                if close_requested {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    close = true;
                }

                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(egui::Margin::symmetric(16, 14)),
                    )
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let rows = [
                                    (t(lang, Text::ScPrevNext), "←  →"),
                                    (t(lang, Text::ScFirstLast), "Home  End"),
                                    (t(lang, Text::ScZoom), "+  -"),
                                    (t(lang, Text::ScFit), "0"),
                                    (t(lang, Text::ScActual), "1"),
                                    (t(lang, Text::ScRotate), "R  Shift+R"),
                                    (t(lang, Text::ScFlip), "H  V"),
                                    (t(lang, Text::ScResetView), "Ctrl+0"),
                                    (t(lang, Text::ScSlideshow), "Space"),
                                    (t(lang, Text::ScInfo), "I"),
                                    (t(lang, Text::ScFilmstrip), "T"),
                                    (t(lang, Text::ScToolbar), "Ctrl+T"),
                                    (t(lang, Text::ScPixelGrid), "G"),
                                    (t(lang, Text::ScFullscreen), "F11"),
                                    (t(lang, Text::ScSettings), "Ctrl+,"),
                                    (t(lang, Text::ScShortcuts), "F1"),
                                    (t(lang, Text::ScOpen), "Ctrl+O  Ctrl+Shift+O"),
                                    (t(lang, Text::ScClose), "Esc"),
                                ];

                                egui::Grid::new("lumen.shortcuts")
                                    .num_columns(2)
                                    .spacing([24.0, 6.0])
                                    .show(ui, |ui| {
                                        for (label, keys) in rows {
                                            ui.label(label);
                                            ui.label(RichText::new(keys).monospace().weak());
                                            ui.end_row();
                                        }
                                    });
                            });
                    });
            },
        );

        self.shortcuts_open = !close;
    }

    // ---------------------------------------------------------------------
    // Toasts and persistence
    // ---------------------------------------------------------------------

    fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            created: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn toasts_ui(&mut self, ctx: &egui::Context) {
        self.toasts
            .retain(|toast| toast.created.elapsed().as_secs_f32() < TOAST_SECONDS);
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));

        egui::Area::new(egui::Id::unique("lumen.toasts"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -70.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                for toast in &self.toasts {
                    egui::Frame::NONE
                        .fill(Color32::from_black_alpha(215))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(&toast.text)
                                    .color(Color32::from_gray(240))
                                    .size(13.0),
                            );
                        });
                }
            });
    }

    fn mark_dirty(&mut self) {
        self.settings_dirty = true;
        self.dirty_since = Some(Instant::now());
    }

    /// Opens the shortcut list, re-centring it over the main window.
    fn open_shortcuts(&mut self) {
        self.shortcuts_open = true;
        self.shortcuts_position = None;
    }

    /// Opens the settings window, re-centring it over the main window.
    fn open_settings(&mut self) {
        self.settings_open = true;
        // Forgetting the old position makes the window appear over the middle
        // of the main window again instead of wherever it was last dragged.
        self.settings_position = None;
    }

    /// Saves shortly after the last change, so a crash never loses much.
    fn auto_save(&mut self, ctx: &egui::Context) {
        if !self.settings_dirty {
            return;
        }
        let Some(since) = self.dirty_since else { return };

        if since.elapsed() >= AUTOSAVE_DELAY {
            self.save_now();
        } else {
            ctx.request_repaint_after(AUTOSAVE_DELAY - since.elapsed());
        }
    }

    fn save_now(&mut self) {
        self.settings_dirty = false;
        self.dirty_since = None;
        if let Err(error) = self.settings.save() {
            diag::log(&format!("settings: could not be saved: {error}"));
        }
    }

    // ---------------------------------------------------------------------
    // Deferred actions (blocking dialogs live here)
    // ---------------------------------------------------------------------

    fn run_actions(&mut self, ctx: &egui::Context) {
        for action in std::mem::take(&mut self.pending_actions) {
            match action {
                Action::OpenFolder => {
                    let start = self.start_folder();
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title(t(self.lang, Text::OpenFolder))
                        .set_directory(start)
                        .pick_folder()
                    {
                        self.open_folder(&folder, None);
                    }
                }
                Action::OpenFile => {
                    let start = self.start_folder();
                    if let Some(file) = rfd::FileDialog::new()
                        .set_title(t(self.lang, Text::OpenFile))
                        .set_directory(start)
                        .add_filter("Images", library::SUPPORTED_EXTENSIONS)
                        .pick_file()
                    {
                        self.open_path(&file);
                    }
                }
                Action::ExportSettings => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title(t(self.lang, Text::ExportSettings))
                        .add_filter("JSON", &["json"])
                        .set_file_name("lumen-settings.json")
                        .save_file()
                    {
                        match self.settings.export_to(&path) {
                            Ok(()) => self.toast(t(self.lang, Text::SettingsExported)),
                            Err(error) => self.toast(format!(
                                "{}: {error}",
                                t(self.lang, Text::ExportFailed)
                            )),
                        }
                    }
                }
                Action::ImportSettings => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title(t(self.lang, Text::ImportSettings))
                        .add_filter("JSON", &["json"])
                        .pick_file()
                    {
                        match Settings::import_from(&path) {
                            Ok(settings) => {
                                self.settings = settings;
                                self.lang = Lang::resolve(self.settings.language);
                                theme::apply(ctx, self.settings.theme);
                                self.library.set_sort(self.settings.sort_order);
                                self.save_now();
                                self.toast(t(self.lang, Text::SettingsImported));
                            }
                            Err(error) => self.toast(format!(
                                "{}: {error}",
                                t(self.lang, Text::ImportFailed)
                            )),
                        }
                    }
                }
                Action::ResetSettings => {
                    self.settings = Settings::default();
                    self.lang = Lang::resolve(self.settings.language);
                    theme::apply(ctx, self.settings.theme);
                    self.save_now();
                    self.toast(t(self.lang, Text::SettingsReset));
                }
                Action::RegisterDefault => match platform::register() {
                    Ok(()) => {
                        self.default_app_registered = platform::is_registered();
                        if let Err(error) = platform::open_default_apps_settings() {
                            diag::log(&format!("could not open Windows settings: {error}"));
                        }
                        self.toast(t(self.lang, Text::RegistrationDone));
                    }
                    Err(error) => self.toast(format!(
                        "{}: {error}",
                        t(self.lang, Text::RegistrationFailed)
                    )),
                },
                Action::UnregisterDefault => match platform::unregister() {
                    Ok(()) => {
                        self.default_app_registered = platform::is_registered();
                        self.toast(t(self.lang, Text::RegistrationRemoved));
                    }
                    Err(error) => self.toast(format!(
                        "{}: {error}",
                        t(self.lang, Text::RegistrationFailed)
                    )),
                },
                Action::OpenSettingsFolder => platform::open_folder(&crate::settings::config_dir()),
                Action::OpenShortcuts => self.open_shortcuts(),
            }
        }
    }

    fn start_folder(&self) -> PathBuf {
        self.library
            .folder
            .clone()
            .or_else(|| self.settings.last_folder.clone())
            .or_else(dirs::picture_dir)
            .unwrap_or_default()
    }
}

impl eframe::App for LumenApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Measure the time from process start to the first painted frame.
        if self.startup_ms.is_none() {
            let ms = self.started.elapsed().as_secs_f32() * 1000.0;
            self.startup_ms = Some(ms);
            diag::log(&format!("startup: first frame after {ms:.1} ms"));
            if std::env::var_os("LUMEN_TRACE_STARTUP").is_some() {
                let _ = std::fs::write(
                    crate::settings::config_dir().join("startup.txt"),
                    format!("{ms:.1}\n"),
                );
            }
        }

        self.poll_workers(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_keys(&ctx);
        self.tick_animation(&ctx);
        self.tick_slideshow(&ctx);
        self.update_title(&ctx);
        self.track_window(&ctx);

        let panel_fill = ui.visuals().panel_fill;

        // The toolbar can be hidden to give the photo the whole window; the
        // shortcut, the settings window and the canvas context menu all bring
        // it back, so hiding it can never leave the user stuck.
        if self.settings.show_toolbar {
            egui::Panel::top("lumen.toolbar")
                .frame(
                    egui::Frame::NONE
                        .fill(panel_fill)
                        .inner_margin(egui::Margin::symmetric(0, 8)),
                )
                .exact_size(TOOLBAR_HEIGHT)
                .show(ui, |ui| self.toolbar(ui));
        }
        if self.settings.show_status_bar {
            egui::Panel::bottom("lumen.status")
                .frame(
                    egui::Frame::NONE
                        .fill(panel_fill)
                        .inner_margin(egui::Margin::symmetric(0, 4)),
                )
                .exact_size(STATUS_HEIGHT)
                .show(ui, |ui| self.status_bar(ui));
        }

        if self.settings.show_filmstrip {
            egui::Panel::bottom("lumen.filmstrip")
                .frame(
                    egui::Frame::NONE
                        .fill(panel_fill)
                        .inner_margin(egui::Margin::symmetric(0, 8)),
                )
                .exact_size(FILMSTRIP_HEIGHT)
                .show(ui, |ui| self.filmstrip(ui));
        }

        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));

        self.settings_window(&ctx);
        self.shortcuts_window(&ctx);
        self.toasts_ui(&ctx);
        self.run_actions(&ctx);
        self.auto_save(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Best effort: the auto-save usually got here first.
        let _ = self.settings.save();
        // The window may have moved since the last debounced write.
        if self.window != self.window_saved {
            self.window.save();
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// Semi-transparent information panel drawn over the photo.
fn paint_info_overlay(
    painter: &egui::Painter,
    canvas: Rect,
    image: &LoadedImage,
    lang: Lang,
    position: (usize, usize),
    zoom: f32,
) {
    let title = file_name_of(&image.path);

    // Show the original resolution when the file was too large for the GPU and
    // had to be downscaled on load.
    let dimensions = if image.source_width != image.width || image.source_height != image.height {
        format!(
            "{} × {}  (from {} × {})",
            image.width, image.height, image.source_width, image.source_height
        )
    } else {
        format!("{} × {}", image.width, image.height)
    };

    let mut rows = vec![
        (t(lang, Text::Dimensions), dimensions),
        (t(lang, Text::Zoom), format!("{:.0}%", zoom * 100.0)),
        (t(lang, Text::FileSize), format_bytes(image.file_size)),
        (t(lang, Text::Format), image.format.clone()),
        (
            t(lang, Text::Modified),
            image
                .modified
                .map(format_time)
                .unwrap_or_else(|| t(lang, Text::Unknown).to_owned()),
        ),
        (t(lang, Text::Position), format_position(position.0, position.1)),
        (t(lang, Text::LoadedIn), format!("{:.0} ms", image.decode_ms)),
        (
            t(lang, Text::Folder),
            image
                .path
                .parent()
                .map(|parent| parent.display().to_string())
                .unwrap_or_default(),
        ),
    ];

    // Animated files also report how many frames they have and how long the
    // loop lasts.
    if image.is_animated() {
        let total: u32 = image.frames.iter().map(|frame| frame.delay_ms).sum();
        rows.push((
            t(lang, Text::Frames),
            format!("{}  ({:.1} s)", image.frame_count(), total as f32 / 1000.0),
        ));
    }

    let line_height = 18.0;
    let padding = 10.0;
    // Left edge of the value column, relative to the panel's padding.
    const VALUE_COLUMN: f32 = 78.0;

    // Size the panel to its contents so it does not sit there as a large empty
    // box when the values are short.
    let label_font = FontId::proportional(11.0);
    let value_font = FontId::proportional(11.5);
    let mut content_width = painter
        .layout_no_wrap(
            truncate(&title, 42),
            FontId::proportional(12.5),
            Color32::WHITE,
        )
        .size()
        .x;
    for (label, value) in &rows {
        let label_width = painter
            .layout_no_wrap((*label).to_owned(), label_font.clone(), Color32::WHITE)
            .size()
            .x;
        let value_width = painter
            .layout_no_wrap(value.clone(), value_font.clone(), Color32::WHITE)
            .size()
            .x;
        content_width = content_width.max(label_width).max(VALUE_COLUMN + value_width);
    }

    let width = (content_width + padding * 2.0).clamp(220.0, (canvas.width() - 24.0).min(460.0));
    let height = padding * 2.0 + line_height * (rows.len() as f32 + 1.0);
    let rect = Rect::from_min_size(
        canvas.left_top() + Vec2::new(12.0, 12.0),
        Vec2::new(width, height),
    );

    painter.rect_filled(rect, 8.0, Color32::from_black_alpha(170));
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, Color32::from_white_alpha(28)),
        StrokeKind::Inside,
    );

    let mut y = rect.top() + padding + line_height * 0.5;
    painter.text(
        egui::pos2(rect.left() + padding, y),
        Align2::LEFT_CENTER,
        truncate(&title, 42),
        FontId::proportional(12.5),
        Color32::from_gray(245),
    );

    for (label, value) in rows {
        y += line_height;
        painter.text(
            egui::pos2(rect.left() + padding, y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(11.0),
            Color32::from_gray(150),
        );
        painter.text(
            egui::pos2(rect.left() + padding + VALUE_COLUMN, y),
            Align2::LEFT_CENTER,
            truncate(&value, 44),
            FontId::proportional(11.5),
            Color32::from_gray(225),
        );
    }
}

/// Top-left corner that centres a window of the given size over the main one.
fn centered_pos(ctx: &egui::Context, width: f32, height: f32) -> Pos2 {
    let screen = ctx.content_rect();
    let x = screen.center().x - width * 0.5;
    let y = screen.center().y - height * 0.5;
    Pos2::new(x.max(screen.left() + 16.0), y.max(screen.top() + 16.0))
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn format_position(index: usize, total: usize) -> String {
    format!("{index} / {total}")
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Formats a timestamp in the user's local time.
fn format_time(time: SystemTime) -> String {
    let seconds = time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let seconds = seconds + platform::local_utc_offset_minutes() * 60;
    let days = seconds.div_euclid(86_400);
    let remainder = seconds.rem_euclid(86_400);
    let (year, month, day) = diag::civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        remainder / 3600,
        (remainder % 3600) / 60
    )
}

/// Shortens a string to `max` characters, adding an ellipsis.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut result: String = text.chars().take(max.saturating_sub(1)).collect();
        result.push('…');
        result
    }
}
