use egui::{
    vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Key, Layout, Pos2, Rect, RichText,
    ScrollArea, Stroke, StrokeKind, TextEdit, Ui, ViewportBuilder, ViewportCommand,
};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use super::markdown::MarkdownRenderer;
use super::theme::Theme;
use crate::ai::{AiClient, MultiSelectionContext, SelectedClip, StreamChunk};
use crate::automation::DetectedHotspot;
use crate::capture::CapturedFrame;
use crate::config::Config;

// =========================================================================
// Global Draft State (Auto-saved across invocations)
// =========================================================================

#[derive(Debug, Clone)]
pub struct DraftState {
    pub prompt: String,
    pub tray: Vec<SelectedClip>,
    pub saved_at: Instant,
}

static GLOBAL_DRAFT: Mutex<Option<DraftState>> = Mutex::new(None);

pub fn get_saved_draft() -> Option<DraftState> {
    GLOBAL_DRAFT.lock().ok()?.clone()
}

pub fn save_draft(prompt: String, tray: Vec<SelectedClip>) {
    if prompt.trim().is_empty() && tray.is_empty() {
        if let Ok(mut guard) = GLOBAL_DRAFT.lock() {
            *guard = None;
        }
        return;
    }
    if let Ok(mut guard) = GLOBAL_DRAFT.lock() {
        *guard = Some(DraftState {
            prompt,
            tray,
            saved_at: Instant::now(),
        });
    }
}

pub fn clear_draft() {
    if let Ok(mut guard) = GLOBAL_DRAFT.lock() {
        *guard = None;
    }
}

// =========================================================================
// Overlay Application
// =========================================================================

pub struct OverlayApp {
    config: Config,
    captured_frame: Option<CapturedFrame>,
    hotspots: Vec<DetectedHotspot>,
    _initial_cursor: (i32, i32),
    ai_client: AiClient,

    // Selection & Canvas State
    virt_origin: Pos2,
    hovered_hotspot: Option<usize>,
    selected_hotspot: Option<usize>,
    drag_box_start: Option<Pos2>,
    drag_box_current: Option<Pos2>,
    custom_crop_rect: Option<Rect>,
    selection_spring_anim: f32, // 0.0 to 1.0 for spring animation

    // Multi-Select Tray (Supports up to 10 clips)
    tray: MultiSelectionContext,

    // Floating AI Notetaker Panel
    panel_pos: Pos2,
    _panel_dragging: bool,
    prompt_input: String,
    response_text: String,
    is_streaming: bool,
    status_message: Option<String>,
    stream_rx: Option<Receiver<StreamChunk>>,
    stream_task: Option<JoinHandle<()>>,

    // Animation & Toast
    toast: Option<(String, Instant)>,
    _start_time: Instant,
    has_restored_draft: bool,
    initial_focus_done: bool,
}

