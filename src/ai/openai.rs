//! OpenAI-compatible chat completions, which also covers Ollama, vLLM,
//! `OpenRouter`, Groq, and `DeepSeek`.

use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc::Sender;

use super::client::{MultiSelectionContext, SelectedClip, StreamChunk};
use super::error::StreamError;
use super::sse::{self, SsePayload};
use crate::config::ApiConfig;

/// Model used when none is configured.
const DEFAULT_MODEL: &str = "gpt-4o";

/// System prompt used when the configuration leaves it empty.
const DEFAULT_SYSTEM_PROMPT: &str = "You are MagicCursor, a high-speed desktop AI assistant \
     operating under the user's cursor. Provide concise, clear, and actionable responses.";

/// Prompt used when the user sends clips without typing a question.
const EMPTY_PROMPT: &str = "Analyze the selected items and provide a clear, concise summary.";

/// The sentinel payload that ends an `OpenAI` stream.
const DONE_SENTINEL: &str = "[DONE]";

/// Streams an OpenAI-compatible completion for `prompt` plus the selected clips.
///
/// Errors are returned for the caller to forward; text fragments are pushed to
/// `tx` as they arrive.
pub async fn stream_openai(
    client: &Client,
    config: &ApiConfig,
    prompt: &str,
    context: &MultiSelectionContext,
    tx: &Sender<StreamChunk>,
) -> Result<(), StreamError> {
    let payload = build_payload(config, prompt, context);

    let mut request = client.post(&config.endpoint).json(&payload);
    if !config.api_key.trim().is_empty() {
        request = request.bearer_auth(&config.api_key);
    }

    let response = request.send().await?;
    let response = sse::ensure_success(response, "OpenAI").await?;

    sse::pump(response, tx, parse_chunk).await
}

/// Assembles the `messages` body for one request.
fn build_payload(config: &ApiConfig, prompt: &str, context: &MultiSelectionContext) -> Value {
    let system_text = if config.system_prompt.is_empty() {
        DEFAULT_SYSTEM_PROMPT
    } else {
        &config.system_prompt
    };

    let mut user_parts = Vec::new();

    if let Some(meta) = &context.app_context {
        user_parts.push(json!({
            "type": "text",
            "text": format!("Target Application / Screen Context: {meta}")
        }));
    }

    for (index, clip) in context.items.iter().enumerate() {
        let number = index + 1;
        match clip {
            SelectedClip::Text { title, content } => user_parts.push(json!({
                "type": "text",
                "text": format!("--- Clip #{number} [{title}] ---\n{content}\n--------------------------")
            })),
            SelectedClip::Image {
                title, base64_png, ..
            } => {
                user_parts.push(json!({
                    "type": "text",
                    "text": format!("[Image Clip #{number}: {title}]")
                }));
                user_parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:image/png;base64,{base64_png}") }
                }));
            }
        }
    }

    let final_prompt = if prompt.trim().is_empty() {
        EMPTY_PROMPT
    } else {
        prompt
    };
    user_parts.push(json!({ "type": "text", "text": final_prompt }));

    json!({
        "model": model_name(&config.model),
        "messages": [
            { "role": "system", "content": system_text },
            { "role": "user", "content": user_parts }
        ],
        "stream": true,
        "temperature": config.temperature,
        "max_tokens": config.max_tokens
    })
}

/// Returns the configured model, or the `OpenAI` default.
fn model_name(model: &str) -> &str {
    if model.is_empty() {
        DEFAULT_MODEL
    } else {
        model
    }
}

/// Extracts the assistant delta from one `OpenAI` chunk.
fn parse_chunk(data: &str) -> SsePayload {
    if data == DONE_SENTINEL {
        return SsePayload::Done;
    }

    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return SsePayload::Ignore;
    };

    let text = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    if text.is_empty() {
        SsePayload::Ignore
    } else {
        SsePayload::Text(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_sentinel_ends_the_stream() {
        assert!(matches!(parse_chunk(DONE_SENTINEL), SsePayload::Done));
    }

    #[test]
    fn ignores_role_only_deltas() {
        let payload = r#"{"choices":[{"delta":{"role":"assistant"}}]}"#;

        assert!(matches!(parse_chunk(payload), SsePayload::Ignore));
    }

    #[test]
    fn extracts_content_deltas() {
        let payload = r#"{"choices":[{"delta":{"content":"hi"}}]}"#;

        match parse_chunk(payload) {
            SsePayload::Text(text) => assert_eq!(text, "hi"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn empty_model_falls_back_to_default() {
        assert_eq!(model_name(""), DEFAULT_MODEL);
        assert_eq!(model_name("qwen2.5-vl"), "qwen2.5-vl");
    }

    #[test]
    fn images_use_a_data_url() {
        let config = ApiConfig::default();
        let context = MultiSelectionContext {
            items: vec![SelectedClip::Image {
                title: "Crop".to_string(),
                width: 4,
                height: 4,
                base64_png: "AAAA".to_string(),
            }],
            app_context: None,
        };

        let rendered = build_payload(&config, "", &context).to_string();

        assert!(rendered.contains("data:image/png;base64,AAAA"));
    }
}
