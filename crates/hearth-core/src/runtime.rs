//! The tokio runtime every connection actor runs on. It is process-wide so the
//! core works the same under Tauri (which is handed this runtime) and in tests.

use std::future::Future;
use std::sync::OnceLock;
use tokio::runtime::{Handle, Runtime};
use tokio::task::JoinHandle;

pub fn handle() -> &'static Handle {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("hearth")
            .build()
            .expect("could not start the tokio runtime")
    })
    .handle()
}

pub fn spawn<F>(fut: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    handle().spawn(fut)
}

/// Run blocking work (SQLite reads) off the async workers.
pub async fn blocking<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    handle().spawn_blocking(f).await.map_err(|e| e.to_string())
}
