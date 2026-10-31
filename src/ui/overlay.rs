//! The full-screen overlay: hotspot rendering, crop selection, the tray, and
//! the floating AI panel.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use egui::{
    vec2, Align, Align2, Color32, CornerRadius, FontId, Frame, Key, Layout, Pos2, Rect, RichText,
    ScrollArea, Stroke, StrokeKind, TextEdit, Ui, ViewportBuilder, ViewportCommand,
};
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

use super::markdown;
use super::theme;
use crate::ai::{AiClient, MultiSelectionContext, SelectedClip, StreamChunk, MAX_TRAY_CLIPS};
use crate::automation::DetectedHotspot;
use crate::capture::{CapturedFrame, VirtualScreen};
use crate::config::Config;

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

/// Smallest panel width the configuration may request.
const PANEL_MIN_WIDTH: f32 = 480.0;
/// Largest panel width the configuration may request.
const PANEL_MAX_WIDTH: f32 = 720.0;
/// Smallest panel height the configuration may request.
const PANEL_MIN_HEIGHT: f32 = 400.0;
/// Largest panel height the configuration may request.
const PANEL_MAX_HEIGHT: f32 = 680.0;
/// Distance the panel keeps from the screen edges.
const PANEL_SCREEN_MARGIN: f32 = 10.0;
/// Offset applied to place the panel below the cursor on first open.
const PANEL_CURSOR_OFFSET: egui::Vec2 = egui::vec2(30.0, -120.0);

/// Height of the multi-select dock.
const TRAY_HEIGHT: f32 = 48.0;
/// Distance the dock keeps from the bottom edge.
const TRAY_BOTTOM_MARGIN: f32 = 70.0;
/// Horizontal space one tray badge needs.
const TRAY_BADGE_WIDTH: f32 = 125.0;
/// Width the dock keeps for its label and clear button.
const TRAY_CHROME_WIDTH: f32 = 180.0;
/// Narrowest the dock may become.
const TRAY_MIN_WIDTH: f32 = 240.0;
/// Widest the dock may become.
const TRAY_MAX_WIDTH: f32 = 850.0;
/// Characters kept in a tray badge before it is ellipsised.
const TRAY_TITLE_CHARS: usize = 13;

/// How long a toast stays on screen.
const TOAST_DURATION: Duration = Duration::from_secs(3);
/// Toast width.
const TOAST_WIDTH: f32 = 280.0;
/// Toast height.
const TOAST_HEIGHT: f32 = 36.0;
/// Distance between the toast and the top edge.
const TOAST_TOP: f32 = 30.0;

/// Spring animation step per frame.
const SPRING_STEP: f32 = 0.12;
/// Spring animation value that restarts the selection bounce.
const SPRING_RESTART: f32 = 0.1;

/// Smallest drag rectangle that counts as a crop.
const MIN_CROP_SIZE: f32 = 15.0;

/// Height limit of the streaming response area.
const RESPONSE_MAX_HEIGHT: f32 = 180.0;

/// Length of the corner brackets drawn around the selection.
const BRACKET_LENGTH: f32 = 10.0;

// ---------------------------------------------------------------------------
// Draft persistence
// ---------------------------------------------------------------------------

/// A prompt and tray snapshot kept between overlay invocations.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftState {
    /// Text the user had typed.
    pub prompt: String,
    /// Clips the user had collected.
    pub tray: Vec<SelectedClip>,
}

/// The single saved draft, shared across overlay windows.
static GLOBAL_DRAFT: Mutex<Option<DraftState>> = Mutex::new(None);

/// Returns a copy of the saved draft, if one exists.
pub fn get_saved_draft() -> Option<DraftState> {
    let guard = GLOBAL_DRAFT.lock().expect("draft lock poisoned");
    guard.clone()
}

/// Saves a draft, or clears it when both the prompt and the tray are empty.
pub fn save_draft(prompt: String, tray: Vec<SelectedClip>) {
    let mut guard = GLOBAL_DRAFT.lock().expect("draft lock poisoned");

    if prompt.trim().is_empty() && tray.is_empty() {
        *guard = None;
        return;
    }

    *guard = Some(DraftState { prompt, tray });
}

/// Forgets the saved draft.
pub fn clear_draft() {
    let mut guard = GLOBAL_DRAFT.lock().expect("draft lock poisoned");
    *guard = None;
}

// ---------------------------------------------------------------------------
// Overlay application
// ---------------------------------------------------------------------------

