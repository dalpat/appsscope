//! Sandbox permissions — what an app can actually reach on your machine.
//!
//! This is the part existing software centres get wrong. GNOME Software
//! collapses the whole question into a single "Potentially unsafe" label;
//! Discover barely mentions it. Meanwhile a large share of Flathub apps ship
//! `filesystems=host`, which is unrestricted access to everything you own, and
//! the store presents them identically to a properly confined app.
//!
//! The model here is deliberately opinionated: permissions are graded, phrased
//! in plain language, and explained in terms of what someone stands to lose —
//! not in terms of the flag that produced them.

use std::fmt;

/// How much a permission actually gives away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    /// Confined to the sandbox, or a capability with no meaningful reach.
    Safe,
    /// Real access, but bounded and usually expected for the app's purpose.
    Caution,
    /// Defeats the sandbox, or reaches personal data wholesale.
    Critical,
}

impl Risk {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Safe => "Sandboxed",
            Self::Caution => "Limited access",
            Self::Critical => "Full access",
        }
    }

    /// One line explaining the grade as a whole.
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Safe => "This app runs fully confined and cannot reach your files.",
            Self::Caution => "This app can reach some things outside its sandbox.",
            Self::Critical => "This app can access your personal files and is not meaningfully sandboxed.",
        }
    }

    /// libadwaita style class for the badge.
    pub const fn css_class(self) -> &'static str {
        match self {
            Self::Safe => "risk-safe",
            Self::Caution => "risk-caution",
            Self::Critical => "risk-critical",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            Self::Safe => "security-high-symbolic",
            Self::Caution => "security-medium-symbolic",
            Self::Critical => "security-low-symbolic",
        }
    }
}

impl fmt::Display for Risk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The kinds of access we recognise and can explain.
///
/// Anything not covered lands in [`PermissionKind::Other`] and is still shown —
/// silently dropping a permission we don't understand would be the one
/// unforgivable bug in a feature like this.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PermissionKind {
    /// Full filesystem access — `filesystems=host`.
    HostFilesystem,
    /// Everything in your home directory — `filesystems=home`.
    HomeFilesystem,
    /// A specific path outside the sandbox.
    Filesystem(String),
    Network,
    /// X11 — any X11 client can read every other window's keystrokes.
    X11,
    Wayland,
    Audio,
    /// Unrestricted device access — `devices=all`, which includes webcams.
    AllDevices,
    /// GPU access. Ordinary for anything that draws.
    Gpu,
    /// Direct access to the session or system bus — a sandbox escape.
    SessionBus,
    SystemBus,
    /// Can drive Flatpak itself, i.e. can grant itself anything else.
    FlatpakEscape,
    /// Can read stored passwords and keys.
    Secrets,
    /// Interprocess communication with the host.
    Ipc,
    Other(String),
}

/// One permission, phrased for a human.
#[derive(Debug, Clone)]
pub struct Permission {
    pub kind: PermissionKind,
    pub risk: Risk,
    /// Short plain-language statement of what the app can do.
    pub summary: String,
    /// Why it matters — the consequence, not the mechanism.
    pub detail: String,
    /// The override token used to revoke this, e.g. `filesystems:host`.
    /// `None` when the permission can't be revoked without breaking the app.
    pub override_key: Option<String>,
    /// Whether the user has already revoked it via an override.
    pub revoked: bool,
}

/// An app's complete sandbox profile.
#[derive(Debug, Clone, Default)]
pub struct Sandbox {
    pub permissions: Vec<Permission>,
}

impl Sandbox {
    /// The worst permission decides the overall grade.
    ///
    /// Deliberately not an average: one sandbox escape is not offset by ten
    /// harmless capabilities, and presenting it that way would mislead.
    /// Revoked permissions don't count — that's the point of revoking them.
    pub fn risk(&self) -> Risk {
        self.permissions
            .iter()
            .filter(|p| !p.revoked)
            .map(|p| p.risk)
            .max()
            .unwrap_or(Risk::Safe)
    }

    /// Permissions worth showing first — highest risk, then stable order.
    pub fn sorted(&self) -> Vec<&Permission> {
        let mut out: Vec<&Permission> = self.permissions.iter().collect();
        out.sort_by_key(|p| std::cmp::Reverse(p.risk));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.permissions.is_empty()
    }

    /// A one-line verdict for list rows and cards.
    pub fn headline(&self) -> String {
        let risk = self.risk();
        let critical = self
            .permissions
            .iter()
            .filter(|p| !p.revoked && p.risk == Risk::Critical)
            .count();

        match (risk, critical) {
            (Risk::Safe, _) => "Fully sandboxed".to_owned(),
            (Risk::Caution, _) => "Limited access outside its sandbox".to_owned(),
            (Risk::Critical, 1) => "1 permission defeats the sandbox".to_owned(),
            (Risk::Critical, n) => format!("{n} permissions defeat the sandbox"),
        }
    }
}
