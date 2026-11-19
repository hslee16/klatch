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

/// One image sent as a file part of a multipart request (images.md, "Over HTTP"). Debug never shows the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct FilePart {
    name: String,
    content_type: &'static str,
    bytes: Vec<u8>,
}

impl FilePart {
    pub fn new(name: String, content_type: &'static str, bytes: Vec<u8>) -> FilePart {
        FilePart {
            name,
            content_type,
            bytes,
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn content_type(&self) -> &'static str {
        self.content_type
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for FilePart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FilePart({}, {}, {} bytes)",
            self.name,
            self.content_type,
            self.bytes.len()
        )
    }
}

/// An upper bound on the framing one part adds (boundary line, two headers, blank line, closing CRLF); the closing line
/// of the whole body is covered by the same bound. Used for the size check before anything is built.
pub const MULTIPART_OVERHEAD: usize = 512;

/// The multipart body and its `Content-Type` value: the JSON as the first part named `request` with content type
/// `application/json`, then one file part per image in order. The boundary is the first of a numbered series that does
/// not occur anywhere in the data, so no part can end early.
pub fn multipart_body(json: &str, files: &[FilePart]) -> (String, Vec<u8>) {
    let contains = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);
    let mut n = 0u32;
    let boundary = loop {
        let candidate = format!("klatch-jev-boundary-{n:08}");
        let delim = format!("--{candidate}");
        let hit = contains(json.as_bytes(), delim.as_bytes())
            || files.iter().any(|f| contains(&f.bytes, delim.as_bytes()));
        if !hit {
            break candidate;
        }
        n += 1;
    };
    let mut out = Vec::with_capacity(
        json.len()
            + files.iter().map(|f| f.bytes.len()).sum::<usize>()
            + MULTIPART_OVERHEAD * (files.len() + 1),
    );
    out.extend(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"request\"\r\nContent-Type: application/json\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend(json.as_bytes());
    out.extend(b"\r\n");
    for f in files {
        out.extend(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{n}\"; filename=\"{n}\"\r\nContent-Type: {t}\r\n\r\n",
                n = f.name,
                t = f.content_type
            )
            .as_bytes(),
        );
        out.extend(&f.bytes);
        out.extend(b"\r\n");
    }
    out.extend(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), out)
}

#[derive(Clone, PartialEq, Eq)]
pub struct JevRequest {
    pub method: Method,
    pub path: &'static str,
    /// Add [`IMAGES_HEADER`].
    pub images: bool,
    /// The JSON body; with `files` it is the `request` part of a multipart body.
    pub body: Option<String>,
    /// Image files; when not empty the request is multipart/form-data (see [`JevRequest::multipart`]).
    pub files: Vec<FilePart>,
}

impl fmt::Debug for JevRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JevRequest({:?} {}, images: {}, files: {}, body: <withheld>)",
            self.method,
            self.path,
            self.images,
            self.files.len()
        )
    }
}

impl JevRequest {
    /// `POST /v1/systemone` with a body that passed [`super::request::build_request`]. A request with images goes as
    /// multipart (the file as it is, images.md "Over HTTP"); one without goes as JSON.
    pub fn system_one(prepared: &super::request::Prepared) -> JevRequest {
        let (body, files) = match prepared.multipart() {
            Some((json, files)) => (json.to_string(), files.to_vec()),
            None => (prepared.body().to_string(), vec![]),
        };
        JevRequest {
            method: Method::Post,
            path: SYSTEM_ONE_PATH,
            images: prepared.images(),
            body: Some(body),
            files,
        }
    }

    /// The `Content-Type` and the body to send when this request is multipart.
    pub fn multipart(&self) -> Option<(String, Vec<u8>)> {
        if self.files.is_empty() {
            return None;
        }
        Some(multipart_body(self.body.as_deref()?, &self.files))
    }

    /// `GET /v1/models`.
    pub fn models() -> JevRequest {
        JevRequest {
            method: Method::Get,
            path: MODELS_PATH,
            images: false,
            body: None,
            files: vec![],
        }
    }

    /// The headers to add besides `Authorization: Bearer <key>`.
    pub fn headers(&self) -> Vec<(&'static str, &'static str)> {
        let mut h = vec![];
        if self.body.is_some() && self.files.is_empty() {
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
