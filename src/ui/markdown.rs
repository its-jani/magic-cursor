use egui::{Color32, CornerRadius, FontId, Frame, RichText, Stroke, Ui};

use super::theme::Theme;

pub struct MarkdownRenderer;

impl MarkdownRenderer {
    pub fn render(ui: &mut Ui, text: &str) {
        if text.is_empty() {
            return;
        }

        let mut in_code_block = false;
        let mut code_buffer = String::new();
        let mut _code_language = String::new();

        for line in text.lines() {
            if line.starts_with("```") {
                if in_code_block {
                    // End code block
                    render_code_box(ui, &code_buffer);
                    code_buffer.clear();
                    in_code_block = false;
                } else {
                    // Start code block
                    _code_language = line.trim_start_matches('`').trim().to_string();
                    in_code_block = true;
                }
                continue;
            }

            if in_code_block {
                if !code_buffer.is_empty() {
                    code_buffer.push('\n');
                }
                code_buffer.push_str(line);
            } else if line.starts_with("# ") {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&line[2..])
                        .font(FontId::proportional(17.0))
                        .strong()
                        .color(Theme::TEXT_PRIMARY),
                );
            } else if line.starts_with("## ") {
                ui.add_space(3.0);
                ui.label(
                    RichText::new(&line[3..])
                        .font(FontId::proportional(15.0))
                        .strong()
                        .color(Theme::TEXT_PRIMARY),
                );
            } else if line.starts_with("### ") {
                ui.add_space(2.0);
                ui.label(
                    RichText::new(&line[4..])
                        .font(FontId::proportional(14.0))
                        .strong()
                        .color(Theme::TEXT_PRIMARY),
                );
            } else if line.starts_with("- ") || line.starts_with("* ") {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("•").color(Theme::ACCENT_PURPLE).strong());
                    ui.label(RichText::new(&line[2..]).color(Theme::TEXT_PRIMARY));
                });
            } else if !line.trim().is_empty() {
                ui.label(RichText::new(line).color(Theme::TEXT_PRIMARY).line_height(Some(20.0)));
            } else {
                ui.add_space(4.0);
            }
        }

        if in_code_block && !code_buffer.is_empty() {
            render_code_box(ui, &code_buffer);
        }
    }
}

fn render_code_box(ui: &mut Ui, code: &str) {
    Frame::NONE
        .fill(Color32::from_rgb(15, 16, 22))
        .stroke(Stroke::new(1.0_f32, Theme::BORDER_COLOR))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("CODE")
                        .font(FontId::monospace(10.0))
                        .color(Theme::TEXT_MUTED),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("📋 Copy").clicked() {
                        ui.ctx().copy_text(code.to_string());
                    }
                });
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new(code)
                    .font(FontId::monospace(12.5))
                    .color(Color32::from_rgb(220, 225, 235)),
            );
        });
}