impl OverlayApp {
    pub fn new(
        config: Config,
        captured_frame: Option<CapturedFrame>,
        hotspots: Vec<DetectedHotspot>,
        cursor_pos: (i32, i32),
        ai_client: AiClient,
    ) -> Self {
        let (virt_x, virt_y) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN) as f32,
                GetSystemMetrics(SM_YVIRTUALSCREEN) as f32,
            )
        };

        let mut app = Self {
            config,
            captured_frame,
            hotspots,
            _initial_cursor: cursor_pos,
            ai_client,

            virt_origin: Pos2::new(virt_x, virt_y),
            hovered_hotspot: None,
            selected_hotspot: None,
            drag_box_start: None,
            drag_box_current: None,
            custom_crop_rect: None,
            selection_spring_anim: 0.0,

            tray: MultiSelectionContext::default(),

            panel_pos: Pos2::new(
                (cursor_pos.0 as f32 - virt_x + 30.0).max(20.0),
                (cursor_pos.1 as f32 - virt_y - 120.0).max(20.0),
            ),
            _panel_dragging: false,
            prompt_input: String::new(),
            response_text: String::new(),
            is_streaming: false,
            status_message: None,
            stream_rx: None,
            stream_task: None,

            toast: None,
            _start_time: Instant::now(),
            has_restored_draft: false,
            initial_focus_done: false,
        };

        // Select initial hotspot under cursor if any exists
        for (idx, hs) in app.hotspots.iter().enumerate() {
            if hs.contains_point(cursor_pos.0, cursor_pos.1) {
                app.selected_hotspot = Some(idx);
                app.selection_spring_anim = 0.1;
                app.push_hotspot_to_tray(idx);
                break;
            }
        }

        app
    }

    fn push_hotspot_to_tray(&mut self, idx: usize) {
        if idx >= self.hotspots.len() {
            return;
        }
        let hs = &self.hotspots[idx];
        let title = format!("[{}] {}", hs.control_type, hs.name);

        // If frame is captured, also crop visual image of the element
        let clip = if let Some(ref frame) = self.captured_frame {
            if let Some(crop) = frame.crop(hs.x, hs.y, hs.width.max(1) as u32, hs.height.max(1) as u32) {
                if let Some(b64) = crop.to_base64_png() {
                    SelectedClip::Image {
                        title,
                        width: crop.width,
                        height: crop.height,
                        base64_png: b64,
                    }
                } else {
                    SelectedClip::Text {
                        title: hs.name.clone(),
                        content: hs.summary(),
                    }
                }
            } else {
                SelectedClip::Text {
                    title: hs.name.clone(),
                    content: hs.summary(),
                }
            }
        } else {
            SelectedClip::Text {
                title: hs.name.clone(),
                content: hs.summary(),
            }
        };

        if self.tray.add(clip) {
            self.set_toast(&format!("Added '{}' to Tray ({}/10)", hs.name, self.tray.len()));
        }
    }

    fn push_custom_crop_to_tray(&mut self, rect: Rect) {
        let abs_x = (rect.min.x + self.virt_origin.x) as i32;
        let abs_y = (rect.min.y + self.virt_origin.y) as i32;
        let crop_w = rect.width().max(1.0) as u32;
        let crop_h = rect.height().max(1.0) as u32;

        if let Some(ref frame) = self.captured_frame {
            if let Some(crop) = frame.crop(abs_x, abs_y, crop_w, crop_h) {
                if let Some(b64) = crop.to_base64_png() {
                    let title = format!("Crop {}x{} px", crop_w, crop_h);
                    let clip = SelectedClip::Image {
                        title: title.clone(),
                        width: crop_w,
                        height: crop_h,
                        base64_png: b64,
                    };
                    if self.tray.add(clip) {
                        self.set_toast(&format!("Added '{}' to Tray ({}/10)", title, self.tray.len()));
                    }
                    return;
                }
            }
        }

        self.set_toast("Captured region added to selection.");
    }

    fn copy_to_clipboard(&mut self, text: &str) {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(text.to_string());
            self.set_toast("📋 Copied to Clipboard!");
        }
    }

    fn send_query(&mut self) {
        let prompt = self.prompt_input.trim().to_string();
        if prompt.is_empty() && self.tray.is_empty() {
            return;
        }

        if let Some(task) = self.stream_task.take() {
            task.abort();
        }

        self.response_text.clear();
        self.is_streaming = true;
        self.status_message = Some("Generating response...".to_string());

        // Attach REA context from selected hotspot if available
        if let Some(idx) = self.selected_hotspot {
            if let Some(hs) = self.hotspots.get(idx) {
                if let Some(ref rea) = hs.rea_context {
                    self.tray.app_context = Some(format!(
                        "Process: {} ({}) | REA Info: {}",
                        hs.process_name, hs.process_id, rea
                    ));
                }
            }
        }

        let (rx, handle) = self.ai_client.start_stream(
            self.config.api.clone(),
            prompt,
            self.tray.clone(),
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

        // Advance spring animation
        if self.selection_spring_anim > 0.0 && self.selection_spring_anim < 1.0 {
            self.selection_spring_anim = (self.selection_spring_anim + 0.12).min(1.0);
            ctx.request_repaint();
        }

        // Process async streaming tokens
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
                        clear_draft(); // Succeeded, clear auto-save draft
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

        // Handle Escape key to close (auto-saving draft)
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if !self.prompt_input.trim().is_empty() || !self.tray.is_empty() {
                save_draft(self.prompt_input.clone(), self.tray.items.clone());
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        let screen_rect = ctx.screen_rect();

        // 1. Semi-transparent backdrop canvas
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(Color32::from_rgba_unmultiplied(10, 12, 18, 170)))
            .show(ctx, |ui| {
                let pointer_pos = ui.input(|i| i.pointer.hover_pos());
                let mouse_down = ui.input(|i| i.pointer.primary_down());
                let mouse_clicked = ui.input(|i| i.pointer.primary_clicked());
                let ctrl_held = ui.input(|i| i.modifiers.ctrl);

                let painter = ui.painter();

                // 2. Render Hotspots (UI Automation Bounding Boxes)
                let mut hovered_idx = None;
                for (idx, hs) in self.hotspots.iter().enumerate() {
                    let rel_x = (hs.x as f32) - self.virt_origin.x;
                    let rel_y = (hs.y as f32) - self.virt_origin.y;
                    let hs_rect = Rect::from_min_size(
                        Pos2::new(rel_x, rel_y),
                        vec2(hs.width as f32, hs.height as f32),
                    );

                    let is_hovered = pointer_pos.map_or(false, |pos| hs_rect.contains(pos));
                    let is_selected = self.selected_hotspot == Some(idx);

                    if is_hovered {
                        hovered_idx = Some(idx);
                    }

                    // Hotspot background and border
                    if is_selected {
                        // Spring animated glowing bounding box
                        let spring_offset = (1.0 - self.selection_spring_anim) * 4.0;
                        let anim_rect = hs_rect.expand(spring_offset);

                        painter.rect_filled(
                            anim_rect,
                            CornerRadius::same(6),
                            Color32::from_rgba_unmultiplied(139, 92, 246, 35),
                        );
                        painter.rect_stroke(
                            anim_rect,
                            CornerRadius::same(6),
                            Stroke::new(2.0_f32, Theme::ACCENT_PURPLE),
                            StrokeKind::Middle,
                        );

                        // Corner highlight brackets
                        let corner_len = 10.0;
                        let bracket_stroke = Stroke::new(2.5_f32, Theme::ACCENT_CYAN);
                        painter.line_segment([anim_rect.left_top(), anim_rect.left_top() + vec2(corner_len, 0.0)], bracket_stroke);
                        painter.line_segment([anim_rect.left_top(), anim_rect.left_top() + vec2(0.0, corner_len)], bracket_stroke);
                        painter.line_segment([anim_rect.right_bottom(), anim_rect.right_bottom() - vec2(corner_len, 0.0)], bracket_stroke);
                        painter.line_segment([anim_rect.right_bottom(), anim_rect.right_bottom() - vec2(0.0, corner_len)], bracket_stroke);
                    } else if is_hovered {
                        // Hover highlight
                        painter.rect_filled(
                            hs_rect,
                            CornerRadius::same(6),
                            Color32::from_rgba_unmultiplied(6, 182, 212, 25),
                        );
                        painter.rect_stroke(
                            hs_rect,
                            CornerRadius::same(6),
                            Stroke::new(1.5_f32, Theme::ACCENT_CYAN),
                            StrokeKind::Middle,
                        );

                        // Hover badge with element name & REA tag
                        let badge_text = if let Some(ref rea) = hs.rea_context {
                            format!("{}: {} [{}]", hs.control_type, hs.name, rea)
                        } else {
                            format!("{}: {}", hs.control_type, hs.name)
                        };

                        let badge_pos = hs_rect.left_top() - vec2(0.0, 22.0);
                        painter.rect_filled(
                            Rect::from_min_size(badge_pos, vec2(badge_text.len() as f32 * 7.0 + 16.0, 20.0)),
                            CornerRadius::same(4),
                            Color32::from_rgba_unmultiplied(20, 24, 34, 230),
                        );
                        painter.text(
                            badge_pos + vec2(8.0, 3.0),
                            Align2::LEFT_TOP,
                            badge_text,
                            FontId::proportional(11.0),
                            Color32::from_rgb(220, 230, 245),
                        );
                    } else {
                        // Subtle hotspot outline
                        painter.rect_stroke(
                            hs_rect,
                            CornerRadius::same(4),
                            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(100, 116, 139, 40)),
                            StrokeKind::Middle,
                        );
                    }
                }
                self.hovered_hotspot = hovered_idx;

                // Handle Hotspot Click
                if mouse_clicked {
                    if let Some(pos) = pointer_pos {
                        // Check if click wasn't on the floating panel
                        let panel_rect = Rect::from_min_size(self.panel_pos, vec2(540.0, 440.0));
                        if !panel_rect.contains(pos) {
                            if let Some(idx) = hovered_idx {
                                self.selected_hotspot = Some(idx);
                                self.selection_spring_anim = 0.1;
                                self.custom_crop_rect = None;

                                if ctrl_held {
                                    self.push_hotspot_to_tray(idx);
                                }
                            }
                        }
                    }
                }

                // 3. Drag Box Selection (L-Selection / Crop Box)
                if mouse_down {
                    if let Some(pos) = pointer_pos {
                        let panel_rect = Rect::from_min_size(self.panel_pos, vec2(540.0, 440.0));
                        if !panel_rect.contains(pos) && self.hovered_hotspot.is_none() {
                            if self.drag_box_start.is_none() {
                                self.drag_box_start = Some(pos);
                            }
                            self.drag_box_current = Some(pos);
                        }
                    }
                } else {
                    // Mouse released: finalize crop box
                    if let (Some(start), Some(curr)) = (self.drag_box_start.take(), self.drag_box_current.take()) {
                        let rect = Rect::from_two_pos(start, curr);
                        if rect.width() > 15.0 && rect.height() > 15.0 {
                            self.custom_crop_rect = Some(rect);
                            self.selected_hotspot = None;
                            self.push_custom_crop_to_tray(rect);
                        }
                    }
                }

                // Render Active Drag Box / Custom Crop Box
                let active_crop = if let (Some(start), Some(curr)) = (self.drag_box_start, self.drag_box_current) {
                    Some(Rect::from_two_pos(start, curr))
                } else {
                    self.custom_crop_rect
                };

                if let Some(rect) = active_crop {
                    painter.rect_filled(
                        rect,
                        CornerRadius::same(6),
                        Color32::from_rgba_unmultiplied(99, 102, 241, 30),
                    );
                    painter.rect_stroke(
                        rect,
                        CornerRadius::same(6),
                        Stroke::new(2.0_f32, Theme::ACCENT_INDIGO),
                        StrokeKind::Middle,
                    );

                    // Dimension label badge
                    let dim_text = format!("{} × {} px", rect.width() as i32, rect.height() as i32);
                    let dim_pos = rect.right_bottom() + vec2(8.0, -18.0);
                    painter.rect_filled(
                        Rect::from_min_size(dim_pos, vec2(dim_text.len() as f32 * 7.5 + 14.0, 20.0)),
                        CornerRadius::same(4),
                        Color32::from_rgba_unmultiplied(15, 23, 42, 220),
                    );
                    painter.text(
                        dim_pos + vec2(7.0, 3.0),
                        Align2::LEFT_TOP,
                        dim_text,
                        FontId::monospace(11.0),
                        Color32::from_rgb(147, 197, 253),
                    );
                }

                // 4. Render Multi-Select Tray Bar (Top or Bottom Dock)
                self.render_multi_select_tray(ui);

                // 5. Render Floating AI Notetaker Panel
                self.render_floating_notetaker_panel(ui);

                // 6. Toast Notification
                if let Some((ref msg, time)) = self.toast {
                    if time.elapsed() < Duration::from_secs(3) {
                        let toast_pos = Pos2::new(screen_rect.center().x - 140.0, 30.0);
                        let toast_painter = ui.painter();
                        toast_painter.rect_filled(
                            Rect::from_min_size(toast_pos, vec2(280.0, 36.0)),
                            CornerRadius::same(8),
                            Color32::from_rgba_unmultiplied(30, 41, 59, 240),
                        );
                        toast_painter.rect_stroke(
                            Rect::from_min_size(toast_pos, vec2(280.0, 36.0)),
                            CornerRadius::same(8),
                            Stroke::new(1.0_f32, Theme::ACCENT_CYAN),
                            StrokeKind::Middle,
                        );
                        toast_painter.text(
                            toast_pos + vec2(140.0, 18.0),
                            Align2::CENTER_CENTER,
                            msg,
                            FontId::proportional(12.5),
                            Color32::WHITE,
                        );
                    } else {
                        self.toast = None;
                    }
                }
            });
    }
}