/// Everything one overlay invocation needs.
#[derive(Debug)]
pub struct OverlayRequest {
    /// Snapshot of the user configuration for this invocation.
    pub config: Config,
    /// Desktop frame captured when the trigger fired.
    pub frame: Option<CapturedFrame>,
    /// Hotspots detected under the cursor.
    pub hotspots: Vec<DetectedHotspot>,
    /// Cursor position that triggered the overlay.
    pub cursor: (i32, i32),
    /// Shared HTTP client for the AI panel.
    pub ai_client: AiClient,
}

/// Pointer state sampled once per frame.
#[derive(Debug, Clone, Copy)]
struct CanvasInput {
    pointer: Option<Pos2>,
    clicked: bool,
    down: bool,
    ctrl: bool,
}

/// Result of one frame of panel interaction, applied after the UI closure ends.
///
/// Each field is one independent button in the panel action bar, so the group of
/// flags mirrors the UI rather than a single mode.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is a distinct one-shot button click, not a mode switch"
)]
#[derive(Debug, Default)]
struct PanelActions {
    send: bool,
    cancel: bool,
    copy_response: bool,
    copy_context: bool,
    preset: Option<&'static str>,
}

/// The overlay window state.
#[derive(Debug)]
pub struct OverlayApp {
    config: Config,
    captured_frame: Option<CapturedFrame>,
    hotspots: Vec<DetectedHotspot>,
    ai_client: AiClient,

    virt_origin: Pos2,
    hovered_hotspot: Option<usize>,
    selected_hotspot: Option<usize>,
    drag_box_start: Option<Pos2>,
    drag_box_current: Option<Pos2>,
    custom_crop_rect: Option<Rect>,
    selection_spring_anim: f32,

    tray: MultiSelectionContext,

    panel_pos: Pos2,
    prompt_input: String,
    response_text: String,
    is_streaming: bool,
    status_message: Option<String>,
    stream_rx: Option<Receiver<StreamChunk>>,
    stream_task: Option<JoinHandle<()>>,

    toast: Option<(String, Instant)>,
    has_restored_draft: bool,
    initial_focus_done: bool,
}

impl OverlayApp {
    /// Creates the overlay state and pre-selects the hotspot under the cursor.
    pub fn new(request: OverlayRequest) -> Self {
        let origin = VirtualScreen::current();

        let mut app = Self {
            config: request.config,
            captured_frame: request.frame,
            hotspots: request.hotspots,
            ai_client: request.ai_client,

            virt_origin: Pos2::new(origin.x as f32, origin.y as f32),
            hovered_hotspot: None,
            selected_hotspot: None,
            drag_box_start: None,
            drag_box_current: None,
            custom_crop_rect: None,
            selection_spring_anim: 0.0,

            tray: MultiSelectionContext::default(),

            panel_pos: Pos2::new(
                (request.cursor.0 as f32 - origin.x as f32 + PANEL_CURSOR_OFFSET.x)
                    .max(PANEL_SCREEN_MARGIN),
                (request.cursor.1 as f32 - origin.y as f32 + PANEL_CURSOR_OFFSET.y)
                    .max(PANEL_SCREEN_MARGIN),
            ),
            prompt_input: String::new(),
            response_text: String::new(),
            is_streaming: false,
            status_message: None,
            stream_rx: None,
            stream_task: None,

            toast: None,
            has_restored_draft: false,
            initial_focus_done: false,
        };

        if let Some(index) = app.hotspot_under(request.cursor) {
            app.selected_hotspot = Some(index);
            app.selection_spring_anim = SPRING_RESTART;
            app.push_hotspot_to_tray(index);
        }

        app
    }

    /// Returns the index of the first hotspot containing `position`.
    fn hotspot_under(&self, position: (i32, i32)) -> Option<usize> {
        self.hotspots
            .iter()
            .position(|hotspot| hotspot.contains_point(position.0, position.1))
    }

    /// Returns the panel size clamped to the supported range.
    fn panel_size(&self) -> (f32, f32) {
        (
            self.config
                .overlay
                .width
                .clamp(PANEL_MIN_WIDTH, PANEL_MAX_WIDTH),
            self.config
                .overlay
                .height
                .clamp(PANEL_MIN_HEIGHT, PANEL_MAX_HEIGHT),
        )
    }

