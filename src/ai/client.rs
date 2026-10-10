use reqwest::Client;
use std::time::Duration;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::task::JoinHandle;

use super::anthropic::stream_anthropic;
use super::gemini::stream_gemini;
use super::openai::stream_openai;
use crate::config::ApiConfig;
use crate::context::CapturedContext;

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

    /// Spawns an asynchronous streaming request returning a channel receiver and task handle for aborting.
    pub fn start_stream(
        &self,
        config: ApiConfig,
        prompt: String,
        context: CapturedContext,
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
                    // Default to OpenAI compatible (covers OpenAI, Ollama, LM Studio, Groq, DeepSeek, vLLM, etc.)
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
