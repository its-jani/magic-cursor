use egui::{
    vec2, Align, Color32, CornerRadius, FontId, Frame, Layout, Pos2, RichText, ScrollArea, Stroke,
    TextEdit, Ui, ViewportBuilder, ViewportCommand,
};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::markdown::MarkdownRenderer;
use super::theme::Theme;
use crate::ai::{AiClient, StreamChunk};
use crate::config::Config;
use crate::context::CapturedContext;

pub struct OverlayApp {
    config: Config,
    context: CapturedContext,
    prompt_input: String,
    response_text: String,
    is_streaming: bool,
    status_message: Option<String>,
    stream_rx: Option<Receiver<StreamChunk>>,
    stream_task: Option<JoinHandle<()>>,
    ai_client: AiClient,
    _cursor_pos: (i32, i32),
    toast: Option<(String, Instant)>,
    show_context_preview: bool,
    initial_focus_set: bool,
    last_pulse_time: Instant,
    pulse_frame: usize,
}

impl OverlayApp {
    pub fn new(
        config: Config,
        context: CapturedContext,
        cursor_pos: (i32, i32),
        ai_client: AiClient,
    ) -> Self {
        Self {
            config,
            context,
            prompt_input: String::new(),
            response_text: String::new(),
            is_streaming: false,
            status_message: None,
            stream_rx: None,
            stream_task: None,
            ai_client,
            _cursor_pos: cursor_pos,
            toast: None,
            show_context_preview: false,
            initial_focus_set: false,
            last_pulse_time: Instant::now(),
            pulse_frame: 0,
        }
    }

    fn send_query(&mut self) {
        let prompt = self.prompt_input.trim().to_string();
        if prompt.is_empty() && self.context.is_empty() {
            return;
        }

        // Abort previous stream if any
        if let Some(task) = self.stream_task.take() {
            task.abort();
        }

        self.response_text.clear();
        self.is_streaming = true;
        self.status_message = Some("Generating response...".to_string());

        let (rx, handle) = self.ai_client.start_stream(
            self.config.api.clone(),
            prompt,
            self.context.clone(),
        );

        self.stream_rx = Some(rx);
        self.stream_task = Some(handle);
    }

    fn cancel_query(&mut self) {
        if let Some(task) = self.stream_task.take() {
            task.abort();
        }
        self.stream_rx = None;
        self.is_streaming = false;
        self.status_message = Some("Cancelled.".to_string());
    }

    fn set_toast(&mut self, msg: &str) {
        self.toast = Some((msg.to_string(), Instant::now()));
    }
}

impl eframe::App for OverlayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        Theme::apply(ctx);

        // Process streaming chunks from async channel
        if let Some(ref mut rx) = self.stream_rx {
            let mut received_any = false;
            while let Ok(chunk) = rx.try_recv() {
                received_any = true;
                match chunk {
                    StreamChunk::Token(token) => {
                        self.response_text.push_str(&token);
                    }
                    StreamChunk::Done => {
                        self.is_streaming = false;
                        self.status_message = None;
                        self.stream_task = None;
                        break;
                    }
                    StreamChunk::Error(err) => {
                        self.is_streaming = false;
                        self.status_message = Some(format!("Error: {}", err));
                        self.stream_task = None;
                        break;
                    }
                }
            }

            if received_any || self.is_streaming {
                ctx.request_repaint();
            }
        }

        // Handle Escape key to close
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        // Update animation pulse
        if self.last_pulse_time.elapsed() > Duration::from_millis(180) {
            self.pulse_frame = (self.pulse_frame + 1) % 4;
            self.last_pulse_time = Instant::now();
        }

        egui::CentralPanel::default()
            .frame(
                Frame::NONE
                    .fill(Theme::BG_DARK)
                    .stroke(Stroke::new(1.0_f32, Theme::BORDER_COLOR))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ctx, |ui| {
                self.render_header(ui, ctx);
                ui.add_space(8.0);
                self.render_context_badge(ui);
                ui.add_space(8.0);
                self.render_input_box(ui, ctx);
                ui.add_space(8.0);
                self.render_quick_chips(ui);
                ui.add_space(8.0);
                self.render_response_area(ui);
                ui.add_space(4.0);
                self.render_footer(ui, ctx);
            });
    }
}

