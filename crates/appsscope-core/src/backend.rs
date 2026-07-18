use async_trait::async_trait;
use futures_core::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::error::Result;
use crate::plan::InstallPlan;
use crate::sandbox::Sandbox;
use crate::types::{App, AppRef, BackendId, InstalledApp, Progress};

/// A package system AppsScope can drive.
///
/// Every method runs off the GTK main thread — implementations are free to
/// block internally on their own runtime, and must never touch GTK types.
///
/// Kept dyn-compatible on purpose: the app holds a `Vec<Arc<dyn Backend>>` and
/// fans queries out across all of them, so this uses `#[async_trait]` rather
/// than native async-fn-in-trait.
#[async_trait]
pub trait Backend: Send + Sync + 'static {
    fn id(&self) -> BackendId;

    /// Cheap liveness check — is the daemon running, is the tool installed?
    /// Called at startup to decide whether to register this backend at all.
    async fn is_available(&self) -> bool;

    /// Refresh remote metadata (appstream, package lists).
    ///
    /// Expected to be slow and network-bound. Callers pass a token so a
    /// window close or a user cancel can abort it mid-download.
    async fn refresh(&self, cancel: CancellationToken) -> Result<()>;

    /// Everything currently installed via this backend.
    async fn installed(&self) -> Result<Vec<InstalledApp>>;

    /// Apps with a pending update.
    ///
    /// Separate from `installed()` because backends can usually answer this
    /// far more cheaply than by diffing the full installed set.
    async fn updates(&self) -> Result<Vec<InstalledApp>>;

    /// Full detail for one app — the expensive fields (screenshots, changelog,
    /// exact sizes) that the list view doesn't need.
    async fn details(&self, app_ref: &AppRef) -> Result<App>;

    /// The app's sandbox profile, or `None` for backends with no sandbox.
    ///
    /// Returning `None` is meaningfully different from returning an empty
    /// `Sandbox`: `None` means "this packaging format confines nothing", which
    /// the UI must say out loud rather than implying the app is safe.
    async fn sandbox(&self, app_ref: &AppRef) -> Result<Option<Sandbox>>;

    /// Revoke or restore one permission for an installed app.
    ///
    /// `key` is the [`crate::sandbox::Permission::override_key`] value.
    async fn set_permission(&self, app_ref: &AppRef, key: &str, granted: bool) -> Result<()>;

    /// Work out exactly what installing this app would fetch, without
    /// fetching it.
    ///
    /// Expected to hit the network: resolving dependencies means asking the
    /// remote. Returns `None` when the backend cannot answer cheaply enough
    /// to be worth blocking a dialog on.
    async fn install_plan(&self, app_ref: &AppRef) -> Result<Option<InstallPlan>>;

    /// Install an app, reporting progress until the stream ends.
    ///
    /// The stream completing without an error item means success. Dropping the
    /// stream requests cancellation; implementations should treat that as a
    /// rollback trigger where the underlying system supports it.
    fn install(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>>;

    fn remove(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>>;

    /// Update a single app, or everything this backend owns when `None`.
    fn update(&self, app_ref: Option<&AppRef>) -> BoxStream<'static, Result<Progress>>;
}
