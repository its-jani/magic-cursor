//! Minimal streaming-markdown renderer for model output.
//!
//! It understands fenced code blocks, three heading levels, and bullet lists,
//! which is the subset the quick-action prompts produce. Anything else is drawn
//! as a paragraph so no text is ever dropped while a response streams in.

use egui::{Color32, CornerRadius, FontId, Frame, Layout, RichText, Stroke, Ui};

use super::theme;

/// Heading sizes, indexed by `level - 1`.
const HEADING_SIZES: [f32; 3] = [17.0, 15.0, 14.0];

/// Background of a fenced code block.
const CODE_BG: Color32 = Color32::from_rgb(15, 16, 22);

/// Foreground of a fenced code block.
const CODE_FG: Color32 = Color32::from_rgb(220, 225, 235);

/// Renders `text` into `ui`, one markdown block at a time.
pub fn render(ui: &mut Ui, text: &str) {
    let mut code_block: Option<CodeBlock> = None;

    for line in text.lines() {
        // A fence closes an open block and opens a closed one.
        if line.trim_start().starts_with("```") {
            match code_block.take() {
                Some(block) => render_code_box(ui, &block),
                None => code_block = Some(CodeBlock::opening(line)),
            }
            continue;
        }

        if let Some(block) = code_block.as_mut() {
            block.push(line);
            continue;
        }

        render_block(ui, line);
    }

    // A stream can end mid-block; show what arrived instead of dropping it.
    if let Some(block) = code_block {
        render_code_box(ui, &block);
    }
}

/// Renders one non-code line.
fn render_block(ui: &mut Ui, line: &str) {
    if let Some(heading) = heading(line) {
        ui.add_space(4.0);
        ui.label(
            RichText::new(heading.text)
                .font(FontId::proportional(HEADING_SIZES[heading.level]))
                .strong()
                .color(theme::TEXT_PRIMARY),
        );
        return;
    }

    if let Some(item) = bullet(line) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("•").color(theme::ACCENT_PURPLE).strong());
            ui.label(RichText::new(item).color(theme::TEXT_PRIMARY));
        });
        return;
    }

    if line.trim().is_empty() {
        ui.add_space(4.0);
        return;
    }

    ui.label(
        RichText::new(line)
            .color(theme::TEXT_PRIMARY)
            .line_height(Some(20.0)),
    );
}

/// A parsed heading line.
#[derive(Debug, PartialEq, Eq)]
struct Heading<'a> {
    level: usize,
    text: &'a str,
}

/// Splits `line` into a heading level and its text, if it is a heading.
fn heading(line: &str) -> Option<Heading<'_>> {
    for (level, marker) in ["# ", "## ", "### "].iter().enumerate() {
        if let Some(text) = line.strip_prefix(marker) {
            return Some(Heading { level, text });
        }
    }
    None
}

/// Returns the text of a bullet item, if the line is one.
fn bullet(line: &str) -> Option<&str> {
    line.strip_prefix("- ").or_else(|| line.strip_prefix("* "))
}

/// Accumulates the lines of one fenced code block.
struct CodeBlock {
    language: String,
    lines: Vec<String>,
}

impl CodeBlock {
    /// Starts a block from its opening fence, keeping any language label.
    fn opening(fence: &str) -> Self {
        Self {
            language: fence
                .trim_start()
                .trim_start_matches('`')
                .trim()
                .to_string(),
            lines: Vec::new(),
        }
    }

    /// Appends one line of code.
    fn push(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }

    /// Returns the block body.
    fn body(&self) -> String {
        self.lines.join("\n")
    }

    /// Returns the language label shown in the header, if the fence named one.
    fn label(&self) -> &str {
        if self.language.is_empty() {
            "CODE"
        } else {
            &self.language
        }
    }
}

/// Draws a code block with the shared copy button.
fn render_code_box(ui: &mut Ui, block: &CodeBlock) {
    let code = block.body();

    Frame::NONE
        .fill(CODE_BG)
        .stroke(Stroke::new(1.0_f32, theme::BORDER_COLOR))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(block.label())
                        .font(FontId::monospace(10.0))
                        .color(theme::TEXT_MUTED),
                );
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("📋 Copy").clicked() {
                        ui.ctx().copy_text(code.clone());
                    }
                });
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new(&code)
                    .font(FontId::monospace(12.5))
                    .color(CODE_FG),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_headings_and_bullets() {
        assert_eq!(
            heading("## Notes").map(|h| (h.level, h.text)),
            Some((1, "Notes"))
        );
        assert_eq!(heading("plain"), None);
        assert_eq!(bullet("- first"), Some("first"));
        assert_eq!(bullet("• already a bullet"), None);
    }

    #[test]
    fn fence_language_lands_in_the_label() {
        let block = CodeBlock::opening("```rust");

        assert_eq!(block.label(), "rust");
        assert_eq!(CodeBlock::opening("```").label(), "CODE");
    }

    #[test]
    fn block_body_joins_lines() {
        let mut block = CodeBlock::opening("```");
        block.push("fn main() {}");
        block.push("let x = 1;");

        assert_eq!(block.body(), "fn main() {}\nlet x = 1;");
    }
}