    /// Returns the panel rectangle for the current position, used for hit-testing.
    fn panel_rect(&self) -> Rect {
        let (width, height) = self.panel_size();
        Rect::from_min_size(self.panel_pos, vec2(width, height))
    }

    /// Keeps the panel fully on screen.
    fn clamp_panel(&mut self, screen: Rect) {
        let (width, height) = self.panel_size();

        self.panel_pos.x = self.panel_pos.x.clamp(
            PANEL_SCREEN_MARGIN,
            (screen.max.x - width - PANEL_SCREEN_MARGIN).max(PANEL_SCREEN_MARGIN),
        );
        self.panel_pos.y = self.panel_pos.y.clamp(
            PANEL_SCREEN_MARGIN,
            (screen.max.y - height - TRAY_BOTTOM_MARGIN).max(PANEL_SCREEN_MARGIN),
        );
    }

    /// Builds the tray clip for a hotspot, preferring a cropped screenshot.
    fn clip_for_hotspot(&self, index: usize) -> Option<SelectedClip> {
        let hotspot = self.hotspots.get(index)?;

        let text_clip = || SelectedClip::Text {
            title: hotspot.name.clone(),
            content: hotspot.summary(),
        };

        let Some(frame) = self.captured_frame.as_ref() else {
            return Some(text_clip());
        };

        let width = u32::try_from(hotspot.width.max(1)).unwrap_or(1);
        let height = u32::try_from(hotspot.height.max(1)).unwrap_or(1);

        let Some(crop) = frame.crop(hotspot.x, hotspot.y, width, height) else {
            return Some(text_clip());
        };
        let (crop_width, crop_height) = (crop.width, crop.height);

        match crop.into_base64_png() {
            Some(base64_png) => Some(SelectedClip::Image {
                title: hotspot.tray_title(),
                width: crop_width,
                height: crop_height,
                base64_png,
            }),
            None => Some(text_clip()),
        }
    }

    /// Adds the hotspot at `index` to the tray and reports the result.
    fn push_hotspot_to_tray(&mut self, index: usize) {
        let Some(clip) = self.clip_for_hotspot(index) else {
            return;
        };

        let name = clip.title().to_string();
        if self.tray.add(clip) {
            let count = self.tray.len();
            self.set_toast(&format!(
                "Added '{name}' to Tray ({count}/{MAX_TRAY_CLIPS})"
            ));
        } else {
            self.set_toast(&format!("Tray is full ({MAX_TRAY_CLIPS} clips)"));
        }
    }

    /// Crops `rect` out of the captured frame and adds it to the tray.
    fn push_custom_crop_to_tray(&mut self, rect: Rect) {
        let x = (rect.min.x + self.virt_origin.x) as i32;
        let y = (rect.min.y + self.virt_origin.y) as i32;
        let width = rect.width().max(1.0) as u32;
        let height = rect.height().max(1.0) as u32;
        let title = format!("Crop {width}x{height} px");

        let clip = self
            .captured_frame
            .as_ref()
            .and_then(|frame| frame.crop(x, y, width, height))
            .and_then(super::super::capture::dxgi::CapturedFrame::into_base64_png)
            .map(|base64_png| SelectedClip::Image {
                title: title.clone(),
                width,
                height,
                base64_png,
            });

        match clip {
            Some(clip) => {
                if self.tray.add(clip) {
                    self.set_toast(&format!("Added '{title}' to Tray"));
                } else {
                    self.set_toast(&format!("Tray is full ({MAX_TRAY_CLIPS} clips)"));
                }
            }
            None => self.set_toast("Captured region could not be read"),
        }
    }

    /// Adds whatever the user currently has selected to the tray.
    fn add_current_selection_to_tray(&mut self) {
        if let Some(index) = self.selected_hotspot {
            self.push_hotspot_to_tray(index);
        } else if let Some(rect) = self.custom_crop_rect {
            self.push_custom_crop_to_tray(rect);
        } else {
            self.set_toast("Select an element or draw a region first");
        }
    }

    /// Starts a streaming request for the current prompt and tray.
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

        if let Some(hotspot) = self
            .selected_hotspot
            .and_then(|index| self.hotspots.get(index))
        {
            if let Some(context) = &hotspot.rea_context {
                self.tray.app_context = Some(format!(
                    "Process: {} ({}) | REA Info: {context}",
                    hotspot.process_name, hotspot.process_id
                ));
            }
        }

