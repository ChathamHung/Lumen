//! Zoom / pan / rotation state and the canvas painter.
//!
//! The view is described by a zoom factor (image pixels -> screen points), a
//! pan offset from the centre of the canvas and a rotation that is always a
//! multiple of 90 degrees. Rotation is done by permuting the texture
//! coordinates of a quad, so it is free and never touches the decoded pixels.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, TextureId, Ui, Vec2};

/// Smallest and largest zoom factor the user can reach.
pub const MIN_ZOOM: f32 = 0.02;
pub const MAX_ZOOM: f32 = 40.0;

/// Space kept between the photo and the edge of the window when fitting.
const FIT_MARGIN: f32 = 24.0;

/// Whether the image follows the window size or keeps an explicit zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomMode {
    /// Always scaled so the whole image is visible.
    Fit,
    /// Explicit zoom chosen by the user.
    Free,
}

/// Everything about how the current image is displayed.
#[derive(Debug, Clone)]
pub struct ViewState {
    pub zoom: f32,
    pub offset: Vec2,
    pub mode: ZoomMode,
    /// 0, 90, 180 or 270 degrees clockwise.
    pub rotation: i32,
    /// Mirrored left-to-right.
    pub flip_x: bool,
    /// Mirrored top-to-bottom.
    pub flip_y: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            offset: Vec2::ZERO,
            mode: ZoomMode::Fit,
            rotation: 0,
            flip_x: false,
            flip_y: false,
        }
    }
}

impl ViewState {
    /// Size of the image on screen after rotation, in image pixels.
    pub fn rotated_size(image: Vec2, rotation: i32) -> Vec2 {
        if rotation.rem_euclid(180) == 0 {
            image
        } else {
            egui::vec2(image.y, image.x)
        }
    }

    /// Zoom factor that makes the whole image visible.
    pub fn fit_zoom(view: Rect, image: Vec2) -> f32 {
        if image.x <= 0.0 || image.y <= 0.0 {
            return 1.0;
        }
        let available = (view.size() - Vec2::splat(FIT_MARGIN)).max(Vec2::splat(32.0));
        (available.x / image.x)
            .min(available.y / image.y)
            .clamp(MIN_ZOOM, MAX_ZOOM)
    }

    /// Resets pan and zoom, either fitted or at 100%.
    pub fn reset(&mut self, view: Rect, image: Vec2, fitted: bool) {
        self.offset = Vec2::ZERO;
        self.mode = if fitted { ZoomMode::Fit } else { ZoomMode::Free };
        self.zoom = if fitted {
            Self::fit_zoom(view, Self::rotated_size(image, self.rotation))
        } else {
            1.0
        };
    }

    /// Recomputes the zoom while in fit mode (called when the window resizes).
    pub fn update_fit(&mut self, view: Rect, image: Vec2) {
        if self.mode == ZoomMode::Fit {
            self.zoom = Self::fit_zoom(view, Self::rotated_size(image, self.rotation));
            self.offset = Vec2::ZERO;
        }
    }

    /// Sets an absolute zoom, keeping the image point under `anchor` in place.
    pub fn zoom_to(&mut self, zoom: f32, anchor: Option<Pos2>, view: Rect, image: Vec2) {
        let rotated = Self::rotated_size(image, self.rotation);
        let new_zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if (new_zoom - self.zoom).abs() < f32::EPSILON {
            return;
        }

        let center = view.center();
        let anchor = anchor.unwrap_or(center);
        // Image point currently under the anchor, in image pixels.
        let before = (anchor - center - self.offset) / self.zoom;
        self.zoom = new_zoom;
        self.offset = anchor - center - before * new_zoom;
        self.mode = ZoomMode::Free;
        self.clamp(view, rotated);
    }

    /// Multiplies the zoom by `factor`.
    pub fn zoom_by(&mut self, factor: f32, anchor: Option<Pos2>, view: Rect, image: Vec2) {
        self.zoom_to(self.zoom * factor, anchor, view, image);
    }

