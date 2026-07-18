//! Domain types shared by every backend and the UI.
//!
//! These are deliberately backend-agnostic: a Flatpak ref, a deb, and an
//! AppImage all collapse into the same `App`. Anything a single backend needs
//! but the others don't belongs in that backend's crate, not here.

use std::fmt;

/// Which package system an app comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BackendId {
    Flatpak,
    PackageKit,
    AppImage,
}

impl BackendId {
    /// Short lowercase name, used in cache keys and on disk.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flatpak => "flatpak",
            Self::PackageKit => "packagekit",
            Self::AppImage => "appimage",
        }
    }

    /// Name shown to users when the same app exists in several backends.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Flatpak => "Flatpak",
            Self::PackageKit => "System",
            Self::AppImage => "AppImage",
        }
    }

    /// Short label for the format pill shown on every app.
    ///
    /// Kept to one word so it fits beside a name in a list row. "System"
    /// rather than "deb" because the PackageKit backend covers apt, dnf,
    /// pacman and zypper — claiming "deb" would be wrong on four of five
    /// distributions.
    pub const fn badge_label(self) -> &'static str {
        match self {
            Self::Flatpak => "Flatpak",
            Self::PackageKit => "System",
            Self::AppImage => "AppImage",
        }
    }

    /// Style class for the format pill, so formats are colour-coded and
    /// scannable once more than one backend is active.
    pub const fn css_class(self) -> &'static str {
        match self {
            Self::Flatpak => "format-flatpak",
            Self::PackageKit => "format-system",
            Self::AppImage => "format-appimage",
        }
    }
}

impl fmt::Display for BackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Uniquely identifies one installable thing.
///
/// `id` is whatever the backend uses natively — a Flatpak ref like
/// `app/org.gnome.Calculator/x86_64/stable`, a deb package name, an AppImage
/// path. Never parse it outside the owning backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppRef {
    pub backend: BackendId,
    pub id: String,
    /// Remote / repo / origin the app came from, if the backend has a concept
    /// of one (`flathub`, `noble-updates`, …).
    pub origin: Option<String>,
}

impl AppRef {
    pub fn new(backend: BackendId, id: impl Into<String>) -> Self {
        Self { backend, id: id.into(), origin: None }
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }
}

impl fmt::Display for AppRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.backend, self.id)
    }
}

/// A single screenshot from AppStream metadata.
#[derive(Debug, Clone)]
pub struct Screenshot {
    pub url: String,
    pub caption: Option<String>,
    /// AppStream marks exactly one screenshot as the default; show it first.
    pub is_default: bool,
}

/// One entry in an app's version history, from AppStream `<releases>`.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    /// Unix timestamp; AppStream gives either a date or a raw timestamp.
    pub timestamp: Option<i64>,
    /// Changelog. May contain a small subset of HTML.
    pub description: Option<String>,
}

/// An app as presented to the user, merged from package data + AppStream.
#[derive(Debug, Clone)]
pub struct App {
    pub app_ref: AppRef,
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub developer: Option<String>,
    pub license: Option<String>,
    pub categories: Vec<String>,
    pub icon: Option<IconSource>,
    pub screenshots: Vec<Screenshot>,
    pub releases: Vec<Release>,
    pub version: Option<String>,
    /// Bytes to download. `None` when the backend can't say cheaply.
    pub download_size: Option<u64>,
    /// Bytes on disk after install.
    pub installed_size: Option<u64>,
    /// Sandbox profile, when known.
    ///
    /// `None` means "not looked up yet", not "unconfined" — for catalog
    /// entries this requires a network round trip, so it is filled in lazily
    /// on the detail page. Installed apps get it for free from local metadata.
    pub sandbox: Option<crate::sandbox::Sandbox>,
}

/// Where an icon comes from. Resolved lazily — never block the UI on this.
#[derive(Debug, Clone)]
pub enum IconSource {
    /// Named icon in the current GTK icon theme.
    Themed(String),
    /// Already-cached absolute path.
    Cached(std::path::PathBuf),
    /// Needs downloading; the catalog cache will localise it.
    Remote(String),
}

/// An installed app, plus whatever the backend knows about updates.
#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub app: App,
    pub installed_version: Option<String>,
    /// Set when a newer version is available.
    pub update: Option<Update>,
}

#[derive(Debug, Clone)]
pub struct Update {
    pub version: String,
    pub download_size: Option<u64>,
    /// Changelog entries between installed and available version.
    pub releases: Vec<Release>,
}

/// Coarse stage of a transaction. Drives the label above the progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Working out what to install, pulling in dependencies and runtimes.
    Resolving,
    Downloading,
    Installing,
    Removing,
    /// Deploying, updating the desktop database, triggers.
    Finalizing,
}

impl Phase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Resolving => "Preparing",
            Self::Downloading => "Downloading",
            Self::Installing => "Installing",
            Self::Removing => "Removing",
            Self::Finalizing => "Finishing up",
        }
    }
}

/// One progress tick from an in-flight transaction.
///
/// Emitted as a stream rather than reported via callback so cancellation is
/// just dropping the stream, and so the UI can render partial state without
/// every backend inventing its own channel protocol.
#[derive(Debug, Clone)]
pub struct Progress {
    pub phase: Phase,
    /// 0.0..=1.0 within the whole transaction, not just the current phase.
    /// Backends that genuinely cannot estimate should report `None` and the
    /// UI will show a pulsing bar.
    pub fraction: Option<f64>,
    pub bytes_done: Option<u64>,
    pub bytes_total: Option<u64>,
}

impl Progress {
    pub fn phase(phase: Phase) -> Self {
        Self { phase, fraction: None, bytes_done: None, bytes_total: None }
    }

    pub fn with_fraction(mut self, fraction: f64) -> Self {
        self.fraction = Some(fraction.clamp(0.0, 1.0));
        self
    }

    pub fn with_bytes(mut self, done: u64, total: u64) -> Self {
        self.bytes_done = Some(done);
        self.bytes_total = Some(total);
        self
    }
}