        let (receiver, handle) =
            self.ai_client
                .start_stream(self.config.api.clone(), prompt, self.tray.clone());

        self.stream_rx = Some(receiver);
        self.stream_task = Some(handle);
    }

    /// Aborts the running request and reports it as cancelled.
    fn cancel_query(&mut self) {
        if let Some(task) = self.stream_task.take() {
            task.abort();
        }
        self.stream_rx = None;
        self.is_streaming = false;
        self.status_message = Some("Cancelled.".to_string());
    }

    /// Shows a transient message at the top of the screen.
    fn set_toast(&mut self, message: &str) {
        self.toast = Some((message.to_string(), Instant::now()));
    }

    /// Shows the toast that matches a clipboard write.
    fn report_copy(&mut self, copied: bool) {
        self.set_toast(if copied {
            "📋 Copied to Clipboard!"
        } else {
            "Clipboard is unavailable"
        });
    }

    /// Saves the current prompt and tray so the next invocation can restore them.
    fn persist_draft(&mut self) {
        if self.prompt_input.trim().is_empty() && self.tray.is_empty() {
            return;
        }
        save_draft(self.prompt_input.clone(), self.tray.items.clone());
    }

    /// Advances the selection spring animation while it is running.
    fn advance_spring(&mut self, ctx: &egui::Context) {
        if self.selection_spring_anim > 0.0 && self.selection_spring_anim < 1.0 {
            self.selection_spring_anim = (self.selection_spring_anim + SPRING_STEP).min(1.0);
            ctx.request_repaint();
        }
    }

    /// Drains every pending stream message without blocking the frame.
    fn drain_stream(&mut self, ctx: &egui::Context) {
        let Some(receiver) = self.stream_rx.as_mut() else {
            return;
        };

        let mut received_any = false;
        while let Ok(chunk) = receiver.try_recv() {
            received_any = true;

            match chunk {
                StreamChunk::Token(token) => self.response_text.push_str(&token),
                StreamChunk::Done => {
                    self.is_streaming = false;
                    self.status_message = None;
                    self.stream_task = None;
                    clear_draft();
                    break;
                }
                StreamChunk::Error(error) => {
                    self.is_streaming = false;
                    self.status_message = Some(format!("Error: {error}"));
                    self.stream_task = None;
                    break;
                }
            }
        }

        if received_any || self.is_streaming {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for OverlayApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        theme::apply(ctx);
        self.advance_spring(ctx);
        self.drain_stream(ctx);

        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.persist_draft();
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        let screen = ctx.screen_rect();
        self.clamp_panel(screen);

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(Color32::from_rgba_unmultiplied(10, 12, 18, 170)))
            .show(ctx, |ui| {
                let input = CanvasInput {
                    pointer: ui.input(|i| i.pointer.hover_pos()),
                    clicked: ui.input(|i| i.pointer.primary_clicked()),
                    down: ui.input(|i| i.pointer.primary_down()),
                    ctrl: ui.input(|i| i.modifiers.ctrl),
                };

                self.render_hotspots(ui);
                self.handle_canvas_interaction(&input);
                self.render_crop_box(ui);
                self.render_multi_select_tray(ui);
                self.render_notetaker_panel(ui);
                self.render_toast(ui, screen);
            });
    }
}

impl OverlayApp {
    /// Draws every detected hotspot, highlighting the hovered and selected ones.
    fn render_hotspots(&mut self, ui: &mut Ui) {
        let painter = ui.painter();
        let cursor_pos = ui.input(|i| i.pointer.hover_pos());
        let mut hovered = None;

        for (index, hotspot) in self.hotspots.iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(
                    hotspot.x as f32 - self.virt_origin.x,
                    hotspot.y as f32 - self.virt_origin.y,
                ),
                vec2(hotspot.width as f32, hotspot.height as f32),
            );

            let is_hovered = cursor_pos.is_some_and(|position| rect.contains(position));
            if is_hovered {
                hovered = Some(index);
            }

