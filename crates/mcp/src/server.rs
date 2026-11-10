//! Listens for MCP clients inside the running desktop process.
use super::{Grants, Handler, Inbox, socket};
use crate::Error;
use engine::{ByteBudget, control::MediaAccess};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

/// A listener in the running desktop process. Dropping it closes transports and
/// joins its runtime; queued requests never outlive the UI-owned inbox.
pub struct Server {
    pub inbox: Inbox,
    runtime: Option<tokio::runtime::Runtime>,
}
impl Server {
    pub fn bind(endpoint: &Path, grants: Grants, budget: ByteBudget) -> Result<Self, Error> {
        let (handler, inbox) = Inbox::new(grants, budget)?;
        Self::listen(endpoint, handler, inbox)
    }

    /// Bind a desktop listener that lends the process media caches the desktop
    /// timeline also reads instead of opening a second cache root.
    pub fn bind_with_media(
        endpoint: &Path,
        grants: Grants,
        budget: ByteBudget,
        media: Option<Arc<Mutex<MediaAccess>>>,
    ) -> Result<Self, Error> {
        let (handler, inbox) = Inbox::with_media(grants, budget, media)?;
        Self::listen(endpoint, handler, inbox)
    }

    fn listen(endpoint: &Path, mut handler: Handler, inbox: Inbox) -> Result<Self, Error> {
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
