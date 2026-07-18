//! Reading and rewriting an app's sandbox profile.
//!
//! Two halves:
//!
//! - [`parse`] turns Flatpak's `metadata` keyfile into graded, plain-language
//!   [`Permission`]s.
//! - [`set_override`] writes `~/.local/share/flatpak/overrides/<app-id>`, the
//!   same file `flatpak override --user` and Flatseal manage. Writing it
//!   directly rather than shelling out keeps this synchronous and avoids
//!   depending on the CLI's argument parsing.
//!
//! The grading is the opinionated part. It's written so the worst cases —
//! whole-filesystem access, direct bus access, Flatpak-controlling-Flatpak —
//! cannot hide behind a wall of harmless capabilities.

use std::collections::HashSet;
use std::path::PathBuf;

use appsscope_core::{Permission, PermissionKind, Risk, Sandbox};
use libflatpak::glib;
use libflatpak::glib::KeyFile;

/// Parse an app's `metadata`, folding in any overrides the user has set.
pub fn parse(metadata: &str, overrides: Option<&str>) -> Sandbox {
    let keyfile = KeyFile::new();
    if keyfile
        .load_from_data(metadata, glib::KeyFileFlags::NONE)
        .is_err()
    {
        return Sandbox::default();
    }

    let revoked = overrides.map(revoked_keys).unwrap_or_default();
    let mut permissions = Vec::new();

    for (group, key) in [("Context", "filesystems"), ("Context", "shared"),
                         ("Context", "sockets"), ("Context", "devices")] {
        for value in list(&keyfile, group, key) {
            // Overrides can appear inside metadata too; a leading `!` is a
            // removal, not a grant.
            if value.starts_with('!') {
                continue;
            }
            if let Some(permission) = classify(key, &value, &revoked) {
                permissions.push(permission);
            }
        }
    }

    // Bus access is expressed as one key per peer, not a list.
    for group in ["Session Bus Policy", "System Bus Policy"] {
        for name in keyfile.keys(group).unwrap_or_default() {
            let name = name.to_string();
            let policy = keyfile
                .value(group, &name)
                .map(|v| v.to_string())
                .unwrap_or_default();

            // `see` is far weaker than `talk`/`own` and isn't worth alarming
            // anyone about.
            if policy == "see" {
                continue;
            }
            if let Some(permission) = classify_bus(&name, &revoked) {
                permissions.push(permission);
            }
        }
    }

    // Two apps can declare the same capability twice across keys.
    permissions.dedup_by(|a, b| a.kind == b.kind);

    Sandbox { permissions }
}