            if self.selected_hotspot == Some(index) {
                self.paint_selection(ui, rect);
            } else if is_hovered {
                Self::paint_hover(ui, rect, hotspot);
            } else {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(100, 116, 139, 40)),
                    StrokeKind::Middle,
                );
            }
        }

        self.hovered_hotspot = hovered;
    }

    /// Draws the spring-animated frame around the selected hotspot.
    fn paint_selection(&self, ui: &Ui, rect: Rect) {
        let painter = ui.painter();
        let expansion = (1.0 - self.selection_spring_anim) * 4.0;
        let frame_rect = rect.expand(expansion);

        painter.rect_filled(
            frame_rect,
            CornerRadius::same(6),
            Color32::from_rgba_unmultiplied(139, 92, 246, 35),
        );
        painter.rect_stroke(
            frame_rect,
            CornerRadius::same(6),
            Stroke::new(2.0_f32, theme::ACCENT_PURPLE),
            StrokeKind::Middle,
        );

        let bracket = Stroke::new(2.5_f32, theme::ACCENT_CYAN);
        let corners = [
            (frame_rect.left_top(), vec2(BRACKET_LENGTH, 0.0)),
            (frame_rect.left_top(), vec2(0.0, BRACKET_LENGTH)),
            (frame_rect.right_bottom(), vec2(-BRACKET_LENGTH, 0.0)),
            (frame_rect.right_bottom(), vec2(0.0, -BRACKET_LENGTH)),
        ];

        for (corner, offset) in corners {
            painter.line_segment([corner, corner + offset], bracket);
        }
    }

    /// Draws the hover frame and its label badge.
    fn paint_hover(ui: &Ui, rect: Rect, hotspot: &DetectedHotspot) {
        let painter = ui.painter();

        painter.rect_filled(
            rect,
            CornerRadius::same(6),
            Color32::from_rgba_unmultiplied(6, 182, 212, 25),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.5_f32, theme::ACCENT_CYAN),
            StrokeKind::Middle,
        );

        let label = match &hotspot.rea_context {
            Some(context) => format!("{}: {} [{context}]", hotspot.control_type, hotspot.name),
            None => format!("{}: {}", hotspot.control_type, hotspot.name),
        };

        let badge_position = rect.left_top() - vec2(0.0, 22.0);
        let badge_size = vec2(label.chars().count() as f32 * 7.0 + 16.0, 20.0);

        painter.rect_filled(
            Rect::from_min_size(badge_position, badge_size),
            CornerRadius::same(4),
            Color32::from_rgba_unmultiplied(20, 24, 34, 230),
        );
        painter.text(
            badge_position + vec2(8.0, 3.0),
            Align2::LEFT_TOP,
            label,
            FontId::proportional(11.0),
            Color32::from_rgb(220, 230, 245),
        );
    }

    /// Applies hover, click, and drag interactions on the canvas.
    fn handle_canvas_interaction(&mut self, input: &CanvasInput) {
        let Some(position) = input.pointer else {
            return;
        };

        let on_panel = self.panel_rect().contains(position);

        if input.clicked && !on_panel {
            if let Some(index) = self.hovered_hotspot {
                self.selected_hotspot = Some(index);
                self.selection_spring_anim = SPRING_RESTART;
                self.custom_crop_rect = None;

                if input.ctrl {
                    self.push_hotspot_to_tray(index);
                }
            }
        }

        if input.down {
            if !on_panel && self.hovered_hotspot.is_none() {
                if self.drag_box_start.is_none() {
                    self.drag_box_start = Some(position);
                }
                self.drag_box_current = Some(position);
            }
            return;
        }

        // Mouse released: finalize a crop box if the drag was large enough.
        let Some((start, current)) = self.drag_box_start.take().zip(self.drag_box_current.take())
        else {
            return;
        };

        let rect = Rect::from_two_pos(start, current);
        if rect.width() > MIN_CROP_SIZE && rect.height() > MIN_CROP_SIZE {
            self.custom_crop_rect = Some(rect);
            self.selected_hotspot = None;
            self.push_custom_crop_to_tray(rect);
        }
    }

    /// Draws the active drag box or the last finished crop.
    fn render_crop_box(&self, ui: &Ui) {
        let active = self
            .drag_box_start
            .zip(self.drag_box_current)
            .map(|(start, current)| Rect::from_two_pos(start, current))
            .or(self.custom_crop_rect);

        let Some(rect) = active else {
            return;
        };

        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(6),
            Color32::from_rgba_unmultiplied(99, 102, 241, 30),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(2.0_f32, theme::ACCENT_INDIGO),
            StrokeKind::Middle,
        );

        let dimensions = format!("{} × {} px", rect.width() as i32, rect.height() as i32);
        let label_position = rect.right_bottom() + vec2(8.0, -18.0);
        let label_size = vec2(dimensions.chars().count() as f32 * 7.5 + 14.0, 20.0);

        painter.rect_filled(
            Rect::from_min_size(label_position, label_size),
            CornerRadius::same(4),
            Color32::from_rgba_unmultiplied(15, 23, 42, 220),
        );
        painter.text(
            label_position + vec2(7.0, 3.0),
            Align2::LEFT_TOP,
            dimensions,
            FontId::monospace(11.0),
            Color32::from_rgb(147, 197, 253),
        );
    }

    /// Draws the transient toast at the top of the screen.
    fn render_toast(&mut self, ui: &Ui, screen: Rect) {
        let Some((message, shown_at)) = self.toast.as_ref() else {
            return;
        };

        if shown_at.elapsed() >= TOAST_DURATION {
            self.toast = None;
            return;
        }

        let painter = ui.painter();
        let position = Pos2::new(screen.center().x - TOAST_WIDTH / 2.0, TOAST_TOP);
        let rect = Rect::from_min_size(position, vec2(TOAST_WIDTH, TOAST_HEIGHT));

        painter.rect_filled(
            rect,
            CornerRadius::same(8),
            Color32::from_rgba_unmultiplied(30, 41, 59, 240),
        );
        painter.rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, theme::ACCENT_CYAN),
            StrokeKind::Middle,
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            message,
            FontId::proportional(12.5),
            Color32::WHITE,
        );
    }

    /// Draws the clip dock along the bottom edge.
    fn render_multi_select_tray(&mut self, ui: &mut Ui) {
        let screen = ui.ctx().screen_rect();
        let width = ((self.tray.len() as f32) * TRAY_BADGE_WIDTH + TRAY_CHROME_WIDTH)
            .clamp(TRAY_MIN_WIDTH, TRAY_MAX_WIDTH);
        let rect = Rect::from_min_size(
            Pos2::new(
                screen.center().x - width / 2.0,
                screen.max.y - TRAY_BOTTOM_MARGIN,
            ),
            vec2(width, TRAY_HEIGHT),
        );

        let mut remove_index = None;
        let mut clear_clicked = false;

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(22, 27, 34, 235))
                .stroke(Stroke::new(1.0_f32, theme::BORDER_COLOR))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "📎 Multi-Tray ({}/{MAX_TRAY_CLIPS})",
                                self.tray.len()
                            ))
                            .font(FontId::proportional(12.0))
                            .strong()
                            .color(theme::ACCENT_CYAN),
                        );

                        ui.separator();

                        if self.tray.is_empty() {
                            ui.label(
                                RichText::new(
                                    "Hold Ctrl while clicking or draw box to collect clips",
                                )
                                .font(FontId::proportional(11.5))
                                .color(theme::TEXT_MUTED),
                            );
                            return;
                        }

                        for (index, clip) in self.tray.items.iter().enumerate() {
                            if tray_badge(ui, clip) {
                                remove_index = Some(index);
                            }
                        }

                        if ui.small_button("Clear").clicked() {
                            clear_clicked = true;
                        }
                    });
                });
        });

        if let Some(index) = remove_index {
            self.tray.remove(index);
            self.set_toast("Item removed from Tray");
        }

        if clear_clicked {
            self.tray.clear();
            self.set_toast("Multi-Select Tray cleared");
        }
    }

    /// Draws the floating AI notetaker panel.
    fn render_notetaker_panel(&mut self, ui: &mut Ui) {
        let panel_rect = self.panel_rect();
        let has_draft = get_saved_draft().is_some() && !self.has_restored_draft;
        let mut actions = PanelActions::default();

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(panel_rect), |ui| {
            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(20, 24, 34, 245))
                .stroke(Stroke::new(1.5_f32, theme::ACCENT_PURPLE))
                .corner_radius(CornerRadius::same(16))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    self.render_panel_header(ui);
                    ui.add_space(8.0);
                    self.render_context_bar(ui, &mut actions);
                    ui.add_space(6.0);
                    self.render_quick_actions(ui, has_draft, &mut actions);
                    ui.add_space(6.0);
                    self.render_prompt_area(ui, &mut actions);
                    ui.add_space(6.0);
                    self.render_action_bar(ui, &mut actions);
                    self.render_response(ui);
                });
        });

        self.apply_panel_actions(&actions);
    }

    /// Draws the draggable header with the model badge and close button.
    fn render_panel_header(&mut self, ui: &mut Ui) {
        let header = ui.horizontal(|ui| {
            ui.label(
                RichText::new("🪄 Magic Cursor")
                    .font(FontId::proportional(15.0))
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );

            Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(139, 92, 246, 40))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(&self.config.api.model)
                            .font(FontId::monospace(10.5))
                            .color(theme::ACCENT_PURPLE),
                    );
                });

            if !self.prompt_input.trim().is_empty() {
                ui.label(
                    RichText::new("● Draft")
                        .font(FontId::proportional(10.5))
                        .color(theme::ACCENT_GREEN),
                );
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(RichText::new("✕").size(14.0)).clicked() {
                    self.persist_draft();
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });
        });

        if header.response.hovered() && ui.input(|i| i.pointer.primary_down()) {
            self.panel_pos += ui.input(|i| i.pointer.delta());
        }
    }

    /// Draws the selection summary and its two clip actions.
    fn render_context_bar(&mut self, ui: &mut Ui, actions: &mut PanelActions) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.selection_summary())
                    .font(FontId::proportional(11.5))
                    .color(theme::TEXT_MUTED),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("➕ Add to Tray").clicked() {
                    self.add_current_selection_to_tray();
                }
                if ui.small_button("📋 Copy Clip").clicked() {
                    actions.copy_context = true;
                }
            });
        });
    }

    /// Returns the one-line description of what is currently selected.
    fn selection_summary(&self) -> String {
        if let Some(hotspot) = self
            .selected_hotspot
            .and_then(|index| self.hotspots.get(index))
        {
            return format!("🎯 Hotspot: [{}] {}", hotspot.control_type, hotspot.name);
        }

        if self.custom_crop_rect.is_some() {
            return "📐 Custom Region Crop Selected".to_string();
        }

        "🔍 Ready: Select elements or draw a crop region".to_string()
    }

    /// Draws the draft chip and the quick-action presets.
    fn render_quick_actions(&mut self, ui: &mut Ui, has_draft: bool, actions: &mut PanelActions) {
        ui.horizontal_wrapped(|ui| {
            if has_draft
                && ui
                    .button(
                        RichText::new("↩ Resume Draft")
                            .color(theme::ACCENT_GREEN)
                            .strong(),
                    )
                    .clicked()
            {
                if let Some(draft) = get_saved_draft() {
                    self.prompt_input = draft.prompt;
                    self.tray.items = draft.tray;
                    self.has_restored_draft = true;
                    self.set_toast("Draft restored!");
                }
            }

            for preset in PRESETS {
                if ui.button(preset.label).clicked() {
                    actions.preset = Some(preset.prompt);
                }
            }
        });
    }

    /// Draws the prompt editor and handles its keyboard shortcuts.
    fn render_prompt_area(&mut self, ui: &mut Ui, actions: &mut PanelActions) {
        let editor = TextEdit::multiline(&mut self.prompt_input)
            .hint_text("Ask anything about this element or screen region... (Ctrl+Enter to Send)")
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .font(FontId::proportional(13.5));

        let response = ui.add(editor);

        if !self.initial_focus_done {
            response.request_focus();
            self.initial_focus_done = true;
        }

        if response.has_focus()
            && ui.input(|i| i.key_pressed(Key::Enter) && (i.modifiers.ctrl || i.modifiers.command))
        {
            actions.send = true;
        }
    }

    /// Draws the send/cancel row and the response copy button.
    fn render_action_bar(&mut self, ui: &mut Ui, actions: &mut PanelActions) {
        ui.horizontal(|ui| {
            if self.is_streaming {
                if ui
                    .button(RichText::new("⏹ Cancel").color(Color32::from_rgb(239, 68, 68)))
                    .clicked()
                {
                    actions.cancel = true;
                }

                ui.spinner();

                if let Some(status) = &self.status_message {
                    ui.label(
                        RichText::new(status)
                            .font(FontId::proportional(11.5))
                            .color(theme::TEXT_MUTED),
                    );
                }
            } else if ui
                .button(
                    RichText::new("✨ Ask AI (Ctrl+Enter)")
                        .strong()
                        .color(Color32::WHITE),
                )
                .clicked()
            {
                actions.send = true;
            }

            if !self.response_text.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("📋 Copy Response").clicked() {
                        actions.copy_response = true;
                    }
                });
            }
        });
    }

    /// Draws the streaming markdown output area.
    fn render_response(&self, ui: &mut Ui) {
        if self.response_text.is_empty() && !self.is_streaming {
            return;
        }

        ui.add_space(8.0);
        ui.separator();

        ScrollArea::vertical()
            .max_height(RESPONSE_MAX_HEIGHT)
            .auto_shrink([false, false])
            .show(ui, |ui| markdown::render(ui, &self.response_text));
    }

    /// Applies the actions collected while drawing the panel.
    fn apply_panel_actions(&mut self, actions: &PanelActions) {
        if let Some(preset) = actions.preset {
            self.prompt_input = preset.to_string();
            self.send_query();
            return;
        }

        if actions.send {
            self.send_query();
        }

        if actions.cancel {
            self.cancel_query();
        }

        if actions.copy_response {
            let copied = copy_text(&self.response_text);
            self.report_copy(copied);
        }

        if actions.copy_context {
            if let Some(hotspot) = self
                .selected_hotspot
                .and_then(|index| self.hotspots.get(index))
            {
                let copied = copy_text(&hotspot.summary());
                self.report_copy(copied);
            } else if !self.tray.is_empty() {
                let summary = self
                    .tray
                    .items
                    .iter()
                    .map(SelectedClip::summary)
                    .collect::<Vec<_>>()
                    .join("\n");
                let copied = copy_text(&summary);
                self.report_copy(copied);
            }
        }
    }
}

