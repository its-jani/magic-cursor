//! The settings window: provider, API key, model, and endpoint.
//!
//! Opens on the first activation while no API key is configured, and from the
//! tray menu at any time. Saving writes straight to `config.toml`, which the
//! overlay worker re-reads on every trigger.

use std::fmt;
use std::path::PathBuf;

use egui::{FontId, RichText, TextEdit, ViewportBuilder, ViewportCommand};

use super::dialog;
use super::theme;
use crate::config::{Config, Provider};

/// Window size of the settings form in logical pixels.
const SETTINGS_WINDOW_SIZE: egui::Vec2 = egui::vec2(520.0, 470.0);

/// The providers offered in the dropdown, in display order.
const PROVIDERS: [(Provider, &str); 4] = [
    (Provider::Gemini, "Google Gemini"),
    (Provider::Anthropic, "Anthropic Claude"),
    (Provider::Openai, "OpenAI"),
    (Provider::Ollama, "Ollama / local server"),
];

/// Returns the dropdown label of a provider.
fn provider_label(provider: Provider) -> &'static str {
    PROVIDERS
        .iter()
        .find(|(candidate, _)| *candidate == provider)
        .map_or("Google Gemini", |(_, label)| *label)
}

/// The settings form state, seeded from the current `config.toml`.
pub struct SettingsApp {
    config_path: PathBuf,
    provider: Provider,
    api_key: String,
    model: String,
    endpoint: String,
    status: Option<String>,
}

impl fmt::Debug for SettingsApp {
    /// Renders the form with the API key redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsApp")
            .field("config_path", &self.config_path)
            .field("provider", &self.provider)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("status", &self.status)
            .finish()
    }
}

impl SettingsApp {
    /// Loads the editable API fields from `config_path`, or the defaults.
    fn new(config_path: PathBuf) -> Self {
        let config = Config::load_from(&config_path).unwrap_or_default();

        Self {
            config_path,
            provider: config.api.provider,
            api_key: config.api.api_key,
            model: config.api.model,
            endpoint: config.api.endpoint,
            status: None,
        }
    }

    /// Writes the form back to `config.toml`, keeping every other section.
    fn save(&mut self, ctx: &egui::Context) {
        let mut config = Config::load_from(&self.config_path).unwrap_or_default();
        let defaults = Config::default();

        config.api.provider = self.provider;
        config.api.api_key = self.api_key.trim().to_string();
        config.api.model = if self.model.trim().is_empty() {
            defaults.api.model
        } else {
            self.model.trim().to_string()
        };
        config.api.endpoint = if self.endpoint.trim().is_empty() {
            defaults.api.endpoint
        } else {
            self.endpoint.trim().to_string()
        };

        match config.save_to(&self.config_path) {
            Ok(()) => ctx.send_viewport_cmd(ViewportCommand::Close),
            Err(err) => self.status = Some(format!("Could not save: {err}")),
        }
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::apply(ctx);

        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new("🪄 Magic Cursor — Settings")
                    .font(FontId::proportional(18.0))
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
            ui.label(
                RichText::new("Add your API key to activate the Magic Pointer.")
                    .font(FontId::proportional(12.0))
                    .color(theme::TEXT_MUTED),
            );
            ui.add_space(12.0);

            ui.label(RichText::new("Provider").color(theme::TEXT_PRIMARY));
            egui::ComboBox::from_id_salt("settings-provider")
                .selected_text(provider_label(self.provider))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for (provider, label) in PROVIDERS {
                        ui.selectable_value(&mut self.provider, provider, label);
                    }
                });
            ui.add_space(8.0);

            ui.label(RichText::new("API Key").color(theme::TEXT_PRIMARY));
            ui.add(
                TextEdit::singleline(&mut self.api_key)
                    .password(true)
                    .hint_text("Paste your API key here")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(8.0);

            ui.label(RichText::new("Model").color(theme::TEXT_PRIMARY));
            ui.add(
                TextEdit::singleline(&mut self.model)
                    .hint_text("e.g. gemini-2.0-flash")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(8.0);

            ui.label(RichText::new("Endpoint").color(theme::TEXT_PRIMARY));
            ui.add(
                TextEdit::singleline(&mut self.endpoint)
                    .hint_text("Provider base URL")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(14.0);

            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("💾 Save").strong().color(theme::ACCENT_CYAN))
                    .clicked()
                {
                    self.save(ui.ctx());
                }
                if ui.button("Cancel").clicked() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });

            if let Some(status) = &self.status {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(status)
                        .font(FontId::proportional(12.0))
                        .color(egui::Color32::from_rgb(239, 68, 68)),
                );
            }

            ui.add_space(10.0);
            ui.label(
                RichText::new(format!("Saved to: {}", self.config_path.display()))
                    .font(FontId::proportional(10.5))
                    .color(theme::TEXT_MUTED),
            );
        });
    }
}

/// Opens the settings window and blocks until it is closed.
pub fn show_settings_window(config_path: PathBuf) {
    let viewport = ViewportBuilder::default()
        .with_title("Magic Cursor — Settings")
        .with_inner_size(SETTINGS_WINDOW_SIZE)
        .with_resizable(false)
        .with_active(true);

    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        vsync: true,
        ..eframe::NativeOptions::default()
    };

    if let Err(error) = eframe::run_native(
        "Magic Cursor Settings",
        options,
        Box::new(move |_cc| Ok(Box::new(SettingsApp::new(config_path)))),
    ) {
        dialog::show_error(&format!(
            "The settings window could not be opened: {error}\n\n\
             Edit config.toml manually instead (tray menu → Open config.toml)."
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_has_a_label() {
        for (provider, label) in PROVIDERS {
            assert_eq!(provider_label(provider), label, "labels must round-trip");
            assert!(!label.is_empty(), "labels must not be empty");
        }
    }

    #[test]
    fn debug_output_never_contains_the_api_key() {
        let app = SettingsApp {
            config_path: PathBuf::from("config.toml"),
            provider: Provider::Gemini,
            api_key: "sk-settings-secret".to_string(),
            model: String::new(),
            endpoint: String::new(),
            status: None,
        };

        let rendered = format!("{app:?}");

        assert!(rendered.contains("<redacted>"), "got {rendered}");
        assert!(!rendered.contains("sk-settings-secret"), "got {rendered}");
    }
}
