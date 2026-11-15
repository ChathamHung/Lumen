//! Small reusable widgets shared by the toolbar, the status bar and the
//! settings window.

use eframe::egui::{self, Color32, Layout, Response, RichText, Sense, Ui, Vec2};

use crate::icons::{self, Icon};

/// Toolbar button with an icon and a tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str, active: bool, enabled: bool) -> Response {
    icons::button(ui, icon, tooltip, active, enabled)
}

/// Thin vertical divider used between toolbar groups.
pub fn toolbar_separator(ui: &mut Ui) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 22.0), Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.y_range(),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    ui.add_space(4.0);
}

/// Section heading with a divider, used in the settings window.
pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title).strong().size(15.0));
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(4.0);
}

/// Label on the left, control on the right.
pub fn setting_row(ui: &mut Ui, title: &str, hint: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(13.5));
            if !hint.is_empty() {
                ui.label(RichText::new(hint).small().weak());
            }
        });
        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
            control(ui);
        });
    });
    ui.add_space(2.0);
}

/// A boolean row with an animated switch. Returns `true` when toggled.
pub fn toggle(ui: &mut Ui, title: &str, hint: &str, value: &mut bool) -> bool {
    let mut changed = false;
    setting_row(ui, title, hint, |ui| {
        if switch(ui, value).changed() {
            changed = true;
        }
    });
    changed
}

/// iOS-style switch.
pub fn switch(ui: &mut Ui, value: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(38.0, 21.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }

    // `animate_bool_responsive` also requests the repaints the animation needs.
    let t = ui.ctx().animate_bool_responsive(response.id, *value);
    let track = rect.shrink2(Vec2::new(0.0, 2.0));
    let radius = track.height() * 0.5;

    let accent = if ui.visuals().dark_mode {
        crate::theme::ACCENT_DARK
    } else {
        crate::theme::ACCENT_LIGHT
    };
    let off = ui.visuals().widgets.inactive.bg_fill;
    ui.painter()
        .rect_filled(track, radius, lerp_color(off, accent, t));

    let knob_radius = radius - 3.0;
    let knob_x = egui::lerp((track.left() + radius)..=(track.right() - radius), t);
    let knob = if ui.visuals().dark_mode {
        Color32::from_gray(245)
    } else {
        Color32::WHITE
    };
    ui.painter()
        .circle_filled(egui::pos2(knob_x, track.center().y), knob_radius, knob);
    response
}

/// Row of mutually exclusive buttons. Returns `true` when the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (option, label) in options {
            if ui.selectable_label(*value == *option, *label).clicked() && *value != *option {
                *value = *option;
                changed = true;
            }
        }
    });
    changed
}

/// Drop-down list. Returns `true` when the value changed.
pub fn dropdown<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    let current = options
        .iter()
        .find(|(option, _)| option == value)
        .map(|(_, label)| *label)
        .unwrap_or_default();

    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(200.0)
        .show_ui(ui, |ui| {
            for (option, label) in options {
                if ui.selectable_label(*value == *option, *label).clicked() {
                    *value = *option;
                    changed = true;
                }
            }
        });
    changed
}

fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        egui::lerp(from.r() as f32..=to.r() as f32, t) as u8,
        egui::lerp(from.g() as f32..=to.g() as f32, t) as u8,
        egui::lerp(from.b() as f32..=to.b() as f32, t) as u8,
        egui::lerp(from.a() as f32..=to.a() as f32, t) as u8,
    )
}
