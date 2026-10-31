//! Anthropic Messages API streaming support.

use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc::Sender;

use super::client::{MultiSelectionContext, SelectedClip, StreamChunk};
use super::error::StreamError;
use super::sse::{self, SsePayload};
use crate::config::ApiConfig;

/// Endpoint used when the configured value belongs to another provider.
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";

/// Endpoint the config file ships with, which is not valid for Anthropic requests.
const DEFAULT_CONFIG_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

/// Model used when none is configured.
const DEFAULT_MODEL: &str = "claude-3-5-sonnet-20241022";

/// API version header required by the Messages API.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Prompt used when the user sends clips without typing a question.
const EMPTY_PROMPT: &str = "Analyze the attached items and provide a concise response.";

/// Streams an Anthropic completion for `prompt` plus the selected clips.
///
/// Errors are returned for the caller to forward; text fragments are pushed to
/// `tx` as they arrive.
pub async fn stream_anthropic(
    client: &Client,
    config: &ApiConfig,
    prompt: &str,
    context: &MultiSelectionContext,
    tx: &Sender<StreamChunk>,
) -> Result<(), StreamError> {
    let payload = build_payload(config, prompt, context);

    let response = client
        .post(endpoint(&config.endpoint))
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&payload)
        .send()
        .await?;

    let response = sse::ensure_success(response, "Anthropic").await?;

    sse::pump(response, tx, parse_chunk).await
}

/// Returns the endpoint to post to for a configured value.
fn endpoint(configured: &str) -> &str {
    if configured.is_empty() || configured == DEFAULT_CONFIG_ENDPOINT {
        ANTHROPIC_ENDPOINT
    } else {
        configured
    }
}

/// Assembles the `messages` body for one request.
fn build_payload(config: &ApiConfig, prompt: &str, context: &MultiSelectionContext) -> Value {
    let mut content_parts = Vec::new();

    if let Some(meta) = &context.app_context {
        content_parts.push(json!({
            "type": "text",
            "text": format!("Target Application / Screen Context: {meta}")
        }));
    }

    for (index, clip) in context.items.iter().enumerate() {
        let number = index + 1;
        match clip {
            SelectedClip::Text { title, content } => content_parts.push(json!({
                "type": "text",
                "text": format!("--- Clip #{number} [{title}] ---\n{content}\n--------------------------")
            })),
            SelectedClip::Image {
                title, base64_png, ..
            } => {
                content_parts.push(json!({
                    "type": "text",
                    "text": format!("[Image Clip #{number}: {title}]")
                }));
                content_parts.push(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
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
    content_parts.push(json!({ "type": "text", "text": final_prompt }));

    let mut payload = json!({
        "model": model_name(&config.model),
        "max_tokens": config.max_tokens,
        "temperature": config.temperature,
        "stream": true,
        "messages": [{ "role": "user", "content": content_parts }]
    });

    if !config.system_prompt.is_empty() {
        payload["system"] = json!(config.system_prompt);
    }

    payload
}

/// Returns the configured model, or the Anthropic default.
fn model_name(model: &str) -> &str {
    if model.is_empty() {
        DEFAULT_MODEL
    } else {
        model
    }
}

/// Extracts the text delta from one Anthropic event.
fn parse_chunk(data: &str) -> SsePayload {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return SsePayload::Ignore;
    };

    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match event_type {
        "message_stop" => SsePayload::Done,
        "content_block_delta" => {
            let text = value
                .get("delta")
                .and_then(|delta| delta.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default();

            if text.is_empty() {
                SsePayload::Ignore
            } else {
                SsePayload::Text(text.to_string())
            }
        }
        _ => SsePayload::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_stop_ends_the_stream() {
        assert!(matches!(
            parse_chunk(r#"{"type":"message_stop"}"#),
            SsePayload::Done
        ));
    }

    #[test]
    fn content_delta_yields_text() {
        let payload = r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}"#;

        match parse_chunk(payload) {
            SsePayload::Text(text) => assert_eq!(text, "hi"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn other_event_types_are_ignored() {
        assert!(matches!(
            parse_chunk(r#"{"type":"message_start"}"#),
            SsePayload::Ignore
        ));
    }

    #[test]
    fn endpoint_falls_back_when_config_is_foreign() {
        assert_eq!(endpoint(""), ANTHROPIC_ENDPOINT);
        assert_eq!(endpoint(DEFAULT_CONFIG_ENDPOINT), ANTHROPIC_ENDPOINT);
        assert_eq!(
            endpoint("https://proxy.local/messages"),
            "https://proxy.local/messages"
        );
    }
}
