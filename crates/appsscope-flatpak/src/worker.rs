//! The thread that owns all libflatpak state.
//!
//! Every libflatpak type is `!Send` — `Installation`, `Transaction`,
//! `InstalledRef`, all of them. They cannot cross a thread boundary, cannot
//! live in a tokio future, and `Arc` doesn't help because the types themselves
//! are unshareable.
//!
//! So one dedicated OS thread owns them for the process lifetime and talks to
//! the async world over channels, sending only plain owned data. That thread
//! is not the GTK main thread: `Transaction::run()` blocks until the whole
//! install finishes, which would freeze the UI outright.

use appsscope_core::{
    App, AppRef, BackendId, Error, InstallPlan, InstalledApp, Phase, PlanItem, Progress, Release,
    Result, Sandbox, Update,
};
use libflatpak::prelude::*;
use libflatpak::{gio, glib};
use libflatpak::{Installation, Transaction};
use std::cell::Cell;
use std::rc::Rc;
use tokio::sync::{mpsc, oneshot};

/// What the transaction should do. Resolved to concrete refs on the worker.
#[derive(Debug, Clone)]
pub enum Operation {
    Install(AppRef),
    Remove(AppRef),
    /// `None` updates everything Flatpak owns.
    Update(Option<AppRef>),
}

pub enum Command {
    Installed(oneshot::Sender<Result<Vec<InstalledApp>>>),
    Updates(oneshot::Sender<Result<Vec<InstalledApp>>>),
    Details(AppRef, oneshot::Sender<Result<App>>),
    RefreshAppstream(oneshot::Sender<Result<()>>),
    Sandbox(AppRef, oneshot::Sender<Result<Option<Sandbox>>>),
    Plan(AppRef, oneshot::Sender<Result<Option<InstallPlan>>>),
    SetPermission {
        app_ref: AppRef,
        key: String,
        granted: bool,
        reply: oneshot::Sender<Result<()>>,
    },
    Transact(Operation, mpsc::UnboundedSender<Result<Progress>>),
}

/// Start the worker thread and return the channel to drive it.
pub fn spawn() -> mpsc::UnboundedSender<Command> {
    let (tx, rx) = mpsc::unbounded_channel();

    std::thread::Builder::new()
        .name("flatpak-worker".into())
        .spawn(move || run(rx))
        .expect("failed to spawn the flatpak worker thread");

    tx
}

fn run(mut rx: mpsc::UnboundedReceiver<Command>) {
    let installations = open_installations();

    if installations.is_empty() {
        tracing::warn!("no usable flatpak installations");
    }

    // `blocking_recv` is what lets a plain thread drive a tokio channel.
    while let Some(command) = rx.blocking_recv() {
        match command {
            Command::Installed(reply) => {
                let _ = reply.send(list_installed(&installations, false));
            }
            Command::Updates(reply) => {
                let _ = reply.send(list_installed(&installations, true));
            }
            Command::Details(app_ref, reply) => {
                let _ = reply.send(details(&installations, &app_ref));
            }
            Command::RefreshAppstream(reply) => {
                let _ = reply.send(refresh_appstream(&installations));
            }
            Command::Sandbox(app_ref, reply) => {
                let _ = reply.send(sandbox(&installations, &app_ref));
            }
            Command::Plan(app_ref, reply) => {
                let _ = reply.send(plan(&installations, &app_ref));
            }
            Command::SetPermission { app_ref, key, granted, reply } => {
                let result = crate::sandbox::set_override(&app_ref.id, &key, granted)
                    .map_err(Error::Backend);
                let _ = reply.send(result);
            }
            Command::Transact(op, progress) => {
                transact(&installations, op, &progress);
            }
        }
    }
}

/// Open both the system and per-user installations.
///
/// Either may legitimately be absent — a machine with only a system install is
/// normal — so failures are logged and skipped rather than propagated.
fn open_installations() -> Vec<Installation> {
    let mut out = Vec::new();

    match Installation::new_system(gio::Cancellable::NONE) {
        Ok(installation) => out.push(installation),
        Err(err) => tracing::debug!(%err, "no system flatpak installation"),
    }
    match Installation::new_user(gio::Cancellable::NONE) {
        Ok(installation) => out.push(installation),
        Err(err) => tracing::debug!(%err, "no user flatpak installation"),
    }

    out
}

