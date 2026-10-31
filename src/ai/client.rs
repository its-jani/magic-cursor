//! The provider-agnostic streaming client used by the overlay.
//!
//! [`AiClient`] owns one HTTP connection pool and routes each request to the
//! Gemini, Anthropic, or OpenAI-compatible stream implementation based on
//! [`crate::config::Provider`].

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{channel, Receiver};
use tokio::task::JoinHandle;

use super::{anthropic, gemini, openai};
use crate::config::{ApiConfig, Provider};

/// Maximum number of clips the multi-select tray accepts.
pub const MAX_TRAY_CLIPS: usize = 10;

/// Buffer depth for streamed tokens; deep enough to absorb a burst without
/// applying backpressure to the HTTP body, small enough to stay cheap.
const STREAM_CHANNEL_CAPACITY: usize = 64;

/// One captured screenshot region or text block collected in the tray.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SelectedClip {
    /// Text copied from a detected UI element.
    Text {
        /// Short label shown on the tray badge.
        title: String,
        /// Full text sent to the model.
        content: String,
    },
    /// A cropped screenshot region encoded as PNG.
    Image {
        /// Short label shown on the tray badge.
        title: String,
        /// Crop width in pixels.
        width: u32,
        /// Crop height in pixels.
        height: u32,
        /// Standard base64 encoding of the PNG bytes.
        base64_png: String,
    },
}

impl SelectedClip {
    /// Returns the tray label of this clip.
    pub fn title(&self) -> &str {
        match self {
            Self::Text { title, .. } | Self::Image { title, .. } => title,
        }
    }

    /// Returns a one-line description used in prompts and clipboard copies.
    pub fn summary(&self) -> String {
        match self {
            Self::Text { title, content } => {
                format!("Text: {title} ({} chars)", content.chars().count())
            }
            Self::Image {
                title,
                width,
                height,
                ..
            } => format!("Image: {title} ({width}x{height} px)"),
        }
    }
}

/// The clips and application context sent ahead of the user's prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultiSelectionContext {
    /// Clips in the order they were added, at most [`MAX_TRAY_CLIPS`].
    pub items: Vec<SelectedClip>,
    /// Description of the window under the cursor, when known.
    pub app_context: Option<String>,
}

impl MultiSelectionContext {
    /// Returns whether the tray holds no clips.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the number of clips currently in the tray.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Appends a clip, returning `false` when the tray is already full.
    pub fn add(&mut self, clip: SelectedClip) -> bool {
        if self.items.len() >= MAX_TRAY_CLIPS {
            return false;
        }
        self.items.push(clip);
        true
    }

    /// Removes the clip at `index`, if it exists.
    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    /// Drops every clip, leaving the application context untouched.
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

/// One message of a completion stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamChunk {
    /// A fragment of generated text.
    Token(String),
    /// The completion finished successfully.
    Done,
    /// The stream failed; the payload is already formatted for display.
    Error(String),
}

/// Reusable HTTP client that turns prompts into streamed tokens.
#[derive(Debug, Clone)]
pub struct AiClient {
    http: Client,
}

impl Default for AiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AiClient {
    /// Creates a client with a 60s total timeout and a 10s connect timeout.
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|err| {
                // A failure here means TLS could not be initialised; requests will
                // fail individually, which the overlay already reports per stream.
                eprintln!("[magic-cursor] HTTP client fell back to defaults: {err}");
                Client::new()
            });

        Self { http }
    }

    /// Starts a streaming request and returns its receiver and join handle.
    ///
    /// The first error is delivered as [`StreamChunk::Error`] and ends the stream,
    /// so callers only have to watch one channel. Dropping the receiver cancels
    /// the request at the next chunk boundary.
    pub fn start_stream(
        &self,
        config: ApiConfig,
        prompt: String,
        context: MultiSelectionContext,
    ) -> (Receiver<StreamChunk>, JoinHandle<()>) {
        let (tx, rx) = channel::<StreamChunk>(STREAM_CHANNEL_CAPACITY);
        let http = self.http.clone();

        let handle = tokio::spawn(async move {
            let result = match config.provider {
                Provider::Gemini => {
                    gemini::stream_gemini(&http, &config, &prompt, &context, &tx).await
                }
                Provider::Anthropic => {
                    anthropic::stream_anthropic(&http, &config, &prompt, &context, &tx).await
                }
                // Ollama, vLLM, OpenRouter, Groq, and DeepSeek all speak the
                // OpenAI chat-completions dialect.
                Provider::Openai | Provider::Ollama => {
                    openai::stream_openai(&http, &config, &prompt, &context, &tx).await
                }
            };

            if let Err(err) = result {
                let _ = tx.send(StreamChunk::Error(err.to_string())).await;
            }
        });

        (rx, handle)
    }
}

// The overlay runs the stream on the Tokio runtime from a worker thread, so both
// the client and its futures must stay `Send`.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<AiClient>();
};
