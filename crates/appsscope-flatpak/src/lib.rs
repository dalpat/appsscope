//! Flatpak backend.
//!
//! All real work happens on the thread in [`worker`], which owns the `!Send`
//! libflatpak objects. This file is only the async façade over that thread:
//! each method sends a command and awaits the reply, so the rest of the app
//! sees an ordinary async API.

pub mod sandbox;
mod worker;

use appsscope_core::{
    App, AppRef, Backend, BackendId, Error, InstallPlan, InstalledApp, Progress, Result, Sandbox,
};
use async_trait::async_trait;
use futures_core::stream::BoxStream;
use futures_util::stream;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

use worker::{Command, Operation};

pub struct FlatpakBackend {
    commands: mpsc::UnboundedSender<Command>,
}

impl Default for FlatpakBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl FlatpakBackend {
    /// Starts the worker thread. Cheap enough to call at startup even when
    /// Flatpak turns out to be unavailable — the thread just idles.
    pub fn new() -> Self {
        Self { commands: worker::spawn() }
    }

    /// Send a command that produces a single reply.
    async fn ask<T>(&self, make: impl FnOnce(oneshot::Sender<Result<T>>) -> Command) -> Result<T> {
        let (tx, rx) = oneshot::channel();

        self.commands.send(make(tx)).map_err(|_| Error::BackendUnavailable {
            backend: "flatpak",
            reason: "worker thread has stopped".into(),
        })?;

        rx.await.map_err(|_| Error::BackendUnavailable {
            backend: "flatpak",
            reason: "worker thread dropped the request".into(),
        })?
    }

    /// Start a transaction and hand back its progress stream.
    fn transact(&self, operation: Operation) -> BoxStream<'static, Result<Progress>> {
        let (tx, rx) = mpsc::unbounded_channel();

        if self.commands.send(Command::Transact(operation, tx)).is_err() {
            return Box::pin(stream::once(async {
                Err(Error::BackendUnavailable {
                    backend: "flatpak",
                    reason: "worker thread has stopped".into(),
                })
            }));
        }

        Box::pin(UnboundedReceiverStream::new(rx))
    }
}

#[async_trait]
impl Backend for FlatpakBackend {
    fn id(&self) -> BackendId {
        BackendId::Flatpak
    }

    async fn is_available(&self) -> bool {
        // An installation directory is a better signal than the binary
        // existing: the CLI can be present with nothing configured, in which
        // case there is nothing for us to show.
        let system = std::path::Path::new("/var/lib/flatpak").is_dir();
        let user = user_flatpak_dir().is_some_and(|p| p.is_dir());
        system || user
    }

    async fn refresh(&self, _cancel: CancellationToken) -> Result<()> {
        self.ask(Command::RefreshAppstream).await
    }

    async fn installed(&self) -> Result<Vec<InstalledApp>> {
        self.ask(Command::Installed).await
    }

    async fn updates(&self) -> Result<Vec<InstalledApp>> {
        self.ask(Command::Updates).await
    }

    async fn details(&self, app_ref: &AppRef) -> Result<App> {
        let app_ref = app_ref.clone();
        self.ask(move |tx| Command::Details(app_ref, tx)).await
    }

    async fn sandbox(&self, app_ref: &AppRef) -> Result<Option<Sandbox>> {
        let app_ref = app_ref.clone();
        self.ask(move |tx| Command::Sandbox(app_ref, tx)).await
    }

    async fn set_permission(&self, app_ref: &AppRef, key: &str, granted: bool) -> Result<()> {
        let app_ref = app_ref.clone();
        let key = key.to_owned();
        self.ask(move |reply| Command::SetPermission { app_ref, key, granted, reply })
            .await
    }

    async fn install_plan(&self, app_ref: &AppRef) -> Result<Option<InstallPlan>> {
        let app_ref = app_ref.clone();
        self.ask(move |tx| Command::Plan(app_ref, tx)).await
    }

    fn install(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>> {
        self.transact(Operation::Install(app_ref.clone()))
    }

    fn remove(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>> {
        self.transact(Operation::Remove(app_ref.clone()))
    }

    fn update(&self, app_ref: Option<&AppRef>) -> BoxStream<'static, Result<Progress>> {
        self.transact(Operation::Update(app_ref.cloned()))
    }
}

/// `$XDG_DATA_HOME/flatpak`, falling back to `~/.local/share/flatpak`.
fn user_flatpak_dir() -> Option<std::path::PathBuf> {
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(std::path::PathBuf::from(data).join("flatpak"));
    }
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty())?;
    Some(std::path::PathBuf::from(home).join(".local/share/flatpak"))
}
