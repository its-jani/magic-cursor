use egui::{Color32, CornerRadius, Stroke, Style, Visuals};

pub struct Theme;

impl Theme {
    pub const BG_DARK: Color32 = Color32::from_rgb(18, 18, 24);
    pub const BG_PANEL: Color32 = Color32::from_rgb(26, 27, 38);
    pub const BG_INPUT: Color32 = Color32::from_rgb(34, 35, 48);
    pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(139, 92, 246);
    pub const ACCENT_INDIGO: Color32 = Color32::from_rgb(99, 102, 241);
    pub const ACCENT_CYAN: Color32 = Color32::from_rgb(6, 182, 212);
    pub const ACCENT_GREEN: Color32 = Color32::from_rgb(34, 197, 94);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(243, 244, 246);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(156, 163, 175);
    pub const BORDER_COLOR: Color32 = Color32::from_rgb(55, 65, 81);

    pub fn apply(ctx: &egui::Context) {
        let mut visuals = Visuals::dark();

        visuals.override_text_color = Some(Self::TEXT_PRIMARY);
        visuals.panel_fill = Self::BG_DARK;
        visuals.window_fill = Self::BG_DARK;
        visuals.window_stroke = Stroke::new(1.0_f32, Self::BORDER_COLOR);
        visuals.window_corner_radius = CornerRadius::same(12);

        // Widgets
        visuals.widgets.noninteractive.bg_fill = Self::BG_PANEL;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Self::BORDER_COLOR);
        visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);

        visuals.widgets.inactive.bg_fill = Self::BG_PANEL;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Self::BORDER_COLOR);
        visuals.widgets.inactive.corner_radius = CornerRadius::same(8);

        visuals.widgets.hovered.bg_fill = Self::ACCENT_INDIGO;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Self::ACCENT_PURPLE);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(8);

        visuals.widgets.active.bg_fill = Self::ACCENT_PURPLE;
        visuals.widgets.active.bg_stroke = Stroke::new(1.5_f32, Self::ACCENT_CYAN);
        visuals.widgets.active.corner_radius = CornerRadius::same(8);

        let mut style = Style::default();
        style.visuals = visuals;
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(12);

        ctx.set_style(style);
    }
}