    /// Rotates by `degrees` (normally +/-90) and re-centres the image.
    pub fn rotate(&mut self, degrees: i32, view: Rect, image: Vec2) {
        self.rotation = (self.rotation + degrees).rem_euclid(360);
        self.offset = Vec2::ZERO;
        self.clamp(view, Self::rotated_size(image, self.rotation));
    }

    /// Mirrors the image left-to-right.
    pub fn flip_horizontal(&mut self) {
        self.flip_x = !self.flip_x;
    }

    /// Mirrors the image top-to-bottom.
    pub fn flip_vertical(&mut self) {
        self.flip_y = !self.flip_y;
    }

    /// Drops rotation and mirroring, leaving zoom and pan untouched.
    pub fn reset_orientation(&mut self) {
        self.rotation = 0;
        self.flip_x = false;
        self.flip_y = false;
    }

    /// Maps an image pixel to its position inside the displayed photo, in image
    /// pixels measured from the top-left corner of the display rectangle.
    ///
    /// Mirroring is applied first, then rotation, which is exactly the order in
    /// which the two operations read: "turn the photo, then look at it in a
    /// mirror" would be the other way round, but users expect the buttons to
    /// act on what they currently see.
    fn image_to_display(&self, px: f32, py: f32, image: Vec2) -> (f32, f32) {
        let fx = if self.flip_x { image.x - px } else { px };
        let fy = if self.flip_y { image.y - py } else { py };
        match self.rotation.rem_euclid(360) {
            90 => (image.y - fy, fx),
            180 => (image.x - fx, image.y - fy),
            270 => (fy, image.x - fx),
            _ => (fx, fy),
        }
    }

    /// Inverse of [`Self::image_to_display`].
    fn display_to_image(&self, dx: f32, dy: f32, image: Vec2) -> (f32, f32) {
        let (fx, fy) = match self.rotation.rem_euclid(360) {
            90 => (dy, image.y - dx),
            180 => (image.x - dx, image.y - dy),
            270 => (image.x - dy, dx),
            _ => (dx, dy),
        };
        let px = if self.flip_x { image.x - fx } else { fx };
        let py = if self.flip_y { image.y - fy } else { fy };
        (px, py)
    }

    /// Pans by a screen-space delta.
    pub fn pan(&mut self, delta: Vec2, view: Rect, image: Vec2) {
        self.offset += delta;
        self.mode = ZoomMode::Free;
        self.clamp(view, Self::rotated_size(image, self.rotation));
    }

    /// Keeps the photo inside the canvas when it is larger than the view, and
    /// centred when it is smaller.
    pub fn clamp(&mut self, view: Rect, rotated_image: Vec2) {
        let display = rotated_image * self.zoom;
        let max_x = ((display.x - view.width()) * 0.5).max(0.0);
        let max_y = ((display.y - view.height()) * 0.5).max(0.0);
        self.offset.x = self.offset.x.clamp(-max_x, max_x);
        self.offset.y = self.offset.y.clamp(-max_y, max_y);
    }

    /// Handles drag-to-pan and wheel/pinch zoom for the canvas.
    /// Returns `true` when the view changed.
    pub fn interact(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        view: Rect,
        image: Vec2,
        sensitivity: f32,
    ) -> bool {
        let mut changed = false;

        if response.dragged_by(egui::PointerButton::Primary) {
            let delta = response.drag_delta();
            if delta != Vec2::ZERO {
                self.pan(delta, view, image);
                changed = true;
            }
        }

        if response.hovered() {
            let (pinch, scroll, ctrl) = ui.input(|input| {
                (
                    input.zoom_delta(),
                    input.smooth_scroll_delta.y,
                    input.modifiers.ctrl,
                )
            });

            // Ctrl+wheel and touchpad pinch arrive as `zoom_delta`; a plain
            // wheel is a scroll delta that we translate into a zoom factor.
            let factor = if ctrl || (pinch - 1.0).abs() > 0.001 {
                pinch
            } else {
                (scroll * 0.0022 * sensitivity).exp()
            };

            if (factor - 1.0).abs() > 0.0005 {
                let anchor = response.hover_pos();
                self.zoom_by(factor, anchor, view, image);
                changed = true;
            }
        }

        changed
    }

