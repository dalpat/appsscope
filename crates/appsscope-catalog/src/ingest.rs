//! Discovering AppStream catalogs on disk and loading them into SQLite.

use std::path::{Path, PathBuf};

use appstream::enums::{Category, ComponentKind, Icon};
use appstream::{Collection, Component, MarkupTranslatableString, TranslatableString};
use rusqlite::{Connection, OptionalExtension, params};

use crate::Error;

/// One catalog file plus the icon directory that goes with it.
#[derive(Debug, Clone)]
pub struct Source {
    /// Stable identity for this source, e.g. `flatpak:flathub:x86_64`.
    pub key: String,
    pub origin: String,
    pub catalog: PathBuf,
    pub icons: PathBuf,
    /// Changes when the catalog content changes; used to skip re-ingest.
    pub fingerprint: String,
}

/// Flatpak keeps appstream data per installation, remote, and arch:
/// `<install>/appstream/<remote>/<arch>/active/appstream.xml.gz`, where
/// `active` is a symlink to the current commit — which makes an ideal
/// fingerprint, since it changes exactly when the catalog does.
pub fn discover_flatpak_sources() -> Vec<Source> {
    let mut roots = vec![PathBuf::from("/var/lib/flatpak/appstream")];
    if let Some(user) = user_flatpak_dir() {
        roots.push(user.join("appstream"));
    }

    let mut sources = Vec::new();
    for root in roots {
        let Ok(remotes) = std::fs::read_dir(&root) else { continue };

        for remote in remotes.flatten() {
            let Ok(arches) = std::fs::read_dir(remote.path()) else { continue };
            let remote_name = remote.file_name().to_string_lossy().into_owned();

            for arch in arches.flatten() {
                let active = arch.path().join("active");
                let catalog = active.join("appstream.xml.gz");
                if !catalog.exists() {
                    continue;
                }

                // Resolve the symlink for the commit hash. If it isn't a
                // symlink, fall back to mtime so we still detect changes.
                let fingerprint = std::fs::read_link(&active)
                    .map(|target| target.to_string_lossy().into_owned())
                    .or_else(|_| fingerprint_from_mtime(&catalog))
                    .unwrap_or_default();

                sources.push(Source {
                    key: format!(
                        "flatpak:{remote_name}:{}",
                        arch.file_name().to_string_lossy()
                    ),
                    origin: remote_name.clone(),
                    catalog,
                    icons: active.join("icons"),
                    fingerprint,
                });
            }
        }
    }
    sources
}

fn fingerprint_from_mtime(path: &Path) -> Result<String, std::io::Error> {
    let modified = path.metadata()?.modified()?;
    Ok(format!("{modified:?}"))
}

fn user_flatpak_dir() -> Option<PathBuf> {
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(data).join("flatpak"));
    }
    let home = std::env::var_os("HOME").filter(|v| !v.is_empty())?;
    Some(PathBuf::from(home).join(".local/share/flatpak"))
}

/// Load `source` into the database, replacing anything previously ingested
/// from it. Returns the number of apps indexed.
///
/// Skipped entirely when the stored fingerprint still matches — re-parsing a
/// 48 MB XML catalog on every launch is exactly the cost that makes existing
/// software centres feel slow.
pub fn ingest(conn: &mut Connection, source: &Source) -> Result<usize, Error> {
    if is_current(conn, source)? {
        tracing::debug!(key = %source.key, "catalog already current, skipping");
        return Ok(0);
    }

    tracing::info!(key = %source.key, path = %source.catalog.display(), "ingesting catalog");
    let collection = Collection::from_gzipped(source.catalog.clone())
        .map_err(|e| Error::Parse(format!("{}: {e}", source.catalog.display())))?;

    let tx = conn.transaction()?;

    // Wipe this source's previous rows. Screenshots and releases follow via
    // ON DELETE CASCADE, and the FTS triggers keep the index in step.
    tx.execute(
        "DELETE FROM app WHERE backend = 'flatpak' AND origin = ?1",
        params![source.origin],
    )?;

    let mut count = 0usize;
    for component in &collection.components {
        // Runtimes, addons, codecs and fonts are not things a user installs
        // from a store front page.
        if component.kind != ComponentKind::DesktopApplication {
            continue;
        }
        if insert_component(&tx, source, component)? {
            count += 1;
        }
    }

    tx.execute(
        "INSERT INTO source (key, fingerprint) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET fingerprint = excluded.fingerprint",
        params![source.key, source.fingerprint],
    )?;

    tx.commit()?;
    tracing::info!(key = %source.key, count, "catalog ingested");
    Ok(count)
}

fn is_current(conn: &Connection, source: &Source) -> Result<bool, Error> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT fingerprint FROM source WHERE key = ?1",
            params![source.key],
            |row| row.get(0),
        )
        .ok();
    Ok(stored.is_some_and(|s| s == source.fingerprint) && !source.fingerprint.is_empty())
}

