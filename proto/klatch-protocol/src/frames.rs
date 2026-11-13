//! The frame types of wire protocol version 1. One JSON object per line; every frame carries `"v"`.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    Verified,
    Unattributed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

/// Frames the gateway sends. `Unknown` stands for a type this version does not know; consumers ignore it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Hello {
        v: u32,
        member: String,
        attribution: Attribution,
        clearance: String,
    },
    Message {
        v: u32,
        room: String,
        seq: u64,
        id: String,
        sender: String,
        attribution: Attribution,
        label: String,
        body: String,
        ts: i64,
    },
    CaughtUp {
        v: u32,
        room: String,
        seq: u64,
    },
    Heartbeat {
        v: u32,
        ts: i64,
    },
    Refusal {
        v: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<String>,
        rule: String,
        reason: String,
    },
    Response {
        v: u32,
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ErrorBody>,
    },
    #[serde(other)]
    Unknown,
}

/// Frames a client sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Request {
        v: u32,
        id: String,
        method: String,
        #[serde(default)]
        params: Value,
    },
    Ack {
        v: u32,
        room: String,
        seq: u64,
    },
    #[serde(other)]
    Unknown,
}
