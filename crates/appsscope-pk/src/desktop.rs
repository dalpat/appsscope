//! Mapping installed packages to the applications they provide.
//!
//! PackageKit's `APPLICATION` filter is meant to restrict results to
//! user-facing programs. On the apt backend it does not: querying installed
//! applications returns `at-spi2-core`, `gcr`, `gdm3` and
//! `evolution-data-server` — daemons and libraries nobody thinks of as apps.
//! It also returns no icon and no display name, only the package name, so
//! Visual Studio Code appears as "code" with a placeholder icon.
//!
//! Desktop entries answer all three questions at once: which packages ship a
//! visible application, what it is called, and which icon it uses. So this
//! reads `/usr/share/applications`, keeps the entries a menu would show, and
//! asks `dpkg` which package owns each one.
//!
//! The `dpkg` dependency makes this Debian-specific. That is a deliberate
//! narrowing: the alternative is one PackageKit `SearchFiles` transaction per
//! desktop file, which is hundreds of round trips. On non-Debian systems the
//! lookup yields nothing and the backend falls back to unfiltered results,
//! which is worse but still correct.

use std::collections::HashMap;
use std::sync::OnceLock;

/// What a desktop entry tells us about an application.
#[derive(Debug, Clone)]
pub struct Entry {
    /// `Name=`, e.g. "Visual Studio Code" rather than "code".
    pub name: String,
    /// `Icon=`, resolvable through the GTK icon theme.
    pub icon: Option<String>,
}

/// Package name to its application entry, built once per process.
pub fn applications() -> &'static HashMap<String, Entry> {
    static MAP: OnceLock<HashMap<String, Entry>> = OnceLock::new();
    MAP.get_or_init(build)
}

fn build() -> HashMap<String, Entry> {
    let mut entries: HashMap<std::path::PathBuf, Entry> = HashMap::new();

    for dir in [
        "/usr/share/applications",
        "/usr/local/share/applications",
    ] {
        let Ok(read) = std::fs::read_dir(dir) else { continue };

        for file in read.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            if let Some(entry) = parse(&text) {
                entries.insert(path, entry);
            }
        }
    }

    if entries.is_empty() {
        return HashMap::new();
    }

    let owners = owning_packages(entries.keys());

    let mut map = HashMap::new();
    for (path, entry) in entries {
        if let Some(package) = owners.get(&path) {
            // Several desktop files can belong to one package; first wins,
            // which is good enough for a display name.
            map.entry(package.clone()).or_insert(entry);
        }
    }

    tracing::debug!(packages = map.len(), "desktop entries mapped to packages");
    map
}

/// Parse the `[Desktop Entry]` group, keeping only what a menu would show.
fn parse(text: &str) -> Option<Entry> {
    let mut in_group = false;
    let (mut name, mut icon, mut kind) = (None, None, None);
    let mut hidden = false;

    for line in text.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            // Only the main group matters; per-action groups repeat Name= and
            // would otherwise overwrite the application's own.
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Name" => name = Some(value.trim().to_owned()),
            "Icon" => icon = Some(value.trim().to_owned()),
            "Type" => kind = Some(value.trim().to_owned()),
            // Both mean "do not show me in a menu", and neither should appear
            // in a list of installed applications.
            "NoDisplay" | "Hidden" if value.trim() == "true" => hidden = true,
            _ => {}
        }
    }

    if hidden || kind.as_deref() != Some("Application") {
        return None;
    }

    Some(Entry { name: name?, icon })
}

/// Ask dpkg which package owns each path, in one call.
fn owning_packages<'a>(
    paths: impl Iterator<Item = &'a std::path::PathBuf>,
) -> HashMap<std::path::PathBuf, String> {
    let args: Vec<String> = paths.map(|p| p.to_string_lossy().into_owned()).collect();
    if args.is_empty() {
        return HashMap::new();
    }

    let output = match std::process::Command::new("dpkg").arg("-S").args(&args).output() {
        Ok(output) => output,
        Err(err) => {
            tracing::debug!(%err, "dpkg unavailable; not filtering to desktop applications");
            return HashMap::new();
        }
    };

    // Lines look like `pkgname: /usr/share/applications/foo.desktop`. A path
    // owned by several packages produces `a, b: /path`; the first is fine.
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (packages, path) = line.split_once(": ")?;
            let package = packages.split(',').next()?.trim();
            Some((std::path::PathBuf::from(path.trim()), package.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_entries_a_menu_would_hide() {
        assert!(parse("[Desktop Entry]\nType=Application\nName=X\nNoDisplay=true\n").is_none());
        assert!(parse("[Desktop Entry]\nType=Application\nName=X\nHidden=true\n").is_none());
        assert!(parse("[Desktop Entry]\nType=Link\nName=X\n").is_none());
        assert!(parse("[Desktop Entry]\nType=Application\n").is_none());
    }

    #[test]
    fn reads_name_and_icon() {
        let entry = parse("[Desktop Entry]\nType=Application\nName=Code\nIcon=vscode\n").unwrap();
        assert_eq!(entry.name, "Code");
        assert_eq!(entry.icon.as_deref(), Some("vscode"));
    }

    #[test]
    fn ignores_action_groups() {
        // Desktop actions repeat `Name=`; the application's own must win.
        let entry = parse(
            "[Desktop Entry]\nType=Application\nName=Firefox\n\
             [Desktop Action new-window]\nName=New Window\n",
        )
        .unwrap();
        assert_eq!(entry.name, "Firefox");
    }
}