fn insert_component(
    tx: &rusqlite::Transaction<'_>,
    source: &Source,
    component: &Component,
) -> Result<bool, Error> {
    // An app with no name is unusable in a list; skip rather than show blanks.
    let Some(name) = translated(&component.name) else {
        return Ok(false);
    };

    let app_id = component.id.0.clone();
    let categories = component
        .categories
        .iter()
        .map(category_name)
        .collect::<Vec<_>>()
        .join(";");

    // Keywords feed FTS only — they're never displayed, so the default locale
    // is enough and joining them into one column keeps the index simple.
    let keywords = component
        .keywords
        .as_ref()
        .and_then(|k| k.0.get("C").cloned())
        .unwrap_or_default()
        .join(" ");

    // `RETURNING id` rather than `last_insert_rowid()`: on a conflict nothing
    // is inserted and `last_insert_rowid()` still points at the *previous*
    // row, so screenshots would be attached to the wrong app — or, on the
    // first component, to row 0, which fails the foreign key outright.
    // No row returned means this was a duplicate; skip it.
    let row_id: Option<i64> = tx
        .query_row(
            "INSERT INTO app
                (backend, app_id, origin, name, summary, description,
                 developer, license, version, icon_path, categories, keywords)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (backend, app_id, origin) DO NOTHING
             RETURNING id",
            params![
            "flatpak",
            app_id,
            source.origin,
            name,
            component.summary.as_ref().and_then(translated),
            component.description.as_ref().and_then(translated_markup),
            component.developer_name.as_ref().and_then(translated),
            component.project_license.as_ref().map(|l| l.0.clone()),
            component.releases.first().map(|r| r.version.clone()),
            resolve_icon(component, &source.icons).map(|p| p.to_string_lossy().into_owned()),
            categories,
            keywords,
            ],
            |row| row.get(0),
        )
        .optional()?;

    let Some(row_id) = row_id else {
        return Ok(false);
    };

    for (position, shot) in component.screenshots.iter().enumerate() {
        // Prefer a full-size source image; thumbnails are for grids we don't
        // build yet, and picking the first image is good enough otherwise.
        let Some(image) = shot
            .images
            .iter()
            .find(|i| i.kind == appstream::enums::ImageKind::Source)
            .or_else(|| shot.images.first())
        else {
            continue;
        };

        tx.execute(
            "INSERT INTO screenshot (app, url, caption, is_default, position)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                row_id,
                image.url.to_string(),
                shot.caption.as_ref().and_then(translated),
                shot.is_default as i64,
                position as i64,
            ],
        )?;
    }

    // Newest first, capped — a few apps carry hundreds of releases and nobody
    // scrolls past the recent ones.
    for (position, release) in component.releases.iter().take(20).enumerate() {
        tx.execute(
            "INSERT INTO release (app, version, timestamp, description, position)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                row_id,
                release.version,
                release.date.map(|d| d.timestamp()),
                release.description.as_ref().and_then(translated_markup),
                position as i64,
            ],
        )?;
    }

    Ok(true)
}

/// Pick the best on-disk icon for a component.
///
/// Flatpak's catalog stores `Icon::Cached` entries whose path is a bare
/// filename relative to `<active>/icons/<size>/`. Prefer the largest size we
/// know Flatpak ships, then fall back. Remote icons are ignored here — the UI
/// must not block on a download to draw a list row.
fn resolve_icon(component: &Component, icons_dir: &Path) -> Option<PathBuf> {
    let cached: Vec<&PathBuf> = component
        .icons
        .iter()
        .filter_map(|icon| match icon {
            Icon::Cached { path, .. } => Some(path),
            _ => None,
        })
        .collect();

    for size in ["128x128", "64x64"] {
        for path in &cached {
            let file = path.file_name()?;
            let candidate = icons_dir.join(size).join(file);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // A local/absolute icon path is usable directly if it exists.
    component.icons.iter().find_map(|icon| match icon {
        Icon::Local { path, .. } if path.is_file() => Some(path.clone()),
        _ => None,
    })
}

/// `Category` has no `Display`; its `Debug` form is the freedesktop name,
/// except for the catch-all variant which wraps the raw string.
fn category_name(category: &Category) -> String {
    match category {
        Category::Unknown(raw) => raw.clone(),
        other => format!("{other:?}"),
    }
}

/// AppStream stores the untranslated value under the `"C"` locale key.
/// `get_for_locale` does no fallback, so localisation is deliberately left for
/// later rather than half-done here.
fn translated(value: &TranslatableString) -> Option<String> {
    value.get_default().cloned().filter(|s| !s.is_empty())
}

fn translated_markup(value: &MarkupTranslatableString) -> Option<String> {
    value.get_default().cloned().filter(|s| !s.is_empty())
}
