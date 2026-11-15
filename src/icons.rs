//! Vector icons and the window icon.
//!
//! Everything is drawn with painter primitives instead of an icon font, so the
//! icons stay crisp at any DPI and no extra asset or font has to be shipped.

use eframe::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2,
};

/// Icons used by the interface.
///
/// The set is deliberately complete: adding a new button never requires new
/// drawing code, only a new match arm in [`paint`].
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    ChevronLeft,
    ChevronRight,
    Plus,
    Minus,
    Fit,
    OneToOne,
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    Play,
    Pause,
    Filmstrip,
    Info,
    Settings,
    Fullscreen,
    ExitFullscreen,
    Folder,
    Image,
    Close,
    Export,
    Import,
    Reset,
    Check,
    Warning,
}

/// Draws `icon` inside `rect`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32, width: f32) {
    // Leave a little breathing space so the icon never touches the button edge.
    let area = rect.shrink(rect.width() * 0.22);
    let stroke = Stroke::new(width, color);
    let map = |x: f32, y: f32| {
        Pos2::new(
            area.min.x + area.width() * x,
            area.min.y + area.height() * y,
        )
    };
    let line = |points: Vec<Pos2>| {
        painter.add(egui::Shape::line(points, stroke));
    };
    let rect_of = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(map(x0, y0), map(x1, y1));

    match icon {
        Icon::ChevronLeft => line(vec![map(0.66, 0.10), map(0.30, 0.5), map(0.66, 0.90)]),
        Icon::ChevronRight => line(vec![map(0.34, 0.10), map(0.70, 0.5), map(0.34, 0.90)]),
        Icon::Plus => {
            line(vec![map(0.5, 0.12), map(0.5, 0.88)]);
            line(vec![map(0.12, 0.5), map(0.88, 0.5)]);
        }
        Icon::Minus => {
            line(vec![map(0.12, 0.5), map(0.88, 0.5)]);
        }
        Icon::Fit => {
            // A photo sitting inside a frame: the outer rectangle is the
            // window, the inner one is the photo scaled to fit.
            painter.rect_stroke(rect_of(0.0, 0.12, 1.0, 0.88), 2.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(rect_of(0.24, 0.28, 0.76, 0.72), 1.6, stroke, egui::StrokeKind::Inside);
        }
        Icon::OneToOne => {
            painter.text(
                area.center(),
                Align2::CENTER_CENTER,
                "1:1",
                FontId::proportional(area.height() * 0.62),
                color,
            );
        }
        Icon::RotateLeft | Icon::RotateRight => {
            // A circular arrow: most of a circle plus a solid arrow head at the
            // open end. The head is deliberately large so the icon still reads
            // as "rotate" at toolbar size.
            let center = area.center();
            let radius = area.width() * 0.30;
            let clockwise = icon == Icon::RotateRight;
            let sweep = 1.55 * std::f32::consts::PI;
            let (start, end) = if clockwise {
                (0.72 * std::f32::consts::PI, 0.72 * std::f32::consts::PI + sweep)
            } else {
                (0.28 * std::f32::consts::PI, 0.28 * std::f32::consts::PI - sweep)
            };
            arc(painter, center, radius, start, end, stroke);

            // The head points the way the photo will turn.
            let tangent = if clockwise {
                end + std::f32::consts::FRAC_PI_2
            } else {
                end - std::f32::consts::FRAC_PI_2
            };
            let tip = center + Vec2::angled(end) * (radius + stroke.width * 0.5);
            arrow_head(painter, tip, tangent, area.width() * 0.34, color);
        }
        Icon::FlipHorizontal | Icon::FlipVertical => {
            // A mirror line with two arrows pointing away from it, the shape
            // every image editor uses for this action.
            let horizontal = icon == Icon::FlipHorizontal;
            let mirror = if horizontal {
                [map(0.5, 0.04), map(0.5, 0.96)]
            } else {
                [map(0.04, 0.5), map(0.96, 0.5)]
            };
            painter.line_segment(mirror, Stroke::new(stroke.width * 0.8, color));

            // Each arrow is a triangle whose apex points away from the mirror.
            let triangles: [[(f32, f32); 3]; 2] = if horizontal {
                [
                    [(0.40, 0.20), (0.40, 0.80), (0.08, 0.50)],
                    [(0.60, 0.20), (0.60, 0.80), (0.92, 0.50)],
                ]
            } else {
                [
                    [(0.20, 0.40), (0.80, 0.40), (0.50, 0.08)],
                    [(0.20, 0.60), (0.80, 0.60), (0.50, 0.92)],
                ]
            };
            for triangle in triangles {
                let points: Vec<Pos2> = triangle.iter().map(|(x, y)| map(*x, *y)).collect();
                painter.add(egui::Shape::closed_line(points, stroke));
            }
        }
        Icon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![map(0.30, 0.14), map(0.30, 0.86), map(0.84, 0.5)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            painter.rect_filled(rect_of(0.26, 0.16, 0.44, 0.84), 1.5, color);
            painter.rect_filled(rect_of(0.56, 0.16, 0.74, 0.84), 1.5, color);
        }
        Icon::Filmstrip => {
            painter.rect_stroke(rect_of(0.0, 0.16, 1.0, 0.84), 2.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(rect_of(0.10, 0.28, 0.36, 0.72), 1.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(rect_of(0.44, 0.28, 0.70, 0.72), 1.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(rect_of(0.78, 0.28, 0.90, 0.72), 1.0, stroke, egui::StrokeKind::Inside);
        }
        Icon::Info => {
            painter.circle_stroke(area.center(), area.width() * 0.46, stroke);
            painter.circle_filled(map(0.5, 0.28), width * 0.9, color);
            line(vec![map(0.5, 0.45), map(0.5, 0.74)]);
        }
        Icon::Settings => {
            for (y, knob) in [(0.24, 0.36), (0.5, 0.66), (0.76, 0.44)] {
                line(vec![map(0.08, y), map(0.92, y)]);
                painter.circle_filled(map(knob, y), area.width() * 0.10, color);
            }
        }
        Icon::Fullscreen => {
            for (cx, cy, dx, dy) in [
                (0.06, 0.10, 1.0, 1.0),
                (0.94, 0.10, -1.0, 1.0),
                (0.06, 0.90, 1.0, -1.0),
                (0.94, 0.90, -1.0, -1.0),
            ] {
                line(vec![map(cx, cy), map(cx + dx * 0.30, cy)]);
                line(vec![map(cx, cy), map(cx, cy + dy * 0.30)]);
            }
        }
        Icon::ExitFullscreen => {
            for (cx, cy, dx, dy) in [
                (0.30, 0.34, -1.0, -1.0),
                (0.70, 0.34, 1.0, -1.0),
                (0.30, 0.66, -1.0, 1.0),
                (0.70, 0.66, 1.0, 1.0),
            ] {
                line(vec![map(cx, cy), map(cx + dx * 0.26, cy)]);
                line(vec![map(cx, cy), map(cx, cy + dy * 0.26)]);
            }
        }
        Icon::Folder => {
            line(vec![
                map(0.04, 0.26),
                map(0.38, 0.26),
                map(0.46, 0.40),
                map(0.96, 0.40),
                map(0.96, 0.80),
                map(0.04, 0.80),
                map(0.04, 0.26),
            ]);
        }
        Icon::Image => {
            painter.rect_stroke(rect_of(0.02, 0.14, 0.98, 0.86), 2.0, stroke, egui::StrokeKind::Inside);
            painter.circle_filled(map(0.32, 0.36), area.width() * 0.08, color);
            line(vec![map(0.10, 0.78), map(0.38, 0.48), map(0.58, 0.68), map(0.72, 0.54), map(0.90, 0.78)]);
        }
        Icon::Close => {
            line(vec![map(0.18, 0.18), map(0.82, 0.82)]);
            line(vec![map(0.82, 0.18), map(0.18, 0.82)]);
        }
        Icon::Export => {
            line(vec![map(0.14, 0.56), map(0.14, 0.88), map(0.86, 0.88), map(0.86, 0.56)]);
            line(vec![map(0.5, 0.72), map(0.5, 0.14)]);
            line(vec![map(0.30, 0.34), map(0.5, 0.14), map(0.70, 0.34)]);
        }
        Icon::Import => {
            line(vec![map(0.14, 0.56), map(0.14, 0.88), map(0.86, 0.88), map(0.86, 0.56)]);
            line(vec![map(0.5, 0.14), map(0.5, 0.70)]);
            line(vec![map(0.30, 0.50), map(0.5, 0.70), map(0.70, 0.50)]);
        }
        Icon::Reset => {
            let center = area.center();
            let radius = area.width() * 0.34;
            let start = 0.75 * std::f32::consts::PI;
            let end = 2.55 * std::f32::consts::PI;
            arc(painter, center, radius, start, end, stroke);
            arrow_head(
                painter,
                center + Vec2::angled(end) * radius,
                end + std::f32::consts::FRAC_PI_2,
                area.width() * 0.20,
                color,
            );
        }
        Icon::Check => line(vec![map(0.16, 0.54), map(0.40, 0.78), map(0.84, 0.24)]),
        Icon::Warning => {
            line(vec![map(0.5, 0.08), map(0.96, 0.90), map(0.04, 0.90), map(0.5, 0.08)]);
            line(vec![map(0.5, 0.38), map(0.5, 0.64)]);
            painter.circle_filled(map(0.5, 0.76), width * 0.85, color);
        }
    }
}

/// A toolbar button with an icon, hover highlight and tooltip.
pub fn button(ui: &mut Ui, icon: Icon, tooltip: &str, active: bool, enabled: bool) -> Response {
    let size = Vec2::splat(30.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);

    let visuals = ui.style().interact_selectable(&response, active);
    let background = if active {
        ui.visuals().selection.bg_fill
    } else if response.hovered() && enabled {
        visuals.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    if background != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 8.0, background);
    }

    let color = if !enabled {
        ui.visuals().weak_text_color()
    } else if active {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    paint(ui.painter(), rect, icon, color, 1.6);

    response.on_hover_text(tooltip)
}

/// Approximates a circular arc with a polyline.
fn arc(painter: &Painter, center: Pos2, radius: f32, start: f32, end: f32, stroke: Stroke) {
    const SEGMENTS: usize = 28;
    let points = (0..=SEGMENTS)
        .map(|i| {
            let angle = start + (end - start) * (i as f32 / SEGMENTS as f32);
            center + Vec2::angled(angle) * radius
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));
}

/// Filled triangular arrow head pointing along `direction`.
fn arrow_head(painter: &Painter, tip: Pos2, direction: f32, size: f32, color: Color32) {
    let forward = Vec2::angled(direction);
    let side = Vec2::new(-forward.y, forward.x);
    let base = tip - forward * size * 0.9;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, base + side * size * 0.62, base - side * size * 0.62],
        color,
        Stroke::NONE,
    ));
}