    /// Rectangle the photo occupies on screen, including rotation and pan.
    pub fn display_rect(&self, view: Rect, image: Vec2) -> Rect {
        let display = Self::rotated_size(image, self.rotation) * self.zoom;
        Rect::from_center_size(view.center() + self.offset, display)
    }

    /// Screen position of an image pixel. `px`/`py` are in image pixels and may
    /// be fractional; rotation and mirroring are taken into account.
    pub fn image_to_screen(&self, px: f32, py: f32, view: Rect, image: Vec2) -> Pos2 {
        let rect = self.display_rect(view, image);
        let (dx, dy) = self.image_to_display(px, py, image);
        rect.min + Vec2::new(dx, dy) * self.zoom
    }

    /// Image pixel under a screen position, or `None` when outside the photo.
    pub fn screen_to_pixel(&self, pos: Pos2, view: Rect, image: Vec2) -> Option<(u32, u32)> {
        if image.x < 1.0 || image.y < 1.0 {
            return None;
        }
        let rect = self.display_rect(view, image);
        if !rect.contains(pos) {
            return None;
        }
        let dx = (pos.x - rect.min.x) / self.zoom;
        let dy = (pos.y - rect.min.y) / self.zoom;
        let (px, py) = self.display_to_image(dx, dy, image);
        let x = px.floor().clamp(0.0, image.x - 1.0) as u32;
        let y = py.floor().clamp(0.0, image.y - 1.0) as u32;
        Some((x, y))
    }

    /// Paints the image into `view`, honouring rotation and mirroring.
    pub fn paint(&self, painter: &Painter, view: Rect, texture: TextureId, image: Vec2) {
        let display = Self::rotated_size(image, self.rotation) * self.zoom;
        if display.x < 1.0 || display.y < 1.0 {
            return;
        }

        let rect = self.display_rect(view, image);
        if !rect.intersects(view) {
            return;
        }

        // The screen corners are always top-left, top-right, bottom-right,
        // bottom-left; the texture coordinates are what rotates and mirrors.
        // They are looked up through the same inverse mapping the pixel readout
        // uses, so the picture and the reported pixel can never disagree.
        let (display_w, display_h) = (display.x / self.zoom, display.y / self.zoom);
        let corners = [
            (0.0, 0.0),
            (display_w, 0.0),
            (display_w, display_h),
            (0.0, display_h),
        ];
        let positions = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];

        let mut mesh = egui::Mesh::with_texture(texture);
        for (corner, pos) in corners.iter().zip(positions.iter()) {
            let (px, py) = self.display_to_image(corner.0, corner.1, image);
            mesh.vertices.push(egui::epaint::Vertex {
                pos: *pos,
                uv: egui::pos2(
                    (px / image.x).clamp(0.0, 1.0),
                    (py / image.y).clamp(0.0, 1.0),
                ),
                color: Color32::WHITE,
            });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(egui::Shape::mesh(mesh));
    }

