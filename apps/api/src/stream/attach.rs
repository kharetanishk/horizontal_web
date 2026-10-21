use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use uuid::Uuid;

use super::registry::{MAX_ATTACHES_PER_USER, Registry};
use crate::http::error::ApiError;
use crate::types::{ErrorCode, NodeStatus};

struct Slot(Arc<Registry>, Uuid);

impl Drop for Slot {
    fn drop(&mut self) {
        let mut m = self.0.attaches.lock().unwrap();
        if let Some(n) = m.get_mut(&self.1) {
            *n -= 1;
            if *n == 0 {
                m.remove(&self.1);
            }
        }
    }
}

fn ev(name: &str, text: &str) -> Result<Event, Infallible> {
    Ok(Event::default()
        .event(name)
        .data(serde_json::json!({ "text": text }).to_string()))
}

/// Streams `snapshot`, then coalesced `delta`s, then exactly one `done`/`error`.
/// Unknown node or other owner → 404; more than 10 live attaches for the user → 429.
pub fn attach(
    registry: &Arc<Registry>,
    node_id: Uuid,
    user_id: Uuid,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>> + use<>>, ApiError> {
    let stream = registry
        .get(node_id)
        .filter(|s| s.user_id == user_id)
        .ok_or(ApiError::Code(ErrorCode::NotFound))?;
    {
        let mut m = registry.attaches.lock().unwrap();
        let n = m.entry(user_id).or_insert(0);
        if *n >= MAX_ATTACHES_PER_USER {
            return Err(ApiError::TooManyRequests {
                code: ErrorCode::RateLimited,
                retry_after: 1,
            });
        }
        *n += 1;
    }
    let slot = Slot(registry.clone(), user_id);
    let mut rx = stream.state.subscribe();
    let body = async_stream::stream! {
        let _slot = slot;
        let mut offset = 0;
        let mut first = true;
        loop {
            // Read state before text: status is set after the final append, so
            // `len` is complete whenever the status is terminal.
            let st = *rx.borrow_and_update();
            let chunk = stream.text.read().unwrap()[offset..st.len].to_string();
            offset = st.len;
            if first {
                yield ev("snapshot", &chunk);
                first = false;
            } else if !chunk.is_empty() {
                yield ev("delta", &chunk);
            }
            match st.status {
                NodeStatus::Streaming => {
                    if rx.changed().await.is_err() { break; }
                }
                NodeStatus::Complete => { yield ev("done", ""); break; }
                NodeStatus::Error => { yield ev("error", ""); break; }
            }
        }
    };
    Ok(Sse::new(body).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("")))
}
