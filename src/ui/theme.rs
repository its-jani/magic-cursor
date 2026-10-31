//! Fluent-inspired colour tokens and the egui style they produce.

use egui::{Color32, CornerRadius, Margin, Stroke, Style, Visuals};

/// Window background.
pub const BG_DARK: Color32 = Color32::from_rgb(18, 18, 24);
/// Cards and toolbars sitting on the background.
pub const BG_PANEL: Color32 = Color32::from_rgb(26, 27, 38);
/// Text inputs and badges.
pub const BG_INPUT: Color32 = Color32::from_rgb(34, 35, 48);
/// Primary accent, used for the panel outline and selection frames.
pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(139, 92, 246);
/// Secondary accent, used for crop rectangles.
pub const ACCENT_INDIGO: Color32 = Color32::from_rgb(99, 102, 241);
/// Highlight colour for hovered hotspots and badges.
pub const ACCENT_CYAN: Color32 = Color32::from_rgb(6, 182, 212);
/// Success colour for drafts and restored state.
pub const ACCENT_GREEN: Color32 = Color32::from_rgb(34, 197, 94);
/// Default text colour.
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(243, 244, 246);
/// Secondary text colour.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(156, 163, 175);
/// Hairline borders around panels and inputs.
pub const BORDER_COLOR: Color32 = Color32::from_rgb(55, 65, 81);

/// Corner radius shared by panels and frames.
const CORNER_RADIUS: u8 = 8;

/// Installs the dark Fluent palette on the given context.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();

    visuals.override_text_color = Some(TEXT_PRIMARY);
    visuals.panel_fill = BG_DARK;
    visuals.window_fill = BG_DARK;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER_COLOR);
    visuals.window_corner_radius = CornerRadius::same(12);

    visuals.widgets.noninteractive.bg_fill = BG_PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_COLOR);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(CORNER_RADIUS);

    visuals.widgets.inactive.bg_fill = BG_PANEL;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER_COLOR);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(CORNER_RADIUS);

    visuals.widgets.hovered.bg_fill = ACCENT_INDIGO;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT_PURPLE);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(CORNER_RADIUS);

    visuals.widgets.active.bg_fill = ACCENT_PURPLE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.5_f32, ACCENT_CYAN);
    visuals.widgets.active.corner_radius = CornerRadius::same(CORNER_RADIUS);

    let mut style = Style {
        visuals,
        ..Style::default()
    };
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.window_margin = Margin::same(12);

    ctx.set_style(style);
}
