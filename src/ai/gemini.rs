//! Google Gemini `streamGenerateContent` support.

use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc::Sender;

use super::client::{MultiSelectionContext, SelectedClip, StreamChunk};
use super::error::StreamError;
use super::sse::{self, SsePayload};
use crate::config::ApiConfig;

/// Endpoint used when the configured value belongs to another provider.
const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Model used when none is configured.
const DEFAULT_MODEL: &str = "gemini-2.0-flash";

/// Endpoint the config file ships with, which is not valid for Gemini requests.
const DEFAULT_CONFIG_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

/// Prompt used when the user sends clips without typing a question.
const EMPTY_PROMPT: &str =
    "Analyze the selected screen items and provide a clear, concise summary.";

/// Streams a Gemini completion for `prompt` plus the selected clips.
///
/// Errors are returned for the caller to forward; text fragments are pushed to
/// `tx` as they arrive.
pub async fn stream_gemini(
    client: &Client,
    config: &ApiConfig,
    prompt: &str,
    context: &MultiSelectionContext,
    tx: &Sender<StreamChunk>,
) -> Result<(), StreamError> {
    let base_url = gemini_base_url(&config.endpoint);
    let model = model_name(&config.model);

    let url = format!(
        "{base_url}/models/{model}:streamGenerateContent?key={}&alt=sse",
        config.api_key
    );

    let payload = build_payload(config, prompt, context);

    let response = client.post(&url).json(&payload).send().await?;
    let response = sse::ensure_success(response, "Gemini").await?;

    sse::pump(response, tx, parse_chunk).await
}

/// Returns the Gemini base URL for a configured endpoint.
fn gemini_base_url(endpoint: &str) -> &str {
    if endpoint.is_empty() || endpoint == DEFAULT_CONFIG_ENDPOINT {
        GEMINI_BASE_URL
    } else {
        endpoint.trim_end_matches('/')
    }
}

/// Returns the configured model, or the Gemini default.
fn model_name(model: &str) -> &str {
    if model.is_empty() || model == "gpt-4o-mini" {
        DEFAULT_MODEL
    } else {
        model
    }
}

/// Assembles the `contents` and `generationConfig` body for one request.
fn build_payload(config: &ApiConfig, prompt: &str, context: &MultiSelectionContext) -> Value {
    let mut parts = Vec::new();

    if let Some(meta) = &context.app_context {
        parts.push(json!({ "text": format!("Target Application / Screen Context: {meta}") }));
    }

    for (index, clip) in context.items.iter().enumerate() {
        let number = index + 1;
        match clip {
            SelectedClip::Text { title, content } => parts.push(json!({
                "text": format!("--- Clip #{number} [{title}] ---\n{content}\n--------------------------")
            })),
            SelectedClip::Image {
                title, base64_png, ..
            } => {
                parts.push(json!({ "text": format!("[Image Clip #{number}: {title}]") }));
                parts.push(json!({
                    "inline_data": {
                        "mime_type": "image/png",
                        "data": base64_png
                    }
                }));
            }
        }
    }

    let final_prompt = if prompt.trim().is_empty() {
        EMPTY_PROMPT
    } else {
        prompt
    };
    parts.push(json!({ "text": final_prompt }));

    let mut payload = json!({
        "contents": [
            {
                "role": "user",
                "parts": parts
            }
        ],
        "generationConfig": {
            "temperature": config.temperature,
            "maxOutputTokens": config.max_tokens
        }
    });

    if !config.system_prompt.is_empty() {
        payload["system_instruction"] = json!({ "parts": [{ "text": config.system_prompt }] });
    }

    payload
}

/// Extracts generated text from one Gemini chunk.
fn parse_chunk(data: &str) -> SsePayload {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return SsePayload::Ignore;
    };

    let text = value
        .get("candidates")
        .and_then(|candidates| candidates.get(0))
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default();

    if text.is_empty() {
        SsePayload::Ignore
    } else {
        SsePayload::Text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_the_gemini_base_url() {
        assert_eq!(gemini_base_url(""), GEMINI_BASE_URL);
        assert_eq!(
            gemini_base_url(DEFAULT_CONFIG_ENDPOINT),
            GEMINI_BASE_URL,
            "an OpenAI endpoint must not leak into a Gemini request"
        );
        assert_eq!(
            gemini_base_url("https://proxy.local/v1beta/"),
            "https://proxy.local/v1beta"
        );
    }

    #[test]
    fn joins_every_text_part_of_a_chunk() {
        let payload = r#"{"candidates":[{"content":{"parts":[{"text":"he"},{"text":"llo"}]}}]}"#;

        match parse_chunk(payload) {
            SsePayload::Text(text) => assert_eq!(text, "hello"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn ignores_chunks_without_text() {
        assert!(matches!(parse_chunk("{}"), SsePayload::Ignore));
        assert!(matches!(parse_chunk("not json"), SsePayload::Ignore));
    }

    #[test]
    fn payload_includes_clips_and_system_instruction() {
        let config = ApiConfig::default();
        let context = MultiSelectionContext {
            items: vec![SelectedClip::Text {
                title: "Button".to_string(),
                content: "Save".to_string(),
            }],
            app_context: Some("notepad.exe".to_string()),
        };

        let payload = build_payload(&config, "", &context);
        let rendered = payload.to_string();

        assert!(rendered.contains("notepad.exe"));
        assert!(rendered.contains("Clip #1 [Button]"));
        assert!(rendered.contains("system_instruction"));
        assert!(rendered.contains(EMPTY_PROMPT));
    }
}
