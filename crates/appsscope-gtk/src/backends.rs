//! Backend registry — fans queries out across every usable package system.

use std::sync::Arc;

use appsscope_core::{App, AppRef, Backend, BackendId, Error, InstalledApp, Progress, Result};
use appsscope_flatpak::FlatpakBackend;
use appsscope_pk::PackageKitBackend;
use futures_core::stream::BoxStream;
use futures_util::stream;

pub struct Registry {
    backends: Vec<Arc<dyn Backend>>,
}

// `dyn Backend` isn't Debug, but messages carrying a Registry need to be.
impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("backends", &self.backends.iter().map(|b| b.id()).collect::<Vec<_>>())
            .finish()
    }
}

impl Registry {
    /// Probe every compiled-in backend and keep the ones that work here.
    ///
    /// A backend that isn't installed is not an error — it's simply absent,
    /// and the UI should never mention it.
    pub async fn detect() -> Self {
        // Order matters only for display; both are probed concurrently below
        // in the sense that neither blocks the other's availability check.
        let candidates: Vec<Arc<dyn Backend>> = vec![
            Arc::new(FlatpakBackend::new()),
            Arc::new(PackageKitBackend::new()),
        ];

        let mut backends = Vec::new();
        for backend in candidates {
            if backend.is_available().await {
                tracing::info!(backend = %backend.id(), "available");
                backends.push(backend);
            } else {
                tracing::info!(backend = %backend.id(), "unavailable, skipping");
            }
        }
        Self { backends }
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    pub fn backend_count(&self) -> usize {
        self.backends.len()
    }

    /// Formats available on this machine, for the filter control.
    pub fn backend_ids(&self) -> Vec<BackendId> {
        self.backends.iter().map(|backend| backend.id()).collect()
    }

    /// Installed apps across all backends.
    pub async fn installed(&self) -> Result<Vec<App>, String> {
        self.gather(|b| async move { b.installed().await }).await
    }

    /// Apps with pending updates across all backends.
    pub async fn updates(&self) -> Result<Vec<App>, String> {
        self.gather(|b| async move { b.updates().await }).await
    }

    /// An app's sandbox profile, or `None` if unknown/unconfined.
    pub async fn sandbox(&self, app_ref: &AppRef) -> Option<appsscope_core::Sandbox> {
        let backend = self.backend_for(app_ref.backend)?;
        match backend.sandbox(app_ref).await {
            Ok(sandbox) => sandbox,
            Err(err) => {
                tracing::warn!(app = %app_ref, %err, "sandbox lookup failed");
                None
            }
        }
    }

    /// What installing this app would actually fetch.
    pub async fn install_plan(&self, app_ref: &AppRef) -> Option<appsscope_core::InstallPlan> {
        let backend = self.backend_for(app_ref.backend)?;
        match backend.install_plan(app_ref).await {
            Ok(plan) => plan,
            Err(err) => {
                tracing::warn!(app = %app_ref, %err, "install plan failed");
                None
            }
        }
    }

    pub async fn set_permission(
        &self,
        app_ref: &AppRef,
        key: &str,
        granted: bool,
    ) -> Result<(), String> {
        let backend = self
            .backend_for(app_ref.backend)
            .ok_or_else(|| "backend unavailable".to_owned())?;
        backend
            .set_permission(app_ref, key, granted)
            .await
            .map_err(|e| e.to_string())
    }

    fn backend_for(&self, id: BackendId) -> Option<&Arc<dyn Backend>> {
        self.backends.iter().find(|b| b.id() == id)
    }

    pub fn install(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>> {
        match self.backend_for(app_ref.backend) {
            Some(backend) => backend.install(app_ref),
            None => missing_backend(app_ref),
        }
    }

    pub fn remove(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>> {
        match self.backend_for(app_ref.backend) {
            Some(backend) => backend.remove(app_ref),
            None => missing_backend(app_ref),
        }
    }

    pub fn update(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>> {
        match self.backend_for(app_ref.backend) {
            Some(backend) => backend.update(Some(app_ref)),
            None => missing_backend(app_ref),
        }
    }

    /// Run `query` against every backend and merge the results.
    ///
    /// Partial failure is tolerated: one broken backend shouldn't blank the
    /// whole list. Only a total failure — every backend erroring — surfaces as
    /// an error, since then there is genuinely nothing to show.
    async fn gather<F, Fut>(&self, query: F) -> Result<Vec<App>, String>
    where
        F: Fn(Arc<dyn Backend>) -> Fut,
        Fut: Future<Output = appsscope_core::Result<Vec<InstalledApp>>>,
    {
        let mut apps = Vec::new();
        let mut errors = Vec::new();

        for backend in &self.backends {
            match query(Arc::clone(backend)).await {
                Ok(found) => apps.extend(found.into_iter().map(|i| i.app)),
                Err(err) => {
                    tracing::warn!(backend = %backend.id(), %err, "query failed");
                    errors.push(err.to_string());
                }
            }
        }

        if apps.is_empty() && !errors.is_empty() {
            return Err(errors.join("\n"));
        }

        apps.sort_by_key(|a| a.name.to_lowercase());
        Ok(apps)
    }
}

/// The app came from a catalog entry whose backend isn't usable here — rare,
/// but possible if a catalog is left over from before a backend was removed.
fn missing_backend(app_ref: &AppRef) -> BoxStream<'static, Result<Progress>> {
    let backend = app_ref.backend;
    Box::pin(stream::once(async move {
        Err(Error::BackendUnavailable {
            backend: backend.as_str(),
            reason: "not available on this system".into(),
        })
    }))
}
