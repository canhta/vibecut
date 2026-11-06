//! Listens for MCP clients inside the running desktop process.
use super::{Grants, Inbox, socket};
use crate::args::CliError;
use engine::ByteBudget;
use std::{path::Path, time::Duration};

/// A listener in the running desktop process. Dropping it closes transports and
/// joins its runtime; queued requests never outlive the UI-owned inbox.
pub struct Server {
    pub inbox: Inbox,
    runtime: Option<tokio::runtime::Runtime>,
}
impl Server {
    pub fn bind(endpoint: &Path, grants: Grants, budget: ByteBudget) -> Result<Self, CliError> {
        let (mut handler, inbox) = Inbox::new(grants, budget)?;
        handler.project_routing = true;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let listener = {
            let _enter = runtime.enter();
            socket::listen(endpoint)?
        };
        runtime.spawn(socket::serve(listener, handler));
        Ok(Self {
            inbox,
            runtime: Some(runtime),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}
