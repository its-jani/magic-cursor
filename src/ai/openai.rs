use futures_util::StreamExt;
use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc::Sender;

use super::client::StreamChunk;
use crate::config::ApiConfig;
use crate::context::CapturedContext;

pub async fn stream_openai(
    client: &Client,
    config: &ApiConfig,
    prompt: &str,
    context: &CapturedContext,
    tx: Sender<StreamChunk>,
) -> Result<(), String> {
    let mut messages = Vec::new();

    // System prompt
    if !config.system_prompt.is_empty() {
        messages.push(json!({
            "role": "system",
            "content": config.system_prompt
        }));
    }

    // User message with context
    match context {
        CapturedContext::Empty => {
            messages.push(json!({
                "role": "user",
                "content": prompt
            }));
        }
        CapturedContext::Text(captured_text) => {
            let combined = format!(
                "--- ATTACHED CLIPBOARD/TEXT CONTEXT ---\n{}\n----------------------------------------\n\nUser Request: {}",
                captured_text, prompt
            );
            messages.push(json!({
                "role": "user",
                "content": combined
            }));
        }
        CapturedContext::Image(img) => {
            let data_url = format!("data:image/png;base64,{}", img.base64_png);
            messages.push(json!({
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": if prompt.trim().is_empty() { "Analyze the attached screen region and summarize what is visible or answer any implicit question." } else { prompt }
                    },
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": data_url
                        }
                    }
                ]
            }));
        }
    }

    let payload = json!({
        "model": config.model,
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