/// Procedurally generated window icon: a rounded blue square with a sun and two
/// mountains. Keeps the executable free of embedded image assets.
pub fn app_icon() -> egui::IconData {
    const SIZE: u32 = 64;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let fx = (x as f32 + 0.5) / SIZE as f32;
            let fy = (y as f32 + 0.5) / SIZE as f32;

            // Squircle mask with a soft edge for anti-aliasing.
            let nx = (fx - 0.5).abs() * 2.0;
            let ny = (fy - 0.5).abs() * 2.0;
            let distance = (nx.powi(4) + ny.powi(4)).powf(0.25);
            let mask = ((1.0 - distance) / 0.05).clamp(0.0, 1.0);
            if mask <= 0.0 {
                continue;
            }

            // Diagonal gradient background.
            let t = (fx * 0.55 + fy * 0.45).clamp(0.0, 1.0);
            let mut color = [
                lerp(0x3B, 0x1C, t),
                lerp(0x82, 0xB6, t),
                lerp(0xF6, 0xA8, t),
                255.0,
            ];

            // Sun.
            let sun = ((fx - 0.34).powi(2) + (fy - 0.33).powi(2)).sqrt();
            if sun < 0.105 {
                color = [255.0, 255.0, 255.0, 235.0];
            }

            // Two mountains.
            if point_in_triangle((fx, fy), (0.08, 0.80), (0.40, 0.38), (0.72, 0.80))
                || point_in_triangle((fx, fy), (0.40, 0.80), (0.66, 0.50), (0.94, 0.80))
            {
                color = [255.0, 255.0, 255.0, 225.0];
            }

            let index = ((y * SIZE + x) * 4) as usize;
            rgba[index] = color[0] as u8;
            rgba[index + 1] = color[1] as u8;
            rgba[index + 2] = color[2] as u8;
            rgba[index + 3] = (color[3] * mask) as u8;
        }
    }

    egui::IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}

fn lerp(from: u8, to: u8, t: f32) -> f32 {
    from as f32 + (to as f32 - from as f32) * t
}

/// Standard barycentric point-in-triangle test. Points are `(x, y)` pairs.
fn point_in_triangle(
    point: (f32, f32),
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
) -> bool {
    let (px, py) = point;
    let (ax, ay) = a;
    let (bx, by) = b;
    let (cx, cy) = c;
    let d1 = (px - bx) * (ay - by) - (ax - bx) * (py - by);
    let d2 = (px - cx) * (by - cy) - (bx - cx) * (py - cy);
    let d3 = (px - ax) * (cy - ay) - (cx - ax) * (py - ay);
    let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_negative && has_positive)
}