/// Writes `text` to the system clipboard, returning whether it succeeded.
fn copy_text(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text.to_string()))
        .is_ok()
}

/// A quick-action chip: its button label and the prompt it sends.
#[derive(Debug, Clone, Copy)]
struct Preset {
    label: &'static str,
    prompt: &'static str,
}

/// The quick-action chips offered above the prompt box.
const PRESETS: [Preset; 4] = [
    Preset {
        label: "⚡ Summarize",
        prompt: "Summarize this content concisely in clear bullet points.",
    },
    Preset {
        label: "💻 Explain Code",
        prompt: "Explain this code step-by-step and highlight any logic or security bugs.",
    },
    Preset {
        label: "📊 Extract Table",
        prompt: "Extract the structured table data from this into clean Markdown table format.",
    },
    Preset {
        label: "🎯 Action Items",
        prompt: "Extract all concrete action items and next steps from this context.",
    },
];

/// Draws one tray badge, returning whether its remove button was clicked.
fn tray_badge(ui: &mut Ui, clip: &SelectedClip) -> bool {
    let glyph = match clip {
        SelectedClip::Text { .. } => "📝",
        SelectedClip::Image { .. } => "🖼",
    };
    let title: String = clip.title().chars().take(TRAY_TITLE_CHARS).collect();
    let title = if title.chars().count() < clip.title().chars().count() {
        format!("{title}…")
    } else {
        title
    };

    let mut removed = false;

    Frame::NONE
        .fill(theme::BG_INPUT)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(51, 65, 85)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{glyph} {title}"))
                        .font(FontId::proportional(11.0))
                        .color(theme::TEXT_PRIMARY),
                );
                if ui.small_button("✕").clicked() {
                    removed = true;
                }
            });
        });

    removed
}

