//! Server-Sent Events decoding shared by every provider stream.
//!
//! The three providers differ only in their JSON envelope; the wire framing
//! (`data:` lines separated by newlines) is identical, so it lives here. Bytes
//! are buffered until a newline arrives, which keeps multi-byte UTF-8 characters
//! intact even when the HTTP client splits them across chunks.

use futures_util::StreamExt;
use reqwest::Response;
use tokio::sync::mpsc::Sender;

use super::client::StreamChunk;
use super::error::StreamError;

/// What a provider decided to do with one `data:` payload.
#[derive(Debug)]
pub enum SsePayload {
    /// A fragment of generated text to forward to the UI.
    Text(String),
    /// The provider signalled the end of the message.
    Done,
    /// The payload carries no user-visible text (role deltas, usage, pings).
    Ignore,
}

/// Incremental decoder that turns raw SSE bytes into `data:` payloads.
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    /// Creates an empty decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends bytes and returns every payload completed by them.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(bytes);

        let mut payloads = Vec::new();
        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            // Drain through the newline so the buffer never re-scans seen bytes.
            let line: Vec<u8> = self.buffer.drain(..=newline).collect();
            if let Some(payload) = parse_data_line(&line) {
                payloads.push(payload);
            }
        }
        payloads
    }

    /// Returns the final payload of a stream that ended without a trailing newline.
    pub fn finish(&mut self) -> Option<String> {
        let rest: Vec<u8> = std::mem::take(&mut self.buffer);
        parse_data_line(&rest)
    }
}

/// Extracts the payload of a `data:` line, ignoring comments and named events.
fn parse_data_line(line: &[u8]) -> Option<String> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let rest = line.strip_prefix(b"data:")?;
    // A single space after the colon is part of the framing, not of the payload.
    let rest = rest.strip_prefix(b" ").unwrap_or(rest);

    Some(String::from_utf8_lossy(rest).into_owned())
}

/// Returns the response when it is a success, or a [`StreamError::Status`].
pub async fn ensure_success(
    response: Response,
    provider: &'static str,
) -> Result<Response, StreamError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let body = response.text().await.unwrap_or_default();
    Err(StreamError::Status {
        provider,
        status,
        body,
    })
}

/// Streams a response body, forwarding text fragments to the UI channel.
///
/// The decoder stops as soon as the receiver is dropped, so closing the overlay
/// releases the connection instead of draining the whole completion.
pub async fn pump<F>(
    response: Response,
    tx: &Sender<StreamChunk>,
    mut on_data: F,
) -> Result<(), StreamError>
where
    F: FnMut(&str) -> SsePayload,
{
    let mut body = response.bytes_stream();
    let mut decoder = SseDecoder::new();

    while let Some(chunk) = body.next().await {
        let bytes = chunk.map_err(StreamError::Transport)?;

        for payload in decoder.push(&bytes) {
            if !forward(&payload, tx, &mut on_data).await {
                return Ok(());
            }
        }
    }

    if let Some(payload) = decoder.finish() {
        let _ = forward(&payload, tx, &mut on_data).await;
    }

    let _ = tx.send(StreamChunk::Done).await;
    Ok(())
}

/// Handles one payload and reports whether the stream should keep running.
async fn forward<F>(payload: &str, tx: &Sender<StreamChunk>, on_data: &mut F) -> bool
where
    F: FnMut(&str) -> SsePayload,
{
    match on_data(payload) {
        SsePayload::Text(text) => {
            if tx.send(StreamChunk::Token(text)).await.is_err() {
                return false;
            }
            true
        }
        SsePayload::Done => {
            let _ = tx.send(StreamChunk::Done).await;
            false
        }
        SsePayload::Ignore => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_payload_split_across_chunks() {
        let mut decoder = SseDecoder::new();

        assert!(decoder.push(b"data: {\"a\":1}").is_empty());
        assert_eq!(decoder.push(b"\n"), vec![r#"{"a":1}"#.to_string()]);
    }

    #[test]
    fn keeps_multibyte_characters_whole() {
        let mut decoder = SseDecoder::new();
        let payload = "🪄".as_bytes();

        // Split the frame in the middle of the four-byte character.
        let mut head = b"data: ".to_vec();
        head.extend_from_slice(&payload[..2]);
        assert!(decoder.push(&head).is_empty());

        let mut tail = payload[2..].to_vec();
        tail.push(b'\n');
        assert_eq!(decoder.push(&tail), vec!["🪄".to_string()]);
    }

    #[test]
    fn ignores_comments_events_and_blank_lines() {
        let mut decoder = SseDecoder::new();

        assert!(decoder.push(b": keep-alive\n\nevent: ping\n").is_empty());
    }

    #[test]
    fn handles_crlf_line_endings() {
        let mut decoder = SseDecoder::new();

        assert_eq!(decoder.push(b"data: ok\r\n"), vec!["ok".to_string()]);
    }

    #[test]
    fn finish_returns_trailing_payload_without_newline() {
        let mut decoder = SseDecoder::new();
        let _ = decoder.push(b"data: tail");

        assert_eq!(decoder.finish(), Some("tail".to_string()));
        assert_eq!(decoder.finish(), None);
    }
}