fn list(keyfile: &KeyFile, group: &str, key: &str) -> Vec<String> {
    keyfile
        .string_list(group, key)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Override keys the user has already revoked, as `key:value` pairs.
fn revoked_keys(overrides: &str) -> HashSet<String> {
    let keyfile = KeyFile::new();
    if keyfile
        .load_from_data(overrides, glib::KeyFileFlags::NONE)
        .is_err()
    {
        return HashSet::new();
    }

    let mut revoked = HashSet::new();
    for key in ["filesystems", "shared", "sockets", "devices"] {
        for value in list(&keyfile, "Context", key) {
            // In an overrides file, `!x` means "take x away".
            if let Some(stripped) = value.strip_prefix('!') {
                revoked.insert(format!("{key}:{stripped}"));
            }
        }
    }
    revoked
}

fn classify(key: &str, value: &str, revoked: &HashSet<String>) -> Option<Permission> {
    let override_key = format!("{key}:{value}");
    let is_revoked = revoked.contains(&override_key);

    // Read-only mounts are written `path:ro`; grade the path, note the mode.
    let (bare, read_only) = match value.strip_suffix(":ro") {
        Some(path) => (path, true),
        None => (value, false),
    };

    let (kind, risk, summary, detail) = match (key, bare) {
        ("filesystems", "host") => (
            PermissionKind::HostFilesystem,
            Risk::Critical,
            "Read and change every file on this computer".to_owned(),
            "Nothing on your disk is off limits — documents, photos, saved passwords, \
             other apps' data, and system files. The sandbox is not protecting you."
                .to_owned(),
        ),
        ("filesystems", "host-os") | ("filesystems", "host-etc") => (
            PermissionKind::Filesystem(bare.to_owned()),
            Risk::Caution,
            "Read system files and configuration".to_owned(),
            "Can inspect how this machine is set up, though not your personal files."
                .to_owned(),
        ),
        ("filesystems", "home") => (
            PermissionKind::HomeFilesystem,
            Risk::Critical,
            "Read and change everything in your home folder".to_owned(),
            "Every document, photo, and config file you own, including data belonging \
             to other applications."
                .to_owned(),
        ),
        ("filesystems", path) => {
            // xdg-run/* is the sandbox's own runtime dir — routine plumbing.
            let routine = path.starts_with("xdg-run/") || path == "xdg-download";
            (
                PermissionKind::Filesystem(path.to_owned()),
                if routine { Risk::Safe } else { Risk::Caution },
                format!(
                    "{} {}",
                    if read_only { "Read" } else { "Read and change" },
                    describe_path(path)
                ),
                format!("Access is limited to {}.", describe_path(path)),
            )
        }

        ("shared", "network") => (
            PermissionKind::Network,
            Risk::Caution,
            "Access the internet".to_owned(),
            "Can send and receive data, including anything else it is able to read."
                .to_owned(),
        ),
        ("shared", "ipc") => (
            PermissionKind::Ipc,
            Risk::Safe,
            "Communicate with the desktop".to_owned(),
            "Standard plumbing used for things like window management.".to_owned(),
        ),

        ("sockets", "x11") | ("sockets", "fallback-x11") => (
            PermissionKind::X11,
            Risk::Caution,
            "Use the older X11 display system".to_owned(),
            "X11 gives no isolation between windows: an app using it can read what you \
             type into other applications and capture their contents."
                .to_owned(),
        ),
        ("sockets", "wayland") => (
            PermissionKind::Wayland,
            Risk::Safe,
            "Draw its window".to_owned(),
            "Wayland isolates applications from one another.".to_owned(),
        ),
        ("sockets", "pulseaudio") => (
            PermissionKind::Audio,
            Risk::Caution,
            "Play and record audio".to_owned(),
            "Includes microphone access — this permission does not distinguish between \
             playback and recording."
                .to_owned(),
        ),
        ("sockets", "session-bus") => (
            PermissionKind::SessionBus,
            Risk::Critical,
            "Talk to any program in your session".to_owned(),
            "Unrestricted session bus access lets it drive other applications, which is \
             enough to work around most other restrictions."
                .to_owned(),
        ),
        ("sockets", "system-bus") => (
            PermissionKind::SystemBus,
            Risk::Critical,
            "Talk to system services".to_owned(),
            "Can reach privileged services that manage the machine itself.".to_owned(),
        ),
        ("sockets", "ssh-auth") => (
            PermissionKind::Other("ssh-auth".to_owned()),
            Risk::Critical,
            "Use your SSH keys".to_owned(),
            "Can authenticate to any server you can, as you.".to_owned(),
        ),
        ("sockets", "cups") => (
            PermissionKind::Other("cups".to_owned()),
            Risk::Safe,
            "Print".to_owned(),
            "Can send documents to your printers.".to_owned(),
        ),

        ("devices", "all") => (
            PermissionKind::AllDevices,
            Risk::Critical,
            "Use every device, including your camera".to_owned(),
            "Unrestricted device access covers webcams, microphones, and connected \
             hardware, with no further prompt."
                .to_owned(),
        ),
        ("devices", "dri") => (
            PermissionKind::Gpu,
            Risk::Safe,
            "Use the graphics card".to_owned(),
            "Needed by anything that renders or plays video.".to_owned(),
        ),
        ("devices", other) => (
            PermissionKind::Other(other.to_owned()),
            Risk::Caution,
            format!("Use {other} devices"),
            "Direct hardware access.".to_owned(),
        ),

        (_, other) => (
            PermissionKind::Other(other.to_owned()),
            Risk::Caution,
            format!("{key}: {other}"),
            "AppsScope does not recognise this permission, so it is shown unchanged \
             rather than hidden."
                .to_owned(),
        ),
    };

    Some(Permission {
        kind,
        risk,
        summary,
        detail,
        // Wayland and GPU access are load-bearing: revoking them stops the app
        // drawing at all, so we don't offer a switch that only breaks things.
        override_key: match (&risk, key, bare) {
            (_, "sockets", "wayland") | (_, "devices", "dri") => None,
            _ => Some(override_key),
        },
        revoked: is_revoked,
    })
}

fn classify_bus(name: &str, revoked: &HashSet<String>) -> Option<Permission> {
    let override_key = format!("bus:{name}");
    let is_revoked = revoked.contains(&override_key);

    let (kind, risk, summary, detail) = match name {
        // The escape hatch: an app that can talk to Flatpak can rewrite its
        // own permissions, which makes every other restriction advisory.
        n if n.starts_with("org.freedesktop.Flatpak") => (
            PermissionKind::FlatpakEscape,
            Risk::Critical,
            "Control Flatpak itself".to_owned(),
            "This lets the app run commands outside its sandbox and change its own \
             permissions. Every other restriction here is effectively voluntary."
                .to_owned(),
        ),
        "org.freedesktop.secrets" => (
            PermissionKind::Secrets,
            Risk::Critical,
            "Read your saved passwords".to_owned(),
            "Access to the system keyring, where applications store credentials."
                .to_owned(),
        ),
        n if n.starts_with("org.freedesktop.impl.portal") => (
            PermissionKind::Other(n.to_owned()),
            Risk::Caution,
            "Provide desktop dialogs".to_owned(),
            "Implements portal dialogs on behalf of the desktop.".to_owned(),
        ),
        // Notifications, status icons, session management and similar are
        // routine and would only add noise.
        _ => return None,
    };

    Some(Permission {
        kind,
        risk,
        summary,
        detail,
        override_key: Some(override_key),
        revoked: is_revoked,
    })
}

fn describe_path(path: &str) -> String {
    match path {
        "xdg-download" => "your Downloads folder".to_owned(),
        "xdg-documents" => "your Documents folder".to_owned(),
        "xdg-pictures" => "your Pictures folder".to_owned(),
        "xdg-music" => "your Music folder".to_owned(),
        "xdg-videos" => "your Videos folder".to_owned(),
        "xdg-config" => "application settings".to_owned(),
        "xdg-cache" => "cached data".to_owned(),
        other if other.starts_with("xdg-run/") => {
            format!("the {} runtime directory", other.trim_start_matches("xdg-run/"))
        }
        other => other.to_owned(),
    }
}

/// Path to the per-user override file for an app.
pub fn overrides_path(app_id: &str) -> Option<PathBuf> {
    let base = if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(data)
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".local/share")
    };
    Some(base.join("flatpak").join("overrides").join(app_id))
}

