//! In-process streaming of AI node text to SSE clients.
pub mod attach;
pub mod registry;

pub use attach::attach;
pub use registry::{Registry, Writer};
