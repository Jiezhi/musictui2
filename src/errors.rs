//! Typed errors for module boundaries.
//!
//! Each I/O module exposes its own error enum so callers can match on
//! actionable variants (rate-limiting, auth failures) instead of inspecting
//! `Box<dyn Error>` strings. Application code at the CLI/TUI boundary
//! collapses these back into `Box<dyn Error>` via the blanket conversions.

use std::path::PathBuf;

use reqwest::StatusCode;
use thiserror::Error;

/// Errors raised when talking to the GitHub Contents API.
#[derive(Debug, Error)]
pub enum GitHubError {
    /// GitHub returned 403 with a "rate limit" message.
    #[error("GitHub API rate limit exceeded. Set GITHUB_TOKEN to increase limit")]
    RateLimited { reset_at: Option<u64> },

    /// GitHub returned 403 without a rate-limit hint (private repo, missing scope, etc).
    #[error("GitHub API access forbidden. Please check your access or set GITHUB_TOKEN")]
    Forbidden,

    /// A non-success HTTP status that is not specifically handled above.
    #[error("GitHub API error: HTTP {status}")]
    Status { status: StatusCode },

    /// Underlying transport-level error from `reqwest`.
    #[error("GitHub HTTP transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// Response body could not be parsed as the expected JSON shape.
    #[error("Failed to decode GitHub response: {0}")]
    Decode(String),

    /// Catch-all for download failures and other text-only errors.
    #[error("{0}")]
    Other(String),
}

impl GitHubError {
    /// Returns true when the caller should back off and retry later instead of
    /// surfacing a hard failure (rate limit / 429 / 5xx).
    #[allow(dead_code)]
    pub fn is_retryable(&self) -> bool {
        match self {
            GitHubError::RateLimited { .. } => true,
            GitHubError::Status { status } => {
                *status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
            }
            _ => false,
        }
    }
}

/// Errors raised when talking to a WebDAV collection.
///
/// Currently exposed for future use — the WebDAV module today returns
/// `Box<dyn Error>` at the public boundary. Migrating it is tracked
/// separately; the variants are defined here so the migration is a
/// one-spot rename.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum WebDavError {
    #[error("WebDAV authentication failed (HTTP {status})")]
    AuthFailed { status: StatusCode },

    #[error("WebDAV request failed: HTTP {status}")]
    Status { status: StatusCode },

    #[error("WebDAV HTTP transport error: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("Failed to parse WebDAV URL: {0}")]
    UrlParse(String),

    #[error("Invalid WebDAV PROPFIND request: {0}")]
    BadRequest(String),

    #[error("{0}")]
    Other(String),
}

impl WebDavError {
    #[allow(dead_code)]
    pub fn from_status(status: StatusCode) -> Self {
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            WebDavError::AuthFailed { status }
        } else {
            WebDavError::Status { status }
        }
    }
}

/// Errors raised by the on-disk file cache.
///
/// Currently exposed for future use — the cache module today returns
/// `std::io::Error` directly. Defined here so callers can match on the
/// `path`-bearing variant when we migrate it.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("Cache I/O error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Cache I/O error: {0}")]
    BareIo(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limited_is_retryable() {
        let err = GitHubError::RateLimited { reset_at: None };
        assert!(err.is_retryable());
        assert!(err.to_string().contains("rate limit"));
    }

    #[test]
    fn server_errors_are_retryable_but_not_forbidden() {
        assert!(GitHubError::Status {
            status: StatusCode::INTERNAL_SERVER_ERROR,
        }
        .is_retryable());
        assert!(GitHubError::Status {
            status: StatusCode::TOO_MANY_REQUESTS,
        }
        .is_retryable());
        assert!(!GitHubError::Status {
            status: StatusCode::NOT_FOUND,
        }
        .is_retryable());
        assert!(!GitHubError::Forbidden.is_retryable());
    }

    #[test]
    fn forbidden_message_does_not_leak_token() {
        let err = GitHubError::Forbidden;
        let text = err.to_string();
        assert!(text.contains("GITHUB_TOKEN"));
        // The variant must never embed a secret value — only the env var name.
        assert!(!text.contains("ghp_"));
        assert!(!text.contains("github_pat_"));
    }

    #[test]
    fn webdav_from_status_classifies_auth_vs_other() {
        assert!(matches!(
            WebDavError::from_status(StatusCode::UNAUTHORIZED),
            WebDavError::AuthFailed { .. }
        ));
        assert!(matches!(
            WebDavError::from_status(StatusCode::FORBIDDEN),
            WebDavError::AuthFailed { .. }
        ));
        assert!(matches!(
            WebDavError::from_status(StatusCode::INTERNAL_SERVER_ERROR),
            WebDavError::Status { .. }
        ));
    }

    #[test]
    fn cache_error_carries_path_context() {
        let path = PathBuf::from("/tmp/musictui2/abc");
        let err = CacheError::Io {
            path: path.clone(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
        };
        let msg = err.to_string();
        assert!(msg.contains("musictui2"));
        assert!(msg.contains("missing"));
    }
}
