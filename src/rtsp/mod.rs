//! RTSP control protocol module.

pub mod handler;
pub mod plist;
pub mod types;

use std::sync::Arc;
use anyhow::Result;
use crate::config::Config;
use crate::RuntimeState;

pub use handler::RtspHandle;

pub async fn start(config: &Config, runtime: Arc<RuntimeState>) -> Result<RtspHandle> {
    handler::serve(config, runtime).await
}