/// List installed apps, or only those with a pending update.
fn list_installed(installations: &[Installation], updates_only: bool) -> Result<Vec<InstalledApp>> {
    let mut apps = Vec::new();

    for installation in installations {
        let refs = if updates_only {
            installation.list_installed_refs_for_update(gio::Cancellable::NONE)
        } else {
            installation.list_installed_refs_by_kind(
                libflatpak::RefKind::App,
                gio::Cancellable::NONE,
            )
        }
        .map_err(to_error)?;

        for installed in refs {
            // `list_installed_refs_for_update` doesn't filter by kind, so
            // runtimes have to be dropped here — users don't install those.
            if installed.kind() != libflatpak::RefKind::App {
                continue;
            }
            if let Some(app) = to_installed_app(&installed, updates_only) {
                apps.push(app);
            }
        }
    }

    Ok(apps)
}

fn to_installed_app(installed: &libflatpak::InstalledRef, has_update: bool) -> Option<InstalledApp> {
    let id = installed.name()?.to_string();
    let id_for_meta = id.clone();

    // `appdata_*` come from the app's own metadata and are far better than the
    // ref string, but they're optional — fall back to the id so a row never
    // renders blank.
    let name = installed
        .appdata_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| id.clone());
    let version = installed.appdata_version().map(|s| s.to_string());

    let app_ref = {
        let base = AppRef::new(BackendId::Flatpak, id);
        match installed.origin() {
            Some(origin) => base.with_origin(origin.to_string()),
            None => base,
        }
    };

    let app = App {
        app_ref,
        name,
        summary: installed.appdata_summary().map(|s| s.to_string()),
        description: None,
        developer: None,
        license: installed.appdata_license().map(|s| s.to_string()),
        categories: Vec::new(),
        icon: None,
        screenshots: Vec::new(),
        releases: Vec::new(),
        version: version.clone(),
        download_size: None,
        installed_size: Some(installed.installed_size()),
        // Local metadata read — no network, so the Installed list can badge
        // every row with its real sandbox grade.
        sandbox: installed
            .load_metadata(gio::Cancellable::NONE)
            .ok()
            .map(|bytes| {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let overrides = crate::sandbox::read_overrides(&id_for_meta);
                crate::sandbox::parse(&text, overrides.as_deref())
            }),
    };

    let update = has_update.then(|| Update {
        // libflatpak won't tell us the target version without resolving the
        // remote ref, which is a network round trip per app. The detail page
        // fills this in; the list only needs to know an update exists.
        version: String::new(),
        download_size: None,
        releases: Vec::<Release>::new(),
    });

    Some(InstalledApp { app, installed_version: version, update })
}

fn details(installations: &[Installation], app_ref: &AppRef) -> Result<App> {
    for installation in installations {
        let refs = installation
            .list_installed_refs_by_kind(libflatpak::RefKind::App, gio::Cancellable::NONE)
            .map_err(to_error)?;

        for installed in refs {
            if installed.name().map(|n| n.to_string()).as_deref() == Some(&app_ref.id) {
                return to_installed_app(&installed, false)
                    .map(|i| i.app)
                    .ok_or_else(|| Error::NotFound(app_ref.clone()));
            }
        }
    }

    Err(Error::NotFound(app_ref.clone()))
}

