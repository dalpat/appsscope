//! Distro package backend, via PackageKit.
//!
//! One backend covers apt, dnf, pacman and zypper, which is the whole reason
//! for choosing PackageKit over talking to apt directly.
//!
//! Unlike Flatpak, these packages are **not sandboxed** — [`Backend::sandbox`]
//! returns `None`, which the UI renders as "Not sandboxed" rather than as an
//! absence of information. That distinction is the point: a store that shows a
//! deb and a Flatpak identically is hiding the most important difference
//! between them.

mod desktop;
mod proxy;

use std::collections::HashMap;

use appsscope_core::{
    App, AppRef, Backend, BackendId, Error, IconSource, InstallPlan, InstalledApp, Phase, PlanItem,
    Progress, Result, Sandbox,
};
use async_trait::async_trait;
use futures_core::stream::BoxStream;
use futures_util::{StreamExt, stream};
use tokio::sync::{OnceCell, mpsc};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

use proxy::{
    EXIT_SUCCESS, PackageKitProxy, TransactionProxy, filter, flags, info, split_package_id,
};

/// One connection for the whole process.
///
/// Not just an optimisation: PackageKit refuses method calls on a transaction
/// from any connection other than the one that created it.
async fn connection() -> Result<&'static zbus::Connection> {
    static CONNECTION: OnceCell<zbus::Connection> = OnceCell::const_new();

    CONNECTION
        .get_or_try_init(|| async {
            zbus::Connection::system().await.map_err(|err| {
                Error::BackendUnavailable {
                    backend: "packagekit",
                    reason: format!("cannot reach the system bus: {err}"),
                }
            })
        })
        .await
}

/// A package row returned by a transaction.
#[derive(Debug, Clone)]
struct PackageRow {
    info: u32,
    package_id: String,
    summary: String,
}

pub struct PackageKitBackend {
    _private: (),
}

impl Default for PackageKitBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageKitBackend {
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Create a transaction and return a proxy bound to the shared connection.
    async fn transaction() -> Result<TransactionProxy<'static>> {
        let connection = connection().await?;

        let daemon = PackageKitProxy::new(connection)
            .await
            .map_err(|err| backend_error("cannot reach PackageKit", err))?;

        let path = daemon
            .create_transaction()
            .await
            .map_err(|err| backend_error("cannot start a transaction", err))?;

        let transaction = TransactionProxy::builder(connection)
            .path(path)
            .map_err(|err| backend_error("bad transaction path", err))?
            .build()
            .await
            .map_err(|err| backend_error("cannot bind the transaction", err))?;

        // Non-interactive: the daemon must not try to prompt on our behalf.
        let _ = transaction.set_hints(&["interactive=false"]).await;

        Ok(transaction)
    }

    /// Run a query-style transaction and collect its `Package` rows.
    ///
    /// Signals are subscribed to *before* the method is called — PackageKit
    /// starts emitting as soon as the call lands, and a stream created
    /// afterwards would miss the first rows.
    async fn collect_packages<F, Fut>(start: F) -> Result<Vec<PackageRow>>
    where
        F: FnOnce(TransactionProxy<'static>) -> Fut,
        Fut: Future<Output = zbus::Result<()>>,
    {
        let transaction = Self::transaction().await?;

        let mut packages = transaction
            .receive_package()
            .await
            .map_err(|err| backend_error("cannot watch results", err))?;
        let mut finished = transaction
            .receive_finished()
            .await
            .map_err(|err| backend_error("cannot watch completion", err))?;
        let mut errors = transaction
            .receive_error_code()
            .await
            .map_err(|err| backend_error("cannot watch errors", err))?;

        start(transaction.clone())
            .await
            .map_err(|err| backend_error("query failed", err))?;

        let mut rows = Vec::new();
        loop {
            tokio::select! {
                Some(signal) = packages.next() => {
                    if let Ok(args) = signal.args() {
                        rows.push(PackageRow {
                            info: args.info,
                            package_id: args.package_id.to_string(),
                            summary: args.summary.to_string(),
                        });
                    }
                }
                Some(signal) = errors.next() => {
                    if let Ok(args) = signal.args() {
                        return Err(map_error(args.code, args.details.to_string()));
                    }
                }
                Some(signal) = finished.next() => {
                    let exit = signal.args().map(|args| args.exit).unwrap_or_default();
                    if exit != EXIT_SUCCESS {
                        return Err(Error::Backend(format!(
                            "the package manager exited with status {exit}"
                        )));
                    }
                    return Ok(rows);
                }
                else => return Ok(rows),
            }
        }
    }

    /// Installed application packages.
    ///
    /// `APPLICATION` restricts results to things with a desktop entry, which
    /// is what keeps this from returning every shared library on the system.
    async fn query_installed(updates_only: bool) -> Result<Vec<PackageRow>> {
        if updates_only {
            Self::collect_packages(|transaction| async move {
                transaction.get_updates(0).await
            })
            .await
        } else {
            Self::collect_packages(|transaction| async move {
                transaction
                    .get_packages(filter::INSTALLED | filter::APPLICATION | filter::NEWEST)
                    .await
            })
            .await
        }
    }
}