impl OverlayApp {
    // =====================================================================
    // Multi-Select Tray Dock Rendering (Up to 10 visual / text items)
    // =====================================================================
    fn render_multi_select_tray(&mut self, ui: &mut Ui) {
        let screen_rect = ui.ctx().screen_rect();
        let tray_w = (self.tray.len() as f32 * 125.0 + 180.0).clamp(240.0, 850.0);
        let tray_rect = Rect::from_min_size(
            Pos2::new(screen_rect.center().x - (tray_w / 2.0), screen_rect.max.y - 70.0),
            vec2(tray_w, 48.0),
        );

        let mut remove_idx = None;
        let mut clear_clicked = false;

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(tray_rect), |ui| {
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(22, 27, 34, 235))
                .stroke(Stroke::new(1.0_f32, Theme::BORDER_COLOR))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("📎 Multi-Tray ({}/10)", self.tray.len()))
                                .font(FontId::proportional(12.0))
                                .strong()
                                .color(Theme::ACCENT_CYAN),
                        );

                        ui.separator();

                        if self.tray.is_empty() {
                            ui.label(
                                RichText::new("Hold Ctrl while clicking or draw box to collect clips")
                                    .font(FontId::proportional(11.5))
                                    .color(Theme::TEXT_MUTED),
                            );
                        } else {
                            for (idx, item) in self.tray.items.iter().enumerate() {
                                let badge_title = match item {
                                    SelectedClip::Text { title, .. } => format!("📝 {}", title),
                                    SelectedClip::Image { title, .. } => format!("🖼 {}", title),
                                };
                                let short_title = if badge_title.len() > 14 {
                                    format!("{}…", &badge_title[..13])
                                } else {
                                    badge_title
                                };

                                Frame::NONE
                                    .fill(Theme::BG_INPUT)
                                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(51, 65, 85)))
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(egui::Margin::symmetric(6, 4))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(short_title)
                                                    .font(FontId::proportional(11.0))
                                                    .color(Theme::TEXT_PRIMARY),
                                            );
                                            if ui.small_button("✕").clicked() {
                                                remove_idx = Some(idx);
                                            }
                                        });
                                    });
                            }

                            if ui.small_button("Clear").clicked() {
                                clear_clicked = true;
                            }
                        }
                    });
                });
        });

        if let Some(idx) = remove_idx {
            self.tray.remove(idx);
            self.set_toast("Item removed from Tray");
        }
        if clear_clicked {
            self.tray.clear();
            self.set_toast("Multi-Select Tray cleared");
        }
    }

    // =====================================================================
    // Floating Fluent AI Notetaker Panel
    // =====================================================================
    fn render_floating_notetaker_panel(&mut self, ui: &mut Ui) {
        let width = self.config.overlay.width.clamp(480.0, 720.0);
        let height = self.config.overlay.height.clamp(400.0, 680.0);

        let screen_rect = ui.ctx().screen_rect();
        // Clamp panel inside screen
        self.panel_pos.x = self.panel_pos.x.clamp(10.0, (screen_rect.max.x - width - 10.0).max(10.0));
        self.panel_pos.y = self.panel_pos.y.clamp(10.0, (screen_rect.max.y - height - 80.0).max(10.0));

        let panel_rect = Rect::from_min_size(self.panel_pos, vec2(width, height));

        let mut trigger_send = false;
        let mut trigger_cancel = false;
        let mut copy_response = false;
        let mut copy_context = false;
        let mut apply_preset = None;

        let has_draft = get_saved_draft().is_some() && !self.has_restored_draft;

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(panel_rect), |ui| {
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(20, 24, 34, 245))
                .stroke(Stroke::new(1.5_f32, Theme::ACCENT_PURPLE))
                .corner_radius(CornerRadius::same(16))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    // Header Bar (Draggable Title)
                    let header_res = ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("🪄 Magic Cursor")
                                .font(FontId::proportional(15.0))
                                .strong()
                                .color(Theme::TEXT_PRIMARY),
                        );

                        // Model badge
                        Frame::NONE
                            .fill(Color32::from_rgba_unmultiplied(139, 92, 246, 40))
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(&self.config.api.model)
                                        .font(FontId::monospace(10.5))
                                        .color(Theme::ACCENT_PURPLE),
                                );
                            });

                        if !self.prompt_input.trim().is_empty() {
                            ui.label(
                                RichText::new("● Draft")
                                    .font(FontId::proportional(10.5))
                                    .color(Theme::ACCENT_GREEN),
                            );
                        }

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button(RichText::new("✕").size(14.0)).clicked() {
                                if !self.prompt_input.trim().is_empty() || !self.tray.is_empty() {
                                    save_draft(self.prompt_input.clone(), self.tray.items.clone());
                                }
                                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                            }
                        });
                    });

                    // Drag panel by header
                    if header_res.response.hovered() && ui.input(|i| i.pointer.primary_down()) {
                        let delta = ui.input(|i| i.pointer.delta());
                        self.panel_pos += delta;
                    }

                    ui.add_space(8.0);

                    // Selected item context info bar
                    ui.horizontal(|ui| {
                        if let Some(idx) = self.selected_hotspot {
                            if let Some(hs) = self.hotspots.get(idx) {
                                ui.label(
                                    RichText::new(format!("🎯 Hotspot: [{}] {}", hs.control_type, hs.name))
                                        .font(FontId::proportional(11.5))
                                        .color(Theme::TEXT_MUTED),
                                );
                            }
                        } else if self.custom_crop_rect.is_some() {
                            ui.label(
                                RichText::new("📐 Custom Region Crop Selected")
                                    .font(FontId::proportional(11.5))
                                    .color(Theme::TEXT_MUTED),
                            );
                        } else {
                            ui.label(
                                RichText::new("🔍 Ready: Select elements or draw a crop region")
                                    .font(FontId::proportional(11.5))
                                    .color(Theme::TEXT_MUTED),
                            );
                        }

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("➕ Add to Tray").clicked() {
                                add_current_current_to_tray_action(self);
                            }
                            if ui.small_button("📋 Copy Clip").clicked() {
                                copy_context = true;
                            }
                        });
                    });

                    ui.add_space(6.0);

                    // Quick-Action Chips
                    ui.horizontal_wrapped(|ui| {
                        if has_draft {
                            if ui
                                .button(RichText::new("↩ Resume Draft").color(Theme::ACCENT_GREEN).strong())
                                .clicked()
                            {
                                if let Some(draft) = get_saved_draft() {
                                    self.prompt_input = draft.prompt;
                                    self.tray.items = draft.tray;
                                    self.has_restored_draft = true;
                                    self.set_toast("Draft restored!");
                                }
                            }
                        }

                        if ui.button("⚡ Summarize").clicked() {
                            apply_preset = Some("Summarize this content concisely in clear bullet points.");
                        }
                        if ui.button("💻 Explain Code").clicked() {
                            apply_preset = Some("Explain this code step-by-step and highlight any logic or security bugs.");
                        }
                        if ui.button("📊 Extract Table").clicked() {
                            apply_preset = Some("Extract the structured table data from this into clean Markdown table format.");
                        }
                        if ui.button("🎯 Action Items").clicked() {
                            apply_preset = Some("Extract all concrete action items and next steps from this context.");
                        }
                    });

                    ui.add_space(6.0);

                    // Free-form multiline prompt input field
                    let prompt_edit = TextEdit::multiline(&mut self.prompt_input)
                        .hint_text("Ask anything about this element or screen region... (Ctrl+Enter to Send)")
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .font(FontId::proportional(13.5));

                    let edit_res = ui.add(prompt_edit);
                    if !self.initial_focus_done {
                        edit_res.request_focus();
                        self.initial_focus_done = true;
                    }

                    if edit_res.has_focus()
                        && ui.input(|i| i.key_pressed(Key::Enter) && (i.modifiers.ctrl || i.modifiers.command))
                    {
                        trigger_send = true;
                    }

                    ui.add_space(6.0);

                    // Actions Bar: Ask AI / Cancel
                    ui.horizontal(|ui| {
                        if self.is_streaming {
                            if ui.button(RichText::new("⏹ Cancel").color(Color32::from_rgb(239, 68, 68))).clicked() {
                                trigger_cancel = true;
                            }
                            ui.spinner();
                            if let Some(ref status) = self.status_message {
                                ui.label(RichText::new(status).font(FontId::proportional(11.5)).color(Theme::TEXT_MUTED));
                            }
                        } else {
                            if ui
                                .button(RichText::new("✨ Ask AI (Ctrl+Enter)").strong().color(Color32::WHITE))
                                .clicked()
                            {
                                trigger_send = true;
                            }
                        }

                        if !self.response_text.is_empty() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button("📋 Copy Response").clicked() {
                                    copy_response = true;
                                }
                            });
                        }
                    });

                    // Streaming Response Output Section
                    if !self.response_text.is_empty() || self.is_streaming {
                        ui.add_space(8.0);
                        ui.separator();

                        ScrollArea::vertical()
                            .max_height(180.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                MarkdownRenderer::render(ui, &self.response_text);
                            });
                    }
                });
        });

        // Handle dispatched actions
        if let Some(preset) = apply_preset {
            self.prompt_input = preset.to_string();
            trigger_send = true;
        }

        if trigger_send {
            self.send_query();
        }

        if trigger_cancel {
            self.cancel_query();
        }

        if copy_response {
            let resp = self.response_text.clone();
            self.copy_to_clipboard(&resp);
        }

        if copy_context {
            if let Some(idx) = self.selected_hotspot {
                if let Some(hs) = self.hotspots.get(idx) {
                    self.copy_to_clipboard(&hs.summary());
                }
            } else if !self.tray.is_empty() {
                let summary: Vec<String> = self.tray.items.iter().map(|c| c.summary()).collect();
                self.copy_to_clipboard(&summary.join("\n"));
            }
        }
    }
}

fn add_current_current_to_tray_action(app: &mut OverlayApp) {
    if let Some(idx) = app.selected_hotspot {
        app.push_hotspot_to_tray(idx);
    } else if let Some(rect) = app.custom_crop_rect {
        app.push_custom_crop_to_tray(rect);
    }
}

// =========================================================================
// Overlay Launcher Function
// =========================================================================

pub fn show_magic_overlay(
    config: Config,
    captured_frame: Option<CapturedFrame>,
    hotspots: Vec<DetectedHotspot>,
    cursor_pos: (i32, i32),
    ai_client: AiClient,
) {
    let (virt_x, virt_y, virt_w, virt_h) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_YVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32,
        )
    };

    let viewport = ViewportBuilder::default()
        .with_decorations(false)
        .with_always_on_top()
        .with_transparent(true)
        .with_inner_size(vec2(virt_w, virt_h))
        .with_position(Pos2::new(virt_x, virt_y))
        .with_taskbar(false)
        .with_active(true);

    let options = eframe::NativeOptions {
        viewport,
        centered: false,
        vsync: true,
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Magic Cursor Overlay",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(OverlayApp::new(
                config,
                captured_frame,
                hotspots,
                cursor_pos,
                ai_client,
            )))
        }),
    );
}