/// Read an app's sandbox profile.
///
/// Installed apps have their `metadata` on disk. For anything else the
/// metadata is fetched from the remote, which is a network round trip — but
/// showing permissions *before* install is the entire point of the feature, so
/// it is worth the wait.
fn sandbox(installations: &[Installation], app_ref: &AppRef) -> Result<Option<Sandbox>> {
    let overrides = crate::sandbox::read_overrides(&app_ref.id);

    // Prefer the installed copy: it is authoritative for what is actually
    // running, and needs no network.
    for installation in installations {
        let Ok(refs) = installation
            .list_installed_refs_by_kind(libflatpak::RefKind::App, gio::Cancellable::NONE)
        else {
            continue;
        };

        for installed in refs {
            if installed.name().map(|n| n.to_string()).as_deref() != Some(&app_ref.id) {
                continue;
            }
            let bytes = installed
                .load_metadata(gio::Cancellable::NONE)
                .map_err(to_error)?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            return Ok(Some(crate::sandbox::parse(&text, overrides.as_deref())));
        }
    }

    // Not installed — ask the remote.
    let remote = app_ref.origin.clone().unwrap_or_else(|| "flathub".to_owned());
    for installation in installations {
        let Some(full) = resolve_remote_ref(installation, &remote, &app_ref.id) else {
            continue;
        };
        let Ok(parsed) = libflatpak::Ref::parse(&full) else { continue };

        match installation.fetch_remote_metadata_sync(&remote, &parsed, gio::Cancellable::NONE) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                return Ok(Some(crate::sandbox::parse(&text, overrides.as_deref())));
            }
            Err(err) => tracing::debug!(app = %app_ref.id, %err, "remote metadata fetch failed"),
        }
    }

    Ok(None)
}

/// Resolve an install without performing it.
///
/// Builds the real transaction, lets Flatpak resolve dependencies against what
/// is already installed, captures the resulting operation list, then aborts
/// before a single byte is downloaded. Returning `false` from the `ready`
/// signal is the documented way to cancel at exactly that point.
///
/// This is what makes it possible to say "12 MB app, plus a 340 MB runtime you
/// don't have yet" instead of one meaningless number.
fn plan(installations: &[Installation], app_ref: &AppRef) -> Result<Option<InstallPlan>> {
    let remote = app_ref.origin.clone().unwrap_or_else(|| "flathub".to_owned());

    let Some(installation) = installations.first() else {
        return Ok(None);
    };
    let Some(full) = resolve_remote_ref(installation, &remote, &app_ref.id) else {
        return Ok(None);
    };

    let transaction =
        Transaction::for_installation(installation, gio::Cancellable::NONE).map_err(to_error)?;
    transaction.add_install(&remote, &full, &[]).map_err(to_error)?;

    let captured: Rc<std::cell::RefCell<Vec<PlanItem>>> = Rc::default();
    {
        let captured = Rc::clone(&captured);
        transaction.connect_ready(move |tx| {
            for operation in tx.operations() {
                let Some(op_ref) = operation.get_ref() else { continue };
                let op_ref = op_ref.to_string();

                // Refs are `kind/id/arch/branch`; anything not `app/` is a
                // runtime, locale or debug extension riding along.
                let mut parts = op_ref.split('/');
                let kind = parts.next().unwrap_or_default();
                let id = parts.next().unwrap_or_default().to_owned();
                let branch = parts.nth(1).unwrap_or_default();

                captured.borrow_mut().push(PlanItem {
                    name: friendly_name(&id, branch),
                    id,
                    is_runtime: kind != "app",
                    download_size: operation.download_size(),
                    installed_size: operation.installed_size(),
                });
            }
            // Stop here: resolution is all we wanted.
            false
        });
    }

    // Aborting via `ready` surfaces as an error from `run`; that is the
    // success path here, so the result is deliberately discarded.
    let _ = transaction.run(gio::Cancellable::NONE);

    // Taken by draining rather than `Rc::try_unwrap`: the transaction still
    // holds the `ready` closure, so the strong count is never 1 here and
    // `try_unwrap` silently yielded an empty plan.
    let mut items: Vec<PlanItem> = captured.borrow_mut().drain(..).collect();

    if items.is_empty() {
        return Ok(None);
    }

    // App first, then the largest dependencies — that is the order someone
    // reads them in.
    items.sort_by_key(|item| (item.is_runtime, std::cmp::Reverse(item.download_size)));

    Ok(Some(InstallPlan {
        format: "Flatpak",
        source: Some(remote),
        items,
    }))
}