#[async_trait]
impl Backend for PackageKitBackend {
    fn id(&self) -> BackendId {
        BackendId::PackageKit
    }

    async fn is_available(&self) -> bool {
        // The daemon is D-Bus activatable, so "is it running" is the wrong
        // question — asking for a property starts it if it is installed.
        let Ok(connection) = connection().await else {
            return false;
        };
        let Ok(daemon) = PackageKitProxy::new(connection).await else {
            return false;
        };
        daemon.version_major().await.is_ok()
    }

    async fn refresh(&self, _cancel: CancellationToken) -> Result<()> {
        // Refreshing package lists needs authorisation and is the system's
        // job, not a store's — apt and dnf both do it on a timer.
        Ok(())
    }

    async fn installed(&self) -> Result<Vec<InstalledApp>> {
        let rows = Self::query_installed(false).await?;
        Ok(rows.into_iter().filter_map(|row| to_installed(row, false)).collect())
    }



    async fn updates(&self) -> Result<Vec<InstalledApp>> {
        let rows = Self::query_installed(true).await?;
        Ok(rows.into_iter().filter_map(|row| to_installed(row, true)).collect())
    }

    async fn details(&self, app_ref: &AppRef) -> Result<App> {
        let rows = Self::query_installed(false).await?;
        rows.into_iter()
            .find(|row| split_package_id(&row.package_id).0 == app_ref.id)
            .and_then(|row| to_installed(row, false))
            .map(|installed| installed.app)
            .ok_or_else(|| Error::NotFound(app_ref.clone()))
    }

    async fn sandbox(&self, _app_ref: &AppRef) -> Result<Option<Sandbox>> {
        // Deliberately `None`, not an empty `Sandbox`. Distro packages run
        // with the invoking user's full authority; the UI must say "not
        // sandboxed" rather than imply confinement we cannot offer.
        Ok(None)
    }

    async fn set_permission(&self, _app_ref: &AppRef, _key: &str, _granted: bool) -> Result<()> {
        Err(Error::Backend(
            "distro packages have no sandbox to adjust".to_owned(),
        ))
    }

    async fn install_plan(&self, app_ref: &AppRef) -> Result<Option<InstallPlan>> {
        let Some(package_id) = resolve_id(&app_ref.id, filter::NOT_INSTALLED).await? else {
            return Ok(None);
        };

        // SIMULATE resolves dependencies and reports them without touching
        // the system — the same trick the Flatpak backend uses with a
        // transaction it aborts.
        let rows = PackageKitBackend::collect_packages(move |transaction| async move {
            transaction
                .install_packages(flags::SIMULATE, &[package_id.as_str()])
                .await
        })
        .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        let items = rows
            .into_iter()
            .map(|row| {
                let (name, version, _, _) = split_package_id(&row.package_id);
                PlanItem {
                    id: name.to_owned(),
                    name: if version.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{name} {version}")
                    },
                    // Everything except the requested package is a dependency.
                    is_runtime: name != app_ref.id,
                    // PackageKit reports sizes only via GetDetails, one extra
                    // round trip per package; not worth blocking a dialog on.
                    download_size: 0,
                    installed_size: 0,
                }
            })
            .collect();

