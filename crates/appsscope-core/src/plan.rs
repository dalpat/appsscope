//! What an install will actually do, worked out before anything is downloaded.
//!
//! "Install" is not one thing. A 12 MB app can pull a 350 MB runtime; a deb can
//! pull forty dependencies; the same app may exist as a Flatpak from Flathub
//! and a snap from the Snap Store with completely different confinement. Stores
//! generally show one number, if that, and never name the format.
//!
//! An [`InstallPlan`] is the resolved answer: the packaging format, where it
//! comes from, every item that will be fetched, and what each one costs.

/// One thing that will be downloaded and installed.
#[derive(Debug, Clone)]
pub struct PlanItem {
    /// Identifier, e.g. `org.gnome.Calculator` or `org.gnome.Platform`.
    pub id: String,
    /// Human-facing name, where the backend can give one.
    pub name: String,
    /// Shared runtimes and extensions, as opposed to the app itself.
    pub is_runtime: bool,
    pub download_size: u64,
    pub installed_size: u64,
}

/// The complete effect of installing one app.
#[derive(Debug, Clone)]
pub struct InstallPlan {
    /// Packaging format, e.g. "Flatpak". Shown verbatim.
    pub format: &'static str,
    /// Where it comes from — remote, repository, or archive.
    pub source: Option<String>,
    /// Everything to be fetched, the app first.
    ///
    /// Items already present are absent by construction: the backend resolves
    /// against what is installed, so this is what will *actually* be
    /// downloaded, not a nominal dependency list.
    pub items: Vec<PlanItem>,
}

impl InstallPlan {
    pub fn download_size(&self) -> u64 {
        self.items.iter().map(|item| item.download_size).sum()
    }

    pub fn installed_size(&self) -> u64 {
        self.items.iter().map(|item| item.installed_size).sum()
    }

    /// Extra items beyond the app itself — the surprise in "this is a 12 MB app".
    pub fn dependencies(&self) -> impl Iterator<Item = &PlanItem> {
        self.items.iter().filter(|item| item.is_runtime)
    }

    pub fn has_dependencies(&self) -> bool {
        self.dependencies().next().is_some()
    }

    /// One-line summary: "Flatpak from Flathub".
    pub fn provenance(&self) -> String {
        match &self.source {
            Some(source) => format!("{} from {source}", self.format),
            None => self.format.to_owned(),
        }
    }
}

/// Bytes as a short human string. Decimal units, matching how download sizes
/// are quoted everywhere else.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "kB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