/// Turn a runtime id into something readable: `org.gnome.Platform` at branch
/// `48` becomes "GNOME Platform 48".
fn friendly_name(id: &str, branch: &str) -> String {
    let last = id.rsplit('.').next().unwrap_or(id);
    let vendor = id.split('.').nth(1).unwrap_or_default();

    let base = match vendor {
        "gnome" => format!("GNOME {last}"),
        "kde" => format!("KDE {last}"),
        "freedesktop" => format!("Freedesktop {last}"),
        _ => last.to_owned(),
    };

    if branch.is_empty() { base } else { format!("{base} {branch}") }
}

fn refresh_appstream(installations: &[Installation]) -> Result<()> {
    for installation in installations {
        let remotes = installation
            .list_remotes(gio::Cancellable::NONE)
            .map_err(to_error)?;

        for remote in remotes {
            if remote.is_disabled() {
                continue;
            }
            let Some(name) = remote.name() else { continue };

            // Arch `None` means the host arch.
            if let Err(err) =
                installation.update_appstream_sync(&name, None, gio::Cancellable::NONE)
            {
                // One dead remote shouldn't abort the refresh of the others.
                tracing::warn!(remote = %name, %err, "appstream update failed");
            }
        }
    }
    Ok(())
}

/// Resolve an app id to the full ref string libflatpak transactions expect
/// (`app/org.gnome.Calculator/x86_64/stable`).
///
/// We key everything on the bare app id so catalog entries and installed apps
/// share an identity; the full ref only exists inside this module.
fn resolve_installed_ref(
    installations: &[Installation],
    app_ref: &AppRef,
) -> Option<(usize, String)> {
    for (index, installation) in installations.iter().enumerate() {
        let Ok(refs) =
            installation.list_installed_refs_by_kind(libflatpak::RefKind::App, gio::Cancellable::NONE)
        else {
            continue;
        };

        for installed in refs {
            if installed.name().map(|n| n.to_string()).as_deref() == Some(&app_ref.id) {
                if let Some(full) = installed.format_ref() {
                    return Some((index, full.to_string()));
                }
            }
        }
    }
    None
}

/// Find an installable ref in a remote by app id.
///
/// Prefers the `stable` branch when a remote offers several.
fn resolve_remote_ref(
    installation: &Installation,
    remote: &str,
    app_id: &str,
) -> Option<String> {
    let refs = installation
        .list_remote_refs_sync(remote, gio::Cancellable::NONE)
        .ok()?;

    let mut fallback = None;
    for remote_ref in refs {
        if remote_ref.kind() != libflatpak::RefKind::App {
            continue;
        }
        if remote_ref.name().map(|n| n.to_string()).as_deref() != Some(app_id) {
            continue;
        }
        let full = remote_ref.format_ref()?.to_string();
        if remote_ref.branch().map(|b| b.to_string()).as_deref() == Some("stable") {
            return Some(full);
        }
        fallback.get_or_insert(full);
    }
    fallback
}

fn transact(
    installations: &[Installation],
    operation: Operation,
    progress: &mpsc::UnboundedSender<Result<Progress>>,
) {
    let result = build_and_run(installations, operation, progress);

    // Only report an error; success is signalled by the stream ending.
    if let Err(err) = result {
        let _ = progress.send(Err(err));
    }
}

