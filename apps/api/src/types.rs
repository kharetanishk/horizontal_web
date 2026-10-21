//! Wire types shared with the web app. Source of truth for `packages/schemas`:
//! `cargo test` regenerates the TypeScript (ts-rs) and JSON Schema (schemars), CI fails on drift.
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    UserMessage,
    AiMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Streaming,
    Complete,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    Reply,
    Branch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoubtType {
    Simpler,
    Example,
    Why,
    Deeper,
    Wrong,
    Free,
}

/// Where a branch hangs off its parent. `start`/`end` are UTF-16 code-unit offsets
/// into the parent's raw markdown; for `code` they cover the whole block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anchor {
    None,
    Text {
        start: u32,
        end: u32,
        text: String,
    },
    Code {
        start: u32,
        end: u32,
        start_line: u32,
        end_line: u32,
        text: String,
    },
}

/// API error codes (docs/ARCHITECTURE.md, API).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    ForbiddenOrigin,
    NotFound,
    Conflict,
    MessageTooLong,
    InvalidAnchor,
    ParentNotComplete,
    BranchStreaming,
    MergeBudgetExhausted,
    RateLimited,
    DailyCapReached,
    LlmUnavailable,
    Unavailable,
    // 500, e.g. a caught panic. A `///` doc here would turn the JSON Schema enum into oneOf.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
pub struct ErrorDetail {
    pub code: ErrorCode,
    pub message: String,
}

/// Body of every non-2xx response and of the SSE `error` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, JsonSchema)]
pub struct ApiErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
pub struct Canvas {
    pub id: Uuid,
    pub title: String,
    pub root_node_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
pub struct Node {
    pub id: Uuid,
    pub canvas_id: Uuid,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub node_type: NodeType,
    pub content: String,
    pub status: NodeStatus,
    /// Set with `status = error` (`interrupted`, `persist_failed`, …) or `truncated` on a complete node.
    pub error_code: Option<String>,
    pub attempt: i32,
    pub model: Option<String>,
    pub position_x: f64,
    pub position_y: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
pub struct Edge {
    pub id: Uuid,
    pub canvas_id: Uuid,
    pub source_id: Uuid,
    pub target_id: Uuid,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub edge_type: EdgeType,
    /// Present exactly when `type = branch`, as is `doubt_type`.
    pub anchor: Option<Anchor>,
    pub doubt_type: Option<DoubtType>,
    pub merge_summary: Option<String>,
    pub merged_at: Option<DateTime<Utc>>,
    pub collapsed: bool,
    pub created_at: DateTime<Utc>,
}

/// SSE events of a send/retry/attach stream. The variant name is the SSE `event:` field
/// and `data` its JSON payload; `reattach` has no payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, JsonSchema)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum StreamEvent {
    Created { nodes: Vec<Node>, edges: Vec<Edge> },
    Snapshot { node: Node },
    Delta { text: String },
    Done { node: Node, context_tokens: u32 },
    Error { error: ErrorDetail, node: Node },
    Reattach,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, path::Path};

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
        v: T,
        expected: serde_json::Value,
    ) {
        assert_eq!(serde_json::to_value(&v).unwrap(), expected);
        assert_eq!(serde_json::from_value::<T>(expected).unwrap(), v);
    }

    #[test]
    fn anchor_wire_shape() {
        round_trip(Anchor::None, json!({"kind": "none"}));
        round_trip(
            Anchor::Text {
                start: 412,
                end: 540,
                text: "x".into(),
            },
            json!({"kind": "text", "start": 412, "end": 540, "text": "x"}),
        );
        round_trip(
            Anchor::Code {
                start: 900,
                end: 1340,
                start_line: 5,
                end_line: 9,
                text: "y".into(),
            },
            json!({"kind": "code", "start": 900, "end": 1340, "start_line": 5, "end_line": 9, "text": "y"}),
        );
    }

    #[test]
    fn error_body_wire_shape() {
        round_trip(
            ApiErrorBody {
                error: ErrorDetail {
                    code: ErrorCode::NotFound,
                    message: "Canvas not found".into(),
                },
            },
            json!({"error": {"code": "not_found", "message": "Canvas not found"}}),
        );
        round_trip(ErrorCode::DailyCapReached, json!("daily_cap_reached"));
    }

    #[test]
    fn stream_event_wire_shape() {
        round_trip(
            StreamEvent::Delta { text: "hi".into() },
            json!({"event": "delta", "data": {"text": "hi"}}),
        );
        round_trip(StreamEvent::Reattach, json!({"event": "reattach"}));
    }

    /// Regenerates packages/schemas (NFR-16). CI runs `cargo test` then
    /// `git diff --exit-code packages/schemas`, so a stale checkout fails.
    #[test]
    fn generate_schemas() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/schemas");
        let cfg = ts_rs::Config::new();
        let mut ts =
            String::from("// Generated by `cargo test` from apps/api/src/types.rs. Do not edit.\n");
        let json_dir = root.join("json");
        let _ = fs::remove_dir_all(&json_dir);
        fs::create_dir_all(&json_dir).unwrap();

        macro_rules! emit {
            ($($t:ty),*) => {$(
                ts.push_str(&format!("\nexport {}\n", <$t as TS>::decl(&cfg)));
                let schema = serde_json::to_string_pretty(&schemars::schema_for!($t)).unwrap();
                fs::write(json_dir.join(format!("{}.json", stringify!($t))), schema + "\n").unwrap();
            )*};
        }
        emit!(
            NodeType,
            NodeStatus,
            EdgeType,
            DoubtType,
            Anchor,
            ErrorCode,
            ErrorDetail,
            ApiErrorBody,
            Canvas,
            Node,
            Edge,
            StreamEvent
        );
        fs::write(root.join("src/types.ts"), ts).unwrap();
    }
}
