//! Streaming LLM access: request assembly, SSE decoding, and provider routing.

pub mod anthropic;
pub mod client;
pub mod error;
pub mod gemini;
pub mod openai;
pub mod sse;

pub use client::{AiClient, MultiSelectionContext, SelectedClip, StreamChunk, MAX_TRAY_CLIPS};
pub use error::StreamError;