impl OverlayApp {
    fn render_header(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            // Icon + Title
            ui.label(
                RichText::new("✨ MagicCursor")
                    .font(FontId::proportional(15.0))
                    .strong()
                    .color(Theme::ACCENT_PURPLE),
            );

            // Provider Pill
            Frame::NONE
                .fill(Theme::BG_INPUT)
                .corner_radius(CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}: {}",
                            self.config.api.provider.to_uppercase(),
                            self.config.api.model
                        ))
                        .font(FontId::monospace(10.0))
                        .color(Theme::ACCENT_CYAN),
                    );
                });

            // Close Button
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("✕").font(FontId::proportional(13.0)))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                    )
                    .on_hover_text("Close (Esc)")
                    .clicked()
                {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });
        });
    }

    fn render_context_badge(&mut self, ui: &mut Ui) {
        let mut clear_context = false;
        let mut toggle_peek = false;

        match &self.context {
            CapturedContext::Empty => {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("💬 Direct Query Mode")
                            .font(FontId::proportional(11.5))
                            .color(Theme::TEXT_MUTED),
                    );
                });
            }
            CapturedContext::Text(text) => {
                let char_count = text.chars().count();
                let is_peeking = self.show_context_preview;
                let preview: String = text.chars().take(180).collect();

                Frame::NONE
                    .fill(Theme::BG_PANEL)
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 50, 70)))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("📄 [Text Attached]")
                                    .font(FontId::proportional(11.5))
                                    .strong()
                                    .color(Theme::ACCENT_CYAN),
                            );
                            ui.label(
                                RichText::new(format!("({} chars)", char_count))
                                    .font(FontId::monospace(11.0))
                                    .color(Theme::TEXT_MUTED),
                            );

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("Clear").clicked() {
                                    clear_context = true;
                                }
                                if ui.small_button(if is_peeking { "Hide" } else { "Peek" }).clicked() {
                                    toggle_peek = true;
                                }
                            });
                        });

                        if is_peeking {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!("\"{}...\"", preview))
                                    .font(FontId::monospace(10.5))
                                    .color(Theme::TEXT_MUTED),
                            );
                        }
                    });
            }
            CapturedContext::Image(img) => {
                let (w, h) = (img.width, img.height);
                Frame::NONE
                    .fill(Theme::BG_PANEL)
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(45, 50, 70)))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("📸 [Screen Region Captured]")
                                    .font(FontId::proportional(11.5))
                                    .strong()
                                    .color(Theme::ACCENT_PURPLE),
                            );
                            ui.label(
                                RichText::new(format!("({}x{} px)", w, h))
                                    .font(FontId::monospace(11.0))
                                    .color(Theme::TEXT_MUTED),
                            );

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("Clear").clicked() {
                                    clear_context = true;
                                }
                            });
                        });
                    });
            }
        }

        if clear_context {
            self.context = CapturedContext::Empty;
        }
        if toggle_peek {
            self.show_context_preview = !self.show_context_preview;
        }
    }

    fn render_input_box(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let input_id = ui.make_persistent_id("magic_prompt_input");

        let response = ui.add(
            TextEdit::multiline(&mut self.prompt_input)
                .id(input_id)
                .hint_text("Ask anything, translate, explain code, or press Enter to submit...")
                .font(FontId::proportional(13.0))
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::same(8)),
        );

        // Auto focus on first frame
        if !self.initial_focus_set {
            response.request_focus();
            self.initial_focus_set = true;
        }

        // Check for Enter key (without shift) to submit
        if response.has_focus()
            && ctx.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)
        {
            // Remove trailing newline added by enter
            if self.prompt_input.ends_with('\n') {
                self.prompt_input.pop();
            }
            self.send_query();
        }
    }

    fn render_quick_chips(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            let chips = [
                ("⚡ Summarize", "Summarize this clearly in bullet points:"),
                ("🔍 Explain", "Explain this step-by-step:"),
                ("✏️ Fix Grammar", "Proofread and correct any grammar or tone issues:"),
                ("🌐 Translate to EN", "Translate this into fluent English:"),
                ("🛠️ Refactor", "Refactor and optimize this code:"),
            ];

            for (label, prompt_prefix) in chips {
                if ui
                    .add(
                        egui::Button::new(RichText::new(label).font(FontId::proportional(11.0)))
                            .fill(Theme::BG_PANEL)
                            .stroke(Stroke::new(0.5_f32, Theme::BORDER_COLOR))
                            .corner_radius(CornerRadius::same(12)),
                    )
                    .clicked()
                {
                    self.prompt_input = prompt_prefix.to_string();
                    self.send_query();
                }
            }
        });
    }

    fn render_response_area(&mut self, ui: &mut Ui) {
        let available_height = (ui.available_height() - 36.0).max(120.0);

        Frame::NONE
            .fill(Theme::BG_PANEL)
            .stroke(Stroke::new(1.0_f32, Theme::BORDER_COLOR))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .max_height(available_height)
                    .auto_shrink([false, false])
                    .stick_to_bottom(self.is_streaming)
                    .show(ui, |ui| {
                        if self.response_text.is_empty() {
                            if self.is_streaming {
                                let pulse = match self.pulse_frame {
                                    0 => "●○○",
                                    1 => "○●○",
                                    2 => "○○●",
                                    _ => "●●●",
                                };
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Thinking {} ", pulse))
                                            .font(FontId::proportional(13.0))
                                            .color(Theme::ACCENT_INDIGO),
                                    );
                                });
                            } else {
                                ui.label(
                                    RichText::new(
                                        "Type a prompt or select a quick action above and press Enter.",
                                    )
                                    .font(FontId::proportional(12.5))
                                    .color(Theme::TEXT_MUTED),
                                );
                            }
                        } else {
                            MarkdownRenderer::render(ui, &self.response_text);

                            if self.is_streaming {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("▋")
                                        .font(FontId::monospace(12.0))
                                        .color(Theme::ACCENT_PURPLE),
                                );
                            }
                        }
                    });
            });
    }

    fn render_footer(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            // Status or Toast
            if let Some((ref msg, time)) = self.toast {
                if time.elapsed() < Duration::from_secs(2) {
                    ui.label(
                        RichText::new(format!("✓ {}", msg))
                            .font(FontId::proportional(11.0))
                            .color(Theme::ACCENT_GREEN),
                    );
                } else {
                    self.toast = None;
                }
            } else if let Some(ref status) = self.status_message {
                ui.label(
                    RichText::new(status)
                        .font(FontId::proportional(11.0))
                        .color(Theme::TEXT_MUTED),
                );
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.is_streaming {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("⏹ Stop")
                                    .font(FontId::proportional(11.5))
                                    .color(Color32::from_rgb(239, 68, 68)),
                            )
                            .fill(Theme::BG_INPUT),
                        )
                        .clicked()
                    {
                        self.cancel_query();
                    }
                } else if !self.response_text.is_empty() {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("📋 Copy Response")
                                    .font(FontId::proportional(11.5))
                                    .color(Theme::TEXT_PRIMARY),
                            )
                            .fill(Theme::BG_INPUT),
                        )
                        .clicked()
                    {
                        ctx.copy_text(self.response_text.clone());
                        self.set_toast("Copied to clipboard!");
                    }

                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("🔄 Re-ask")
                                    .font(FontId::proportional(11.5))
                                    .color(Theme::TEXT_MUTED),
                            )
                            .fill(Theme::BG_INPUT),
                        )
                        .clicked()
                    {
                        self.send_query();
                    }
                }
            });
        });
    }
}

