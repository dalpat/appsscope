//! Bridge between the GTK main loop and the async backends.
//!
//! GTK is single-threaded and its types are `!Send`; tokio wants to move work
//! across threads. Rather than fight that, we run one multi-threaded tokio
//! runtime for the whole process and never let a GTK type cross into it.
//!
//! Backend work is spawned with [`spawn`], which runs the future on tokio and
//! delivers the result back on the GTK main thread — so the callback can touch
//! widgets freely.

use std::future::Future;
use std::sync::OnceLock;

use futures_util::{Stream, StreamExt};
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// The process-wide tokio runtime.
///
/// Panics if it can't be built — there is no meaningful degraded mode; without
/// a runtime the app cannot talk to any backend.
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        Runtime::new().expect("failed to start the tokio runtime")
    })
}

/// Forward every item of `stream` to the GTK main thread as it arrives.
///
/// Used for transaction progress, where a single result at the end isn't
/// enough. The forwarding task stops as soon as the GTK side goes away, which
/// closes the channel and — for backends that watch for it — cancels the work.
/// `on_done` fires once the stream has ended and every item has been
/// delivered — for a transaction that is the success signal, since backends
/// report failure as an item and success by simply finishing.
pub fn spawn_stream<S, C, D>(stream: S, mut on_item: C, on_done: D)
where
    S: Stream + Send + 'static,
    S::Item: Send + 'static,
    C: FnMut(S::Item) + 'static,
    D: FnOnce() + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    runtime().spawn(async move {
        let mut stream = std::pin::pin!(stream);
        while let Some(item) = stream.next().await {
            if tx.send(item).is_err() {
                break;
            }
        }
    });

    relm4::spawn_local(async move {
        while let Some(item) = rx.recv().await {
            on_item(item);
        }
        on_done();
    });
}

/// Run `fut` on the tokio runtime, then `on_done` on the GTK main thread.
///
/// The result has to be `Send` to make the trip out; the callback does not,
/// which is what lets it hold widget references.
pub fn spawn<F, T, C>(fut: F, on_done: C)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();

    runtime().spawn(async move {
        // A send failure just means the receiver was dropped — the UI moved on
        // and no longer cares about this result. Not an error.
        let _ = tx.send(fut.await);
    });

    relm4::spawn_local(async move {
        if let Ok(value) = rx.await {
            on_done(value);
        }
    });
}
