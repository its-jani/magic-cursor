use futures_util::StreamExt;
use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc::Sender;

use super::client::{MultiSelectionContext, SelectedClip, StreamChunk};
use crate::config::ApiConfig;

pub async fn stream_anthropic(
    client: &Client,
    config: &ApiConfig,
    prompt: &str,
    context: &MultiSelectionContext,
    tx: Sender<StreamChunk>,
) -> Result<(), String> {
    let endpoint = if config.endpoint.is_empty()
        || config.endpoint == "https://api.openai.com/v1/chat/completions"
    {
        "https://api.anthropic.com/v1/messages"
    } else {
        &config.endpoint
    };

    let model = if config.model.is_empty() {
        "claude-3-5-sonnet-20241022"
    } else {
        &config.model
    };

    let mut user_content = Vec::new();

    if let Some(ref meta) = context.app_context {
        user_content.push(json!({
            "type": "text",
            "text": format!("Target Application / Screen Context: {}", meta)
        }));
    }

    for (idx, clip) in context.items.iter().enumerate() {
        match clip {
            SelectedClip::Text { title, content } => {
                user_content.push(json!({
                    "type": "text",
                    "text": format!("--- Clip #{} [{}] ---\n{}\n--------------------------", idx + 1, title, content)
                }));
            }
            SelectedClip::Image {
                title, base64_png, ..
            } => {
                user_content.push(json!({
                    "type": "text",
                    "text": format!("[Image Clip #{}: {}]", idx + 1, title)
                }));
                user_content.push(json!({
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
        "Analyze the attached items and provide a concise response."
    } else {
        prompt
    };

    user_content.push(json!({
        "type": "text",
        "text": final_prompt
    }));

    let mut payload = json!({
        "model": model,
        "max_tokens": config.max_tokens,
        "temperature": config.temperature,
        "stream": true,
        "messages": [
            {
                "role": "user",
                "content": user_content
            }
        ]
    });

    if !config.system_prompt.is_empty() {
        payload["system"] = json!(config.system_prompt);
    }

    let response = client
        .post(endpoint)
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Anthropic API Error ({}): {}", status, err_text));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(item) = stream.next().await {
        let bytes = item.map_err(|e| format!("Stream error: {}", e))?;
        let text = String::from_utf8_lossy(&bytes);
        buffer.push_str(&text);

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_string();
            buffer = buffer[line_end + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            if line.starts_with("data: ") {
                let data = &line[6..];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                    let event_type = val.get("type").and_then(|t| t.as_str()).unwrap_or_default();
                    if event_type == "content_block_delta" {
                        if let Some(delta) = val.get("delta") {
                            if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    let _ = tx.send(StreamChunk::Token(text.to_string())).await;
                                }
                            }
                        }
                    } else if event_type == "message_stop" {
                        let _ = tx.send(StreamChunk::Done).await;
                        return Ok(());
                    }
                }
            }
        }
    }

    let _ = tx.send(StreamChunk::Done).await;
    Ok(())
}
