use futures_util::StreamExt;
use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc::Sender;

use super::client::{MultiSelectionContext, SelectedClip, StreamChunk};
use crate::config::ApiConfig;

pub async fn stream_openai(
    client: &Client,
    config: &ApiConfig,
    prompt: &str,
    context: &MultiSelectionContext,
    tx: Sender<StreamChunk>,
) -> Result<(), String> {
    let mut messages = Vec::new();

    // System prompt
    let system_text = if config.system_prompt.is_empty() {
        "You are MagicCursor, a high-speed desktop AI assistant operating under the user's cursor. Provide concise, clear, and actionable responses."
    } else {
        &config.system_prompt
    };

    messages.push(json!({
        "role": "system",
        "content": system_text
    }));

    // Build user content parts
    let mut user_parts = Vec::new();

    if let Some(ref meta) = context.app_context {
        user_parts.push(json!({
            "type": "text",
            "text": format!("Target Application / Screen Context: {}", meta)
        }));
    }

    for (idx, clip) in context.items.iter().enumerate() {
        match clip {
            SelectedClip::Text { title, content } => {
                user_parts.push(json!({
                    "type": "text",
                    "text": format!("--- Clip #{} [{}] ---\n{}\n--------------------------", idx + 1, title, content)
                }));
            }
            SelectedClip::Image {
                title, base64_png, ..
            } => {
                let data_url = format!("data:image/png;base64,{}", base64_png);
                user_parts.push(json!({
                    "type": "text",
                    "text": format!("[Image Clip #{}: {}]", idx + 1, title)
                }));
                user_parts.push(json!({
                    "type": "image_url",
                    "image_url": {
                        "url": data_url
                    }
                }));
            }
        }
    }

    let final_prompt = if prompt.trim().is_empty() {
        "Analyze the selected items and provide a clear, concise summary."
    } else {
        prompt
    };

    user_parts.push(json!({
        "type": "text",
        "text": final_prompt
    }));

    messages.push(json!({
        "role": "user",
        "content": user_parts
    }));

    let model = if config.model.is_empty() {
        "gpt-4o"
    } else {
        &config.model
    };

    let payload = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "temperature": config.temperature,
        "max_tokens": config.max_tokens
    });

    let mut req = client.post(&config.endpoint).json(&payload);
    if !config.api_key.trim().is_empty() {
        req = req.bearer_auth(&config.api_key);
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("API Error ({}): {}", status, err_text));
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
                if data == "[DONE]" {
                    let _ = tx.send(StreamChunk::Done).await;
                    return Ok(());
                }

                if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(choices) = val.get("choices").and_then(|c| c.as_array()) {
                        if let Some(first) = choices.first() {
                            if let Some(delta) = first.get("delta") {
                                if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                    if !content.is_empty() {
                                        let _ = tx.send(StreamChunk::Token(content.to_string())).await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let _ = tx.send(StreamChunk::Done).await;
    Ok(())
}
