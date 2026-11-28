//! What every command and background task is given: the shared state and
//! the host it runs in.

use std::future::Future;
use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::error::AppResult;
use crate::host::Host;
use crate::state::AppState;

/// The app's state, its host, and the runtime background tasks run on.
/// Cheap to clone.
#[derive(Clone)]
pub struct Ctx {
    /// Database, keyring and in-memory state.
    pub state: Arc<AppState>,
    /// The program the app runs in.
    pub host: Arc<dyn Host>,
    runtime: Handle,
}

impl Ctx {
    /// A context whose background tasks run on `runtime`.
    #[must_use]
    pub fn new(state: Arc<AppState>, host: Arc<dyn Host>, runtime: Handle) -> Self {
        state.set_host(&host);
        Self {
            state,
            host,
            runtime,
        }
    }

    /// Push an event to the interface.
    ///
    /// # Errors
    /// When the host couldn't send it.
    pub fn emit(&self, event: &str, payload: serde_json::Value) -> AppResult<()> {
        // Automation subscribers hear it too; none listening is fine.
        if self.state.events.receiver_count() > 0 {
            let _ = self.state.events.send((event.to_owned(), payload.clone()));
        }
        self.host.emit(event, payload)
    }

    /// Run a task in the background, from any thread.
    pub fn spawn<F>(&self, task: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.runtime.spawn(task)
    }
}
