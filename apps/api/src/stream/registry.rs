use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::watch;
use uuid::Uuid;

use crate::types::NodeStatus;

/// Finished streams stay attachable this long so late readers still get the full text.
pub const LINGER: Duration = Duration::from_secs(30);
pub const MAX_ATTACHES_PER_USER: usize = 10;

#[derive(Clone, Copy)]
pub struct State {
    pub len: usize,
    pub status: NodeStatus,
}

pub struct Stream {
    pub attempt: u32,
    pub user_id: Uuid,
    pub(crate) text: RwLock<String>,
    pub(crate) state: watch::Sender<State>,
}

#[derive(Default)]
pub struct Registry {
    streams: Mutex<HashMap<Uuid, Arc<Stream>>>,
    pub(crate) attaches: Mutex<HashMap<Uuid, usize>>,
}

/// Producer handle. Never blocks: appends take a short lock and a watch send.
pub struct Writer {
    registry: Arc<Registry>,
    node_id: Uuid,
    stream: Arc<Stream>,
}

impl Registry {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// Registers a stream for `node_id`, replacing any earlier attempt.
    pub fn start(self: &Arc<Self>, node_id: Uuid, user_id: Uuid, attempt: u32) -> Writer {
        let (state, _) = watch::channel(State {
            len: 0,
            status: NodeStatus::Streaming,
        });
        let stream = Arc::new(Stream {
            attempt,
            user_id,
            text: RwLock::new(String::new()),
            state,
        });
        self.streams.lock().unwrap().insert(node_id, stream.clone());
        Writer {
            registry: self.clone(),
            node_id,
            stream,
        }
    }

    pub(crate) fn get(&self, node_id: Uuid) -> Option<Arc<Stream>> {
        self.streams.lock().unwrap().get(&node_id).cloned()
    }
}

impl Writer {
    pub fn append(&self, s: &str) {
        let len = {
            let mut t = self.stream.text.write().unwrap();
            t.push_str(s);
            t.len()
        };
        self.stream.state.send_modify(|st| st.len = len);
    }

    /// Marks the stream terminal (`Complete` or `Error`) and schedules removal.
    pub fn finish(self, status: NodeStatus) {
        self.stream.state.send_modify(|st| st.status = status);
        let Writer {
            registry,
            node_id,
            stream,
        } = self;
        tokio::spawn(async move {
            tokio::time::sleep(LINGER).await;
            let mut m = registry.streams.lock().unwrap();
            // A newer attempt may have replaced this entry.
            if m.get(&node_id).is_some_and(|s| Arc::ptr_eq(s, &stream)) {
                m.remove(&node_id);
            }
        });
    }
}