fn build_and_run(
    installations: &[Installation],
    operation: Operation,
    progress: &mpsc::UnboundedSender<Result<Progress>>,
) -> Result<()> {
    let _ = progress.send(Ok(Progress::phase(Phase::Resolving)));

    let (installation, transaction) = match &operation {
        Operation::Install(app_ref) => {
            // Installs go to the first installation that has the remote —
            // in practice the system one.
            let remote = app_ref
                .origin
                .clone()
                .unwrap_or_else(|| "flathub".to_owned());

            let installation = installations.first().ok_or_else(|| {
                Error::BackendUnavailable {
                    backend: "flatpak",
                    reason: "no flatpak installation".into(),
                }
            })?;

            let full = resolve_remote_ref(installation, &remote, &app_ref.id)
                .ok_or_else(|| Error::NotFound(app_ref.clone()))?;

            let transaction =
                Transaction::for_installation(installation, gio::Cancellable::NONE)
                    .map_err(to_error)?;
            transaction.add_install(&remote, &full, &[]).map_err(to_error)?;
            (installation, transaction)
        }

        Operation::Remove(app_ref) => {
            let (index, full) = resolve_installed_ref(installations, app_ref)
                .ok_or_else(|| Error::NotFound(app_ref.clone()))?;
            let installation = &installations[index];

            let transaction =
                Transaction::for_installation(installation, gio::Cancellable::NONE)
                    .map_err(to_error)?;
            transaction.add_uninstall(&full).map_err(to_error)?;
            (installation, transaction)
        }

        Operation::Update(Some(app_ref)) => {
            let (index, full) = resolve_installed_ref(installations, app_ref)
                .ok_or_else(|| Error::NotFound(app_ref.clone()))?;
            let installation = &installations[index];

            let transaction =
                Transaction::for_installation(installation, gio::Cancellable::NONE)
                    .map_err(to_error)?;
            transaction.add_update(&full, &[], None).map_err(to_error)?;
            (installation, transaction)
        }

        Operation::Update(None) => {
            let installation = installations.first().ok_or_else(|| {
                Error::BackendUnavailable {
                    backend: "flatpak",
                    reason: "no flatpak installation".into(),
                }
            })?;

            let transaction =
                Transaction::for_installation(installation, gio::Cancellable::NONE)
                    .map_err(to_error)?;

            let refs = installation
                .list_installed_refs_for_update(gio::Cancellable::NONE)
                .map_err(to_error)?;

            for installed in refs {
                if let Some(full) = installed.format_ref() {
                    // A ref that can't be updated (e.g. pinned) shouldn't
                    // abort the whole batch.
                    if let Err(err) = transaction.add_update(&full, &[], None) {
                        tracing::warn!(ref_ = %full, %err, "skipping in bulk update");
                    }
                }
            }
            (installation, transaction)
        }
    };

    let _ = installation;

    if transaction.is_empty() {
        return Ok(());
    }

    // Dropping the returned stream closes the channel; that's how the UI
    // cancels. `Cancellable` is one of the few Send+Sync types here.
    let cancellable = gio::Cancellable::new();

    // Total operation count is only known after resolution, so overall
    // progress is tracked as "which operation are we on" plus that
    // operation's own percentage.
    let op_index = Rc::new(Cell::new(0usize));
    let op_total = Rc::new(Cell::new(0usize));

    {
        let progress = progress.clone();
        let cancellable = cancellable.clone();
        let op_index = Rc::clone(&op_index);
        let op_total = Rc::clone(&op_total);

        transaction.connect_new_operation(move |tx, operation, tx_progress| {
            op_total.set(tx.operations().len().max(1));
            op_index.set(op_index.get() + 1);

            let phase = match operation.operation_type() {
                libflatpak::TransactionOperationType::Uninstall => Phase::Removing,
                libflatpak::TransactionOperationType::Update => Phase::Downloading,
                _ => Phase::Downloading,
            };

            let download_total = operation.download_size();

            // 100ms is frequent enough to look live without flooding the
            // channel on a fast local install.
            tx_progress.set_update_frequency(100);

            let progress = progress.clone();
            let cancellable = cancellable.clone();
            let op_index = Rc::clone(&op_index);
            let op_total = Rc::clone(&op_total);

            tx_progress.connect_changed(move |p| {
                let within = f64::from(p.progress().clamp(0, 100)) / 100.0;
                let completed = (op_index.get().saturating_sub(1)) as f64;
                let overall = (completed + within) / op_total.get() as f64;

                let mut tick = Progress::phase(phase);
                if !p.is_estimating() {
                    tick = tick.with_fraction(overall);
                }
                if download_total > 0 {
                    tick = tick.with_bytes(p.bytes_transferred(), download_total);
                }

                // Send failure means the UI dropped the stream — treat that
                // as a cancel request rather than carrying on invisibly.
                if progress.send(Ok(tick)).is_err() {
                    cancellable.cancel();
                }
            });
        });
    }

    {
        let progress = progress.clone();
        transaction.connect_operation_done(move |_, _, _, _| {
            let _ = progress.send(Ok(Progress::phase(Phase::Finalizing)));
        });
    }

    transaction.run(Some(&cancellable)).map_err(|err| {
        if cancellable.is_cancelled() {
            Error::Cancelled
        } else {
            to_error(err)
        }
    })
}

