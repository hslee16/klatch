//! The seam between the pure Jev code and the network. A [`JevTransport`] sends one [`JevRequest`] with the key as the
//! bearer credential and returns the body of a success, or says how it failed. The real HTTP transport is part B (#871);
//! [`super::fake::FakeTransport`] is what tests use. The request carries everything an HTTP client needs except the
//! key: the method, the path, the headers to add, and the body. Bodies are never compressed here (see the design item
//! for #898: no dependency in this module can compress, and compression does not raise the 32 MiB limit).

use super::ApiKey;
use super::error::JevTransportError;
use std::fmt;

/// The host the real transport talks to.
pub const BASE_URL: &str = "https://api.typesafe.ai";
pub const SYSTEM_ONE_PATH: &str = "/v1/systemone";
pub const MODELS_PATH: &str = "/v1/models";
/// The header that must be `true` on a request that holds an image or a box question.
pub const IMAGES_HEADER: (&str, &str) = ("TypeSafe-Danger-Zone-Images", "true");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Clone, PartialEq, Eq)]
pub struct JevRequest {
    pub method: Method,
    pub path: &'static str,
    /// Add [`IMAGES_HEADER`].
    pub images: bool,
    pub body: Option<String>,
}

impl fmt::Debug for JevRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JevRequest({:?} {}, images: {}, body: <withheld>)",
            self.method, self.path, self.images
        )
    }
}

impl JevRequest {
    /// `POST /v1/systemone` with a body that passed [`super::request::build_request`].
    pub fn system_one(prepared: &super::request::Prepared) -> JevRequest {
        JevRequest {
            method: Method::Post,
            path: SYSTEM_ONE_PATH,
            images: prepared.images(),
            body: Some(prepared.body().to_string()),
        }
    }

    /// `GET /v1/models`.
    pub fn models() -> JevRequest {
        JevRequest {
            method: Method::Get,
            path: MODELS_PATH,
            images: false,
            body: None,
        }
    }

    /// The headers to add besides `Authorization: Bearer <key>`.
    pub fn headers(&self) -> Vec<(&'static str, &'static str)> {
        let mut h = vec![];
        if self.body.is_some() {
            h.push(("Content-Type", "application/json"));
        }
        if self.images {
            h.push(IMAGES_HEADER);
        }
        h
    }
}

pub trait JevTransport {
    /// Send the request. `Err` is any failure: a non-success HTTP answer is [`JevTransportError::Status`] (with the
    /// `retry-after` and `retry-after-ms` headers when present), anything else is `Network`.
    fn send(&self, key: &ApiKey, request: &JevRequest) -> Result<String, JevTransportError>;
}

impl<T: JevTransport + ?Sized> JevTransport for &T {
    fn send(&self, key: &ApiKey, request: &JevRequest) -> Result<String, JevTransportError> {
        (**self).send(key, request)
    }
}