/// Computes overlay window coordinates based on cursor position clamped to virtual screen
pub fn compute_overlay_position(cursor_x: i32, cursor_y: i32, width: f32, height: f32) -> Pos2 {
    unsafe {
        let virt_x = GetSystemMetrics(SM_XVIRTUALSCREEN) as f32;
        let virt_y = GetSystemMetrics(SM_YVIRTUALSCREEN) as f32;
        let virt_w = GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32;
        let virt_h = GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32;

        let mut x = cursor_x as f32 + 15.0;
        let mut y = cursor_y as f32 + 15.0;

        if x + width > virt_x + virt_w {
            x = (cursor_x as f32) - width - 15.0;
        }
        if y + height > virt_y + virt_h {
            y = (cursor_y as f32) - height - 15.0;
        }

        // Clamp inside virtual screen
        x = x.clamp(virt_x + 10.0, (virt_x + virt_w) - width - 10.0);
        y = y.clamp(virt_y + 10.0, (virt_y + virt_h) - height - 10.0);

        Pos2::new(x, y)
    }
}

/// Spawns the eframe HUD overlay window
pub fn show_overlay_window(
    config: Config,
    context: CapturedContext,
    cursor_pos: (i32, i32),
    ai_client: AiClient,
) {
    let width = config.overlay.width;
    let height = config.overlay.height;
    let pos = compute_overlay_position(cursor_pos.0, cursor_pos.1, width, height);

    let viewport = ViewportBuilder::default()
        .with_decorations(false)
        .with_always_on_top()
        .with_transparent(true)
        .with_inner_size(vec2(width, height))
        .with_position(pos)
        .with_taskbar(false)
        .with_active(true);

    let options = eframe::NativeOptions {
        viewport,
        centered: false,
        vsync: true,
        ..Default::default()
    };

    let _ = eframe::run_native(
        "MagicCursor Overlay",
        options,
        Box::new(|_cc| Ok(Box::new(OverlayApp::new(config, context, cursor_pos, ai_client)))),
    );
}