        Ok(Some(InstallPlan {
            format: "System package",
            source: app_ref.origin.clone(),
            items,
        }))
    }

    fn install(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>> {
        let id = app_ref.id.clone();
        transact(move |transaction, package_id| async move {
            transaction
                .install_packages(flags::ONLY_TRUSTED, &[package_id.as_str()])
                .await
        }, id, filter::NOT_INSTALLED)
    }

    fn remove(&self, app_ref: &AppRef) -> BoxStream<'static, Result<Progress>> {
        let id = app_ref.id.clone();
        transact(move |transaction, package_id| async move {
            transaction
                .remove_packages(flags::NONE, &[package_id.as_str()], true, false)
                .await
        }, id, filter::INSTALLED)
    }

    fn update(&self, app_ref: Option<&AppRef>) -> BoxStream<'static, Result<Progress>> {
        let Some(app_ref) = app_ref else {
            return Box::pin(stream::once(async {
                Err(Error::Backend(
                    "updating everything is not supported yet".to_owned(),
                ))
            }));
        };
        let id = app_ref.id.clone();
        transact(move |transaction, package_id| async move {
            transaction
                .update_packages(flags::ONLY_TRUSTED, &[package_id.as_str()])
                .await
        }, id, filter::INSTALLED)
    }
}

/// Resolve a bare package name to a full PackageKit id.
async fn resolve_id(name: &str, filters: u64) -> Result<Option<String>> {
    let owned = name.to_owned();
    let rows = PackageKitBackend::collect_packages(move |transaction| async move {
        transaction
            .resolve(filters | filter::NEWEST, &[owned.as_str()])
            .await
    })
    .await?;

    Ok(rows.into_iter().next().map(|row| row.package_id))
}

/// Drive a mutating transaction, reporting progress as it runs.
fn transact<F, Fut>(start: F, name: String, filters: u64) -> BoxStream<'static, Result<Progress>>
where
    F: FnOnce(TransactionProxy<'static>, String) -> Fut + Send + 'static,
    Fut: Future<Output = zbus::Result<()>> + Send,
{
    let (tx, rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        if let Err(err) = run_transaction(start, name, filters, &tx).await {
            let _ = tx.send(Err(err));
        }
    });

    Box::pin(UnboundedReceiverStream::new(rx))
}