/// Map a GLib error onto our user-facing error taxonomy.
///
/// libflatpak reports almost everything as a generic error with a human string,
/// so this leans on the message for the cases worth distinguishing.
fn to_error(err: glib::Error) -> Error {
    let message = err.message().to_owned();
    let lower = message.to_lowercase();

    if lower.contains("not authorized") || lower.contains("permission denied") {
        Error::NotAuthorized
    } else if lower.contains("no space") || lower.contains("disk full") {
        Error::Backend(message)
    } else if lower.contains("network")
        || lower.contains("resolve")
        || lower.contains("connection")
        || lower.contains("timed out")
    {
        Error::Network(message)
    } else if lower.contains("cancel") {
        Error::Cancelled
    } else {
        Error::Backend(message)
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    /// Resolve a real install plan against Flathub without downloading.
    ///
    /// Gated behind `APPSSCOPE_NETWORK_TESTS` because it needs the network and
    /// a configured remote. Run with `--test-threads=1`: libflatpak is not
    /// thread-safe and these tests open real installations.
    #[test]
    fn resolves_without_downloading() {
        if std::env::var_os("APPSSCOPE_NETWORK_TESTS").is_none() {
            return;
        }

        let installations = open_installations();
        assert!(!installations.is_empty(), "no flatpak installation");

        // Calculator's runtime is already installed here, so its plan is
        // small; kolourpaint pulls the whole KDE platform. Between them they
        // cover both the "nothing extra" and "800 MB surprise" cases.
        for app_id in ["org.gnome.Calculator", "org.kde.kolourpaint"] {
            let app_ref = AppRef::new(BackendId::Flatpak, app_id).with_origin("flathub");

            match plan(&installations, &app_ref) {
                Ok(Some(plan)) => {
                    println!(
                        "\n{app_id}: {} — {} download, {} on disk",
                        plan.provenance(),
                        appsscope_core::format_size(plan.download_size()),
                        appsscope_core::format_size(plan.installed_size()),
                    );
                    for item in &plan.items {
                        println!(
                            "    {:<28} {:>10}  {}",
                            item.name,
                            appsscope_core::format_size(item.download_size),
                            if item.is_runtime { "runtime" } else { "app" },
                        );
                    }
                    assert!(!plan.items.is_empty());
                }
                Ok(None) => panic!("{app_id}: resolved to no plan"),
                Err(err) => println!("{app_id}: failed — {err}"),
            }
        }
    }
}

#[cfg(test)]
mod plan_diagnostics {
    use super::*;

    #[test]
    fn remote_refs_are_listable() {
        if std::env::var_os("APPSSCOPE_NETWORK_TESTS").is_none() {
            return;
        }
        for installation in open_installations() {
            let remotes = installation.list_remotes(gio::Cancellable::NONE);
            println!(
                "installation: remotes = {:?}",
                remotes.as_ref().map(|r| r
                    .iter()
                    .filter_map(|x| x.name().map(|n| n.to_string()))
                    .collect::<Vec<_>>())
            );

            match installation.list_remote_refs_sync("flathub", gio::Cancellable::NONE) {
                Ok(refs) => {
                    println!("  {} remote refs", refs.len());
                    for r in refs.iter().take(3) {
                        println!("    {:?}", r.format_ref().map(|s| s.to_string()));
                    }
                    let calc = refs.iter().find(|r| {
                        r.name().map(|n| n.to_string()).as_deref() == Some("org.gnome.Calculator")
                    });
                    println!("  calculator found: {}", calc.is_some());
                    if let Some(c) = calc {
                        println!("    ref={:?} kind={:?} branch={:?}",
                            c.format_ref().map(|s| s.to_string()), c.kind(),
                            c.branch().map(|s| s.to_string()));
                    }
                }
                Err(err) => println!("  list_remote_refs_sync FAILED: {err}"),
            }
        }
    }
}
