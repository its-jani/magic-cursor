use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::task::JoinHandle;

use super::anthropic::stream_anthropic;
use super::gemini::stream_gemini;
use super::openai::stream_openai;
use crate::config::ApiConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SelectedClip {
    Text {
        title: String,
        content: String,
    },
    Image {
        title: String,
        width: u32,
        height: u32,
        base64_png: String,
    },
}

impl SelectedClip {
    pub fn title(&self) -> &str {
        match self {
            SelectedClip::Text { title, .. } => title,
            SelectedClip::Image { title, .. } => title,
        }
    }

    pub fn summary(&self) -> String {
        match self {
            SelectedClip::Text { title, content } => {
                format!("Text: {} ({} chars)", title, content.chars().count())
            }
            SelectedClip::Image {
                title,
                width,
                height,
                ..
            } => {
                format!("Image: {} ({}x{} px)", title, width, height)
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MultiSelectionContext {
    pub items: Vec<SelectedClip>,
    pub app_context: Option<String>,
}

impl MultiSelectionContext {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn add(&mut self, clip: SelectedClip) -> bool {
        if self.items.len() < 10 {
            self.items.push(clip);
            true
        } else {
            false
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[derive(Debug, Clone)]
pub enum StreamChunk {
    Token(String),
    Done,
    Error(String),
}

#[derive(Clone)]
pub struct AiClient {
    client: Client,
}

impl Default for AiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl AiClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        Self { client }
    }

    /// Spawns an asynchronous streaming request across Gemini, Claude, OpenAI, or local Ollama/vLLM endpoints.
    pub fn start_stream(
        &self,
        config: ApiConfig,
        prompt: String,
        context: MultiSelectionContext,
    ) -> (Receiver<StreamChunk>, JoinHandle<()>) {
        let (tx, rx) = channel::<StreamChunk>(64);
        let client = self.client.clone();

        let handle = tokio::spawn(async move {
            let provider = config.provider.to_lowercase();
            let result = match provider.as_str() {
                "gemini" | "google" => {
                    stream_gemini(&client, &config, &prompt, &context, tx.clone()).await
                }
                "anthropic" | "claude" => {
                    stream_anthropic(&client, &config, &prompt, &context, tx.clone()).await
                }
                _ => {
                    // Default to OpenAI Compatible (OpenAI gpt-4o, Ollama qwen2.5-vl/llava, vLLM, OpenRouter, Groq, DeepSeek)
                    stream_openai(&client, &config, &prompt, &context, tx.clone()).await
                }
            };

            if let Err(err) = result {
                let _ = tx.send(StreamChunk::Error(err)).await;
            }
        });

        (rx, handle)
    }
}