pub fn read_overrides(app_id: &str) -> Option<String> {
    std::fs::read_to_string(overrides_path(app_id)?).ok()
}

/// Revoke (`granted == false`) or restore (`granted == true`) one permission.
///
/// Writes the same file `flatpak override --user` maintains, so changes made
/// here are visible to Flatseal and the CLI, and vice versa.
pub fn set_override(app_id: &str, key: &str, granted: bool) -> Result<(), String> {
    let path = overrides_path(app_id).ok_or("cannot locate the overrides directory")?;

    let keyfile = KeyFile::new();
    // A missing file is the normal case for an app that has never been
    // overridden; only a malformed existing file is an error.
    if path.exists() {
        keyfile
            .load_from_file(&path, glib::KeyFileFlags::KEEP_COMMENTS)
            .map_err(|e| format!("failed to read existing overrides: {e}"))?;
    }

    let (group_key, value) = key.split_once(':').ok_or("malformed permission key")?;

    if group_key == "bus" {
        // Bus peers are revoked by setting the policy to `none`, not by `!`.
        if granted {
            let _ = keyfile.remove_key("Session Bus Policy", value);
        } else {
            keyfile.set_string("Session Bus Policy", value, "none");
        }
    } else {
        let negated = format!("!{value}");
        let mut current: Vec<String> = list(&keyfile, "Context", group_key);

        current.retain(|v| v != &negated);
        if !granted {
            current.push(negated);
        }

        if current.is_empty() {
            let _ = keyfile.remove_key("Context", group_key);
        } else {
            // The glib binding exposes no string-list setter, and Flatpak's
            // own format is a semicolon-terminated list, so it's written
            // directly: `filesystems=!host;!home;`.
            keyfile.set_string("Context", group_key, &format!("{};", current.join(";")));
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{e}"))?;
    }

    keyfile
        .save_to_file(&path)
        .map_err(|e| format!("failed to write overrides: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSMISSION: &str = "\
[Application]
name=com.transmissionbt.Transmission

[Context]
shared=ipc;network;
sockets=fallback-x11;pulseaudio;wayland;
devices=dri;
filesystems=xdg-run/gvfsd;host;
";

    #[test]
    fn host_filesystem_is_critical() {
        let sandbox = parse(TRANSMISSION, None);
        assert_eq!(sandbox.risk(), Risk::Critical);
        assert!(
            sandbox
                .permissions
                .iter()
                .any(|p| p.kind == PermissionKind::HostFilesystem)
        );
    }

    #[test]
    fn wayland_and_gpu_are_not_revocable() {
        // Offering a switch that only breaks the app is worse than offering none.
        let sandbox = parse(TRANSMISSION, None);
        for permission in &sandbox.permissions {
            if matches!(permission.kind, PermissionKind::Wayland | PermissionKind::Gpu) {
                assert!(permission.override_key.is_none());
            }
        }
    }

    #[test]
    fn revoking_host_access_lowers_the_grade() {
        let overrides = "[Context]\nfilesystems=!host;\n";
        let sandbox = parse(TRANSMISSION, Some(overrides));

        let host = sandbox
            .permissions
            .iter()
            .find(|p| p.kind == PermissionKind::HostFilesystem)
            .expect("host permission should still be listed");

        assert!(host.revoked, "should be marked revoked");
        // Network remains, so it drops to Caution rather than Safe.
        assert_eq!(sandbox.risk(), Risk::Caution);
    }

    #[test]
    fn flatpak_bus_access_is_an_escape() {
        let metadata = "\
[Application]
name=test

[Session Bus Policy]
org.freedesktop.Flatpak=talk
";
        let sandbox = parse(metadata, None);
        assert_eq!(sandbox.risk(), Risk::Critical);
        assert!(
            sandbox
                .permissions
                .iter()
                .any(|p| p.kind == PermissionKind::FlatpakEscape)
        );
    }

    #[test]
    fn unknown_permissions_are_shown_not_hidden() {
        let metadata = "[Context]\nsockets=some-future-socket;\n";
        let sandbox = parse(metadata, None);
        assert_eq!(sandbox.permissions.len(), 1);
    }
}

#[cfg(test)]
mod real_world {
    use super::*;

    /// Parse every app actually installed on this machine.
    ///
    /// Skips silently when Flatpak isn't installed so CI stays green; the
    /// value is in catching real-world metadata shapes the unit tests invent.
    #[test]
    fn parses_installed_apps() {
        let root = std::path::Path::new("/var/lib/flatpak/app");
        let Ok(entries) = std::fs::read_dir(root) else { return };

        for entry in entries.flatten() {
            let metadata = entry.path().join("current/active/metadata");
            let Ok(text) = std::fs::read_to_string(&metadata) else { continue };

            let app_id = entry.file_name().to_string_lossy().into_owned();
            let sandbox = parse(&text, read_overrides(&app_id).as_deref());

            println!("{:<45} {:<14} {}", app_id, sandbox.risk().label(), sandbox.headline());
            for permission in sandbox.sorted() {
                println!("      [{:?}] {}", permission.risk, permission.summary);
            }
        }
    }
}

#[cfg(test)]
mod overrides_roundtrip {
    use super::*;

    /// Write an override into a scratch data dir and read it back.
    ///
    /// Uses `APPSSCOPE_TEST_DATA_HOME` rather than mutating `XDG_DATA_HOME`,
    /// so it can never touch the real Flatpak configuration.
    #[test]
    fn writes_a_parseable_override_file() {
        let Some(dir) = std::env::var_os("APPSSCOPE_TEST_DATA_HOME") else { return };
        // SAFETY: the harness runs this test with --test-threads=1.
        unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };

        let app = "org.example.Test";
        set_override(app, "filesystems:host", false).expect("revoke host");
        set_override(app, "shared:network", false).expect("revoke network");
        // Re-granting must remove the negation, not add a second entry.
        set_override(app, "shared:network", true).expect("restore network");

        let written = read_overrides(app).expect("override file should exist");
        println!("--- written override ---\n{written}");

        let parsed = parse(
            "[Context]\nfilesystems=host;\nshared=network;\n",
            Some(&written),
        );
        let host = parsed
            .permissions
            .iter()
            .find(|p| p.kind == PermissionKind::HostFilesystem)
            .unwrap();
        let net = parsed
            .permissions
            .iter()
            .find(|p| p.kind == PermissionKind::Network)
            .unwrap();

        assert!(host.revoked, "host should stay revoked");
        assert!(!net.revoked, "network should have been restored");
    }
}