/// Opens the overlay window for one invocation.
pub fn show_magic_overlay(request: OverlayRequest) {
    let screen = VirtualScreen::current();

    let viewport = ViewportBuilder::default()
        .with_decorations(false)
        .with_always_on_top()
        .with_transparent(true)
        .with_inner_size(vec2(screen.width as f32, screen.height as f32))
        .with_position(Pos2::new(screen.x as f32, screen.y as f32))
        .with_taskbar(false)
        .with_active(true);

    let options = eframe::NativeOptions {
        viewport,
        centered: false,
        vsync: true,
        ..eframe::NativeOptions::default()
    };

    if let Err(error) = eframe::run_native(
        "Magic Cursor Overlay",
        options,
        Box::new(move |_cc| Ok(Box::new(OverlayApp::new(request)))),
    ) {
        eprintln!("[magic-cursor] overlay window failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_distinct_and_non_empty() {
        assert_eq!(PRESETS.len(), 4);
        for preset in PRESETS {
            assert_ne!(preset.label, "");
            assert_ne!(preset.prompt, "");
        }
    }

    #[test]
    fn saving_an_empty_draft_clears_it() {
        save_draft("hello".to_string(), Vec::new());
        assert!(get_saved_draft().is_some());

        save_draft("   ".to_string(), Vec::new());
        assert!(get_saved_draft().is_none());
    }
}
