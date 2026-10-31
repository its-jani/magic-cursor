//! The error type returned by every provider stream.

use std::fmt;

use reqwest::StatusCode;

/// Something that stopped a completion stream before it finished.
#[derive(Debug)]
#[non_exhaustive]
pub enum StreamError {
    /// The request could not be sent, or the body failed mid-stream.
    Transport(reqwest::Error),
    /// The provider answered with a non-success HTTP status.
    Status {
        /// Human-readable provider name used in the message.
        provider: &'static str,
        /// Status code returned by the provider.
        status: StatusCode,
        /// Response body, which usually carries the provider's error message.
        body: String,
    },
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(err) => write!(f, "network request failed: {err}"),
            Self::Status {
                provider,
                status,
                body,
            } => {
                if body.is_empty() {
                    write!(f, "{provider} API error ({status})")
                } else {
                    write!(f, "{provider} API error ({status}): {body}")
                }
            }
        }
    }
}

impl std::error::Error for StreamError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(err) => Some(err),
            Self::Status { .. } => None,
        }
    }
}

impl From<reqwest::Error> for StreamError {
    fn from(err: reqwest::Error) -> Self {
        Self::Transport(err)
    }
}
