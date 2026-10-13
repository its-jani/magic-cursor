use futures_util::StreamExt;
use reqwest::Client;
use serde_json::json;
use tokio::sync::mpsc::Sender;

use super::client::{MultiSelectionContext, SelectedClip, StreamChunk};
use crate::config::ApiConfig;

pub async fn stream_gemini(
    client: &Client,
    config: &ApiConfig,
    prompt: &str,
    context: &MultiSelectionContext,
    tx: Sender<StreamChunk>,
) -> Result<(), String> {
    let base_url = if config.endpoint.is_empty()
        || config.endpoint == "https://api.openai.com/v1/chat/completions"
    {
        "https://generativelanguage.googleapis.com/v1beta"
    } else {
        config.endpoint.trim_end_matches('/')
    };

    let model = if config.model.is_empty() || config.model == "gpt-4o-mini" {
        "gemini-2.0-flash"
    } else {
        &config.model
    };

    let url = format!(
        "{}/models/{}:streamGenerateContent?key={}&alt=sse",
        base_url, model, config.api_key
    );

    let mut parts = Vec::new();

    // Attach system/process metadata if present
    if let Some(ref meta) = context.app_context {
        parts.push(json!({
            "text": format!("Target Application / Screen Context: {}", meta)
        }));
    }

    // Attach multi-selection clips
    for (idx, clip) in context.items.iter().enumerate() {
        match clip {
            SelectedClip::Text { title, content } => {
                parts.push(json!({
                    "text": format!("--- Clip #{} [{}] ---\n{}\n--------------------------", idx + 1, title, content)
                }));
            }
            SelectedClip::Image {
                title, base64_png, ..
            } => {
                parts.push(json!({
                    "text": format!("[Image Clip #{}: {}]", idx + 1, title)
                }));
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
        "Analyze the selected screen items and provide a clear, concise summary."
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
        payload["system_instruction"] = json!({
            "parts": [
                { "text": config.system_prompt }
            ]
        });
    }

    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Network request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let err_text = response.text().await.unwrap_or_default();
        return Err(format!("Gemini API Error ({}): {}", status, err_text));
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
                    if let Some(candidates) = val.get("candidates").and_then(|c| c.as_array()) {
                        if let Some(first) = candidates.first() {
                            if let Some(content) = first.get("content") {
                                if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                                    for part in parts {
                                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                            if !text.is_empty() {
                                                let _ = tx.send(StreamChunk::Token(text.to_string())).await;
                                            }
                                        }
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