    /// Draws a grid on the pixel boundaries of the visible part of the photo.
    ///
    /// Only meaningful once the zoom makes single pixels large on screen; the
    /// caller decides when that is. Rotation by a multiple of 90 degrees keeps
    /// the grid axis aligned, it only swaps columns and rows.
    pub fn paint_pixel_grid(&self, painter: &Painter, view: Rect, image: Vec2, color: Color32) {
        /// Refuse to draw a grid so dense that it becomes a grey wash.
        const MAX_LINES_PER_AXIS: i32 = 400;

        let rect = self.display_rect(view, image);
        let visible = rect.intersect(view);
        if visible.width() < 1.0 || visible.height() < 1.0 || self.zoom <= 0.0 {
            return;
        }

        let (columns, rows) = if self.rotation.rem_euclid(180) == 0 {
            (image.x, image.y)
        } else {
            (image.y, image.x)
        };

        let stroke = egui::Stroke::new(1.0, color);

        let first = ((visible.left() - rect.left()) / self.zoom).floor().max(0.0) as i32;
        let last = (((visible.right() - rect.left()) / self.zoom).ceil() as i32).min(columns as i32);
        if last - first <= MAX_LINES_PER_AXIS {
            for k in first..=last {
                let x = rect.left() + k as f32 * self.zoom;
                painter.line_segment(
                    [egui::pos2(x, visible.top()), egui::pos2(x, visible.bottom())],
                    stroke,
                );
            }
        }

        let first = ((visible.top() - rect.top()) / self.zoom).floor().max(0.0) as i32;
        let last = (((visible.bottom() - rect.top()) / self.zoom).ceil() as i32).min(rows as i32);
        if last - first <= MAX_LINES_PER_AXIS {
            for k in first..=last {
                let y = rect.top() + k as f32 * self.zoom;
                painter.line_segment(
                    [egui::pos2(visible.left(), y), egui::pos2(visible.right(), y)],
                    stroke,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn view() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))
    }

    /// The pixel readout has to survive every combination of rotation and
    /// mirroring, otherwise the status bar would report the wrong pixel.
    #[test]
    fn screen_to_pixel_inverts_image_to_screen() {
        let image = vec2(100.0, 50.0);
        let view = view();
        for rotation in [0, 90, 180, 270] {
            for flip_x in [false, true] {
                for flip_y in [false, true] {
                    let state = ViewState {
                        zoom: 2.0,
                        offset: vec2(13.0, -7.0),
                        mode: ZoomMode::Free,
                        rotation,
                        flip_x,
                        flip_y,
                    };
                    for (x, y) in [(0, 0), (1, 0), (0, 1), (37, 21), (99, 49)] {
                        // Aim at the centre of the pixel, not at its corner.
                        let screen = state.image_to_screen(x as f32 + 0.5, y as f32 + 0.5, view, image);
                        let found = state.screen_to_pixel(screen, view, image);
                        assert_eq!(
                            found,
                            Some((x, y)),
                            "rotation {rotation}, flip_x {flip_x}, flip_y {flip_y}"
                        );
                    }
                }
            }
        }
    }

    /// The four image corners must always land on the four corners of the
    /// displayed rectangle, which is what makes the textured quad show the
    /// photo the right way up.
    #[test]
    fn the_displayed_corners_are_the_image_corners() {
        let image = vec2(64.0, 32.0);
        for rotation in [0, 90, 180, 270] {
            for flip_x in [false, true] {
                let state = ViewState {
                    rotation,
                    flip_x,
                    ..ViewState::default()
                };
                // Rotation by 90 or 270 degrees swaps the displayed axes.
                let display = ViewState::rotated_size(image, rotation);
                let corners = [(0.0, 0.0), (64.0, 0.0), (64.0, 32.0), (0.0, 32.0)];
                for (px, py) in corners {
                    let (dx, dy) = state.image_to_display(px, py, image);
                    let on_corner = (dx.abs() < 1e-3 || (dx - display.x).abs() < 1e-3)
                        && (dy.abs() < 1e-3 || (dy - display.y).abs() < 1e-3);
                    assert!(on_corner, "rotation {rotation}: ({px}, {py}) -> ({dx}, {dy})");
                }
            }
        }
    }

    #[test]
    fn flipping_twice_is_the_identity() {
        let image = vec2(64.0, 32.0);
        let mut state = ViewState::default();
        let before = state.image_to_display(10.0, 5.0, image);
        state.flip_horizontal();
        state.flip_horizontal();
        assert_eq!(state.image_to_display(10.0, 5.0, image), before);
        state.flip_vertical();
        state.flip_vertical();
        assert_eq!(state.image_to_display(10.0, 5.0, image), before);
    }
}