async fn run_transaction<F, Fut>(
    start: F,
    name: String,
    filters: u64,
    progress: &mpsc::UnboundedSender<Result<Progress>>,
) -> Result<()>
where
    F: FnOnce(TransactionProxy<'static>, String) -> Fut,
    Fut: Future<Output = zbus::Result<()>>,
{
    let _ = progress.send(Ok(Progress::phase(Phase::Resolving)));

    let Some(package_id) = resolve_id(&name, filters).await? else {
        return Err(Error::Backend(format!("no package named {name}")));
    };

    let transaction = PackageKitBackend::transaction().await?;

    let mut items = transaction
        .receive_item_progress()
        .await
        .map_err(|err| backend_error("cannot watch progress", err))?;
    let mut finished = transaction
        .receive_finished()
        .await
        .map_err(|err| backend_error("cannot watch completion", err))?;
    let mut errors = transaction
        .receive_error_code()
        .await
        .map_err(|err| backend_error("cannot watch errors", err))?;

    start(transaction.clone(), package_id)
        .await
        .map_err(|err| backend_error("the operation was refused", err))?;

    loop {
        tokio::select! {
            Some(signal) = items.next() => {
                if let Ok(args) = signal.args() {
                    let phase = match args.status {
                        info::DOWNLOADING => Phase::Downloading,
                        info::REMOVING => Phase::Removing,
                        info::INSTALLING | info::UPDATING => Phase::Installing,
                        _ => Phase::Resolving,
                    };
                    let mut tick = Progress::phase(phase);
                    if args.percentage <= 100 {
                        tick = tick.with_fraction(f64::from(args.percentage) / 100.0);
                    }
                    if progress.send(Ok(tick)).is_err() {
                        // The UI dropped the stream; nothing to report to.
                        return Ok(());
                    }
                }
            }
            Some(signal) = errors.next() => {
                if let Ok(args) = signal.args() {
                    return Err(map_error(args.code, args.details.to_string()));
                }
            }
            Some(signal) = finished.next() => {
                let exit = signal.args().map(|args| args.exit).unwrap_or_default();
                return if exit == EXIT_SUCCESS {
                    Ok(())
                } else {
                    Err(Error::Backend(format!("the operation ended with status {exit}")))
                };
            }
            else => return Ok(()),
        }
    }
}

fn to_installed(row: PackageRow, has_update: bool) -> Option<InstalledApp> {
    let (name, version, _arch, data) = split_package_id(&row.package_id);
    if name.is_empty() {
        return None;
    }

    // PackageKit's APPLICATION filter lets daemons through on the apt backend,
    // and gives neither a display name nor an icon. The desktop entry supplies
    // all three: whether this is really an application, what to call it, and
    // what to draw. A package with no visible desktop entry is dropped.
    let applications = desktop::applications();
    let entry = applications.get(name);
    if entry.is_none() && !applications.is_empty() {
        return None;
    }

    // `data` is like `installed:ubuntu-resolute-main`; the part after the
    // colon is the origin worth showing.
    let origin = data.split_once(':').map(|(_, repo)| repo.to_owned());

    let app_ref = {
        let base = AppRef::new(BackendId::PackageKit, name);
        match origin {
            Some(origin) if !origin.is_empty() => base.with_origin(origin),
            _ => base,
        }
    };

    let app = App {
        app_ref,
        name: entry
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| name.to_owned()),
        summary: (!row.summary.is_empty()).then(|| row.summary.clone()),
        description: None,
        developer: None,
        license: None,
        categories: Vec::new(),
        icon: entry
            .and_then(|entry| entry.icon.clone())
            .map(IconSource::Themed),
        screenshots: Vec::new(),
        releases: Vec::new(),
        version: (!version.is_empty()).then(|| version.to_owned()),
        download_size: None,
        installed_size: None,
        // No sandbox to describe; see `Backend::sandbox`.
        sandbox: None,
    };

    let update = has_update.then(|| appsscope_core::Update {
        version: version.to_owned(),
        download_size: None,
        releases: Vec::new(),
    });

    let _ = row.info;
    Some(InstalledApp {
        app,
        installed_version: (!version.is_empty()).then(|| version.to_owned()),
        update,
    })
}

fn backend_error(context: &str, err: impl std::fmt::Display) -> Error {
    Error::Backend(format!("{context}: {err}"))
}

/// Map `PkErrorEnum` onto our taxonomy.
///
/// The two worth distinguishing are the ones the UI can act on: a declined
/// authorisation prompt, and anything network-shaped.
fn map_error(code: u32, details: String) -> Error {
    const NOT_AUTHORIZED: u32 = 34;
    const NO_NETWORK: u32 = 10;

    match code {
        NOT_AUTHORIZED => Error::NotAuthorized,
        NO_NETWORK => Error::Network(details),
        _ => Error::Backend(details),
    }
}

/// Unused today; kept so the dictionary shape is documented in one place.
#[allow(dead_code)]
fn detail_size(data: &HashMap<String, zbus::zvariant::OwnedValue>) -> Option<u64> {
    data.get("size").and_then(|value| u64::try_from(value).ok())
}

#[cfg(test)]
mod live {
    use super::*;

    /// Query the real PackageKit daemon.
    ///
    /// Read-only: `GetPackages` needs no authorisation, so this changes
    /// nothing. Gated because it needs a system bus and a running daemon.
    #[tokio::test]
    async fn lists_installed_applications() {
        if std::env::var_os("APPSSCOPE_SYSTEM_TESTS").is_none() {
            return;
        }

        let backend = PackageKitBackend::new();
        println!("available: {}", backend.is_available().await);

        match backend.installed().await {
            Ok(apps) => {
                println!("installed applications: {}", apps.len());
                for app in apps.iter().take(12) {
                    println!(
                        "   {:<28} {:<18} {}",
                        app.app.name,
                        app.app.version.as_deref().unwrap_or("-"),
                        app.app.summary.as_deref().unwrap_or(""),
                    );
                }
                assert!(!apps.is_empty(), "expected at least one installed application");
            }
            Err(err) => panic!("installed() failed: {err}"),
        }
    }
}
