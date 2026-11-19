//! Typed errors for a Jev call. A variant holds a status and at most the `error_type` of a known shape (api__errors.md,
//! and the live check of 2026-11-19 on #911); it never holds any part of a response body, because a 422 body echoes the
//! request (message text included). Only the bodies of 400, 401 and 403 are read at all, and only for `error_type`.

use super::request::RequestError;
use serde_json::Value;
use std::fmt;

/// The `error_type` values the API was seen to send, and the only ones ever reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorType {
    Authentication,
    ApiUsage,
    MaxTokensExceeded,
}

impl ErrorType {
    pub fn name(self) -> &'static str {
        match self {
            ErrorType::Authentication => "authentication_error",
            ErrorType::ApiUsage => "api_usage_error",
            ErrorType::MaxTokensExceeded => "max_tokens_exceeded",
        }
    }

    fn parse(s: &str) -> Option<ErrorType> {
        [
            ErrorType::Authentication,
            ErrorType::ApiUsage,
            ErrorType::MaxTokensExceeded,
        ]
        .into_iter()
        .find(|t| t.name() == s)
    }
}

/// The two headers that say how long to wait, as the raw text the server sent (parsed by the retry policy).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RetryHeaders {
    pub retry_after: Option<String>,
    pub retry_after_ms: Option<String>,
}

/// How a transport failed. It carries a response body only so [`http_error`] can read the `error_type` of the known
/// shapes; its Debug shows the status and never the body.
#[derive(Clone, PartialEq)]
pub enum JevTransportError {
    /// No answer at all (connect, TLS, timeout). Carries no detail.
    Network,
    /// A non-success HTTP answer.
    Status {
        status: u16,
        body: String,
        headers: RetryHeaders,
    },
}

impl fmt::Debug for JevTransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JevTransportError::Network => f.write_str("Network"),
            JevTransportError::Status { status, .. } => {
                write!(f, "Status {{ status: {status}, body: <withheld> }}")
            }
        }
    }
}

impl JevTransportError {
    /// A status error with no retry headers.
    pub fn status(status: u16, body: &str) -> JevTransportError {
        JevTransportError::Status {
            status,
            body: body.into(),
            headers: RetryHeaders::default(),
        }
    }
}

/// Why a call produced no usable answer. No variant ever holds the key, the state or any part of a reply body.
#[derive(Debug, Clone, PartialEq)]
pub enum JevError {
    NoKey,
    /// The request was refused locally; nothing was sent.
    Refused(RequestError),
    /// No answer from the network.
    Unreachable,
    /// 400: the body breaks a request rule or a model rule.
    BadRequest {
        error_type: Option<ErrorType>,
    },
    /// 400 `max_tokens_exceeded`: longer than the model's context. Never retried; send less.
    ContextExceeded,
    /// 401: a missing or invalid key.
    Unauthorized {
        error_type: Option<ErrorType>,
    },
    /// 403 (seen live for a bad key).
    Forbidden {
        error_type: Option<ErrorType>,
    },
    /// 413: the body is over 32 MiB.
    PayloadTooLarge,
    /// 415: an unsupported Content-Encoding or more than one layer.
    UnsupportedMediaType,
    /// 422: validation failed (its body echoes the request, so it is never read).
    Unprocessable,
    /// 429: over the rate limit.
    RateLimited,
    /// 529: temporarily overloaded.
    Overloaded,
    /// Any other HTTP status.
    Http {
        status: u16,
    },
    Malformed(&'static str),
}

impl JevError {
    pub fn status(&self) -> Option<u16> {
        Some(match self {
            JevError::BadRequest { .. } | JevError::ContextExceeded => 400,
            JevError::Unauthorized { .. } => 401,
            JevError::Forbidden { .. } => 403,
            JevError::PayloadTooLarge => 413,
            JevError::UnsupportedMediaType => 415,
            JevError::Unprocessable => 422,
            JevError::RateLimited => 429,
            JevError::Overloaded => 529,
            JevError::Http { status } => *status,
            _ => return None,
        })
    }

    pub fn error_type(&self) -> Option<ErrorType> {
        match self {
            JevError::BadRequest { error_type }
            | JevError::Unauthorized { error_type }
            | JevError::Forbidden { error_type } => *error_type,
            JevError::ContextExceeded => Some(ErrorType::MaxTokensExceeded),
            _ => None,
        }
    }

    /// Only a rate limit or an overload is worth another try (api__errors.md: back off and retry).
    pub fn is_retryable(&self) -> bool {
        matches!(self, JevError::RateLimited | JevError::Overloaded)
    }
}

impl fmt::Display for JevError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JevError::NoKey => write!(f, "no {} in the environment", super::KEY_ENV),
            JevError::Refused(why) => write!(f, "the request was not sent: {why}"),
            JevError::Unreachable => write!(f, "Jev could not be reached"),
            JevError::RateLimited => write!(f, "Jev answered HTTP 429 (rate limited)"),
            JevError::Overloaded => write!(f, "Jev answered HTTP 529 (overloaded)"),
            JevError::Malformed(why) => write!(f, "the Jev reply was refused: {why}"),
            other => {
                let status = other.status().unwrap_or(0);
                match other.error_type() {
                    Some(t) => write!(f, "Jev answered HTTP {status} ({})", t.name()),
                    None => write!(f, "Jev answered HTTP {status}"),
                }
            }
        }
    }
}

impl std::error::Error for JevError {}

/// Turn an HTTP error into a [`JevError`]. Only the status is reported, plus the `error_type` for 400, 401 and 403 when
/// the body is `{"detail": {"error_type": ...}}` with one of the known values. The body of any other status (a 422
/// echoes the whole request) is never read, and no body is ever reported.
pub fn http_error(status: u16, body: &str) -> JevError {
    let error_type = if matches!(status, 400 | 401 | 403) {
        serde_json::from_str::<Value>(body).ok().and_then(|v| {
            v.get("detail")?
                .get("error_type")?
                .as_str()
                .and_then(ErrorType::parse)
        })
    } else {
        None
    };
    match status {
        400 if error_type == Some(ErrorType::MaxTokensExceeded) => JevError::ContextExceeded,
        400 => JevError::BadRequest { error_type },
        401 => JevError::Unauthorized { error_type },
        403 => JevError::Forbidden { error_type },
        413 => JevError::PayloadTooLarge,
        415 => JevError::UnsupportedMediaType,
        422 => JevError::Unprocessable,
        429 => JevError::RateLimited,
        529 => JevError::Overloaded,
        status => JevError::Http { status },
    }
}

#[cfg(test)]
mod tests;
