//! Read queries against the catalog.

use appsscope_core::{App, AppRef, BackendId, IconSource, Release, Screenshot};
use rusqlite::{Connection, Row, params};

use crate::Error;

/// Restricts which apps a query returns.
///
/// Kept as a struct rather than a bare argument so adding future filters
/// (sandbox grade, installed state) doesn't churn every call site again.
#[derive(Debug, Default, Clone)]
pub struct Filter {
    /// Packaging format, as [`appsscope_core::BackendId::as_str`].
    /// `None` means every format.
    pub backend: Option<String>,
}

impl Filter {
    /// SQL fragment appended to the WHERE clause.
    ///
    /// Written as `?N = '' OR ...` so the parameter is always bound and the
    /// positional indices stay identical whether or not a filter is active —
    /// building the clause conditionally is how index bugs get in.
    const CLAUSE: &'static str = " AND (?1 = '' OR a.backend = ?1)";

    /// Bound value: the empty string means "no filter".
    fn value(&self) -> String {
        self.backend.clone().unwrap_or_default()
    }
}

/// Columns every list query selects, in the order [`row_to_app`] expects.
const APP_COLUMNS: &str = "a.id, a.backend, a.app_id, a.origin, a.name, a.summary, \
                           a.description, a.developer, a.license, a.version, \
                           a.icon_path, a.categories";

pub fn count(conn: &Connection) -> Result<usize, Error> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM app", [], |row| row.get(0))?;
    Ok(n as usize)
}

/// Full-text search over name, summary, keywords and app id.
///
/// FTS5's `bm25` ranks lower-is-better. Name matches are weighted far above
/// summary and keywords so typing "gimp" surfaces GIMP rather than the dozen
/// apps whose descriptions mention it.
pub fn search(
    conn: &Connection,
    term: &str,
    limit: usize,
    filter: &Filter,
) -> Result<Vec<App>, Error> {
    let Some(query) = fts_query(term) else {
        return Ok(Vec::new());
    };

    let clause = Filter::CLAUSE;
    let sql = format!(
        "SELECT {APP_COLUMNS}
         FROM app_fts f
         JOIN app a ON a.id = f.rowid
         WHERE app_fts MATCH ?2{clause}
         ORDER BY bm25(app_fts, 10.0, 2.0, 1.0, 5.0)
         LIMIT ?3"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![filter.value(), query, limit as i64], row_to_app)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Error::from)
}

/// Turn user input into an FTS5 MATCH expression.
///
/// Every token is quoted and given a `*` suffix so search updates on each
/// keystroke rather than only on word boundaries. Quoting also stops FTS5
/// treating stray punctuation as query syntax and erroring mid-typing.
fn fts_query(term: &str) -> Option<String> {
    let tokens: Vec<String> = term
        .split_whitespace()
        .map(|token| token.replace('"', ""))
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{token}\"*"))
        .collect();

    (!tokens.is_empty()).then(|| tokens.join(" AND "))
}

pub fn browse(conn: &Connection, limit: usize, filter: &Filter) -> Result<Vec<App>, Error> {
    // Apps with screenshots are the ones with metadata worth showing, so they
    // lead. Beyond that it's alphabetical — a real editorial ranking needs
    // popularity data we don't have yet.
    let clause = Filter::CLAUSE;
    let sql = format!(
        "SELECT {APP_COLUMNS}
         FROM app a
         WHERE 1 = 1{clause}
         ORDER BY (SELECT COUNT(*) FROM screenshot s WHERE s.app = a.id) > 0 DESC,
                  a.name COLLATE NOCASE
         LIMIT ?2"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![filter.value(), limit as i64], row_to_app)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Error::from)
}

/// Apps for the featured hero: recently updated, with artwork worth showing.
///
/// Unlike the shelf queries this eagerly loads the default screenshot, because
/// the hero *is* the screenshot — a hero card with no image is just a big
/// empty rectangle, so apps without one are excluded rather than shown bare.
pub fn featured(conn: &Connection, limit: usize) -> Result<Vec<App>, Error> {
    let sql = format!(
        "SELECT {APP_COLUMNS}, s.url
         FROM app a
         JOIN (SELECT app, MAX(timestamp) AS ts FROM release
               WHERE timestamp IS NOT NULL
                 AND timestamp <= strftime('%s', 'now')
               GROUP BY app) r ON r.app = a.id
         JOIN screenshot s ON s.app = a.id AND s.position = 0
         WHERE a.icon_path IS NOT NULL
           AND a.summary IS NOT NULL
         ORDER BY r.ts DESC
         LIMIT ?1"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        let mut app = row_to_app(row)?;
        app.screenshots = vec![Screenshot {
            url: row.get(12)?,
            caption: None,
            is_default: true,
        }];
        Ok(app)
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Error::from)
}

/// Apps with the most recent release, newest first.
///
/// Uses the AppStream `<releases>` timestamps we already index. Apps with no
/// dated release are excluded rather than sorted to the bottom — an undated
/// app is not "old", it's unknown, and padding the shelf with them would make
/// the ordering meaningless.
pub fn recently_updated(
    conn: &Connection,
    limit: usize,
    filter: &Filter,
) -> Result<Vec<App>, Error> {
    let clause = Filter::CLAUSE;
    // Future-dated releases are excluded. A meaningful number of AppStream
    // files carry timestamps months ahead — typos, or build dates from a
    // machine with a wrong clock — and without this filter those broken
    // entries permanently occupy the top of the shelf.
    let sql = format!(
        "SELECT {APP_COLUMNS}
         FROM app a
         JOIN (SELECT app, MAX(timestamp) AS ts FROM release
               WHERE timestamp IS NOT NULL
                 AND timestamp <= strftime('%s', 'now')
               GROUP BY app) r ON r.app = a.id
         WHERE 1 = 1{clause}
         ORDER BY r.ts DESC
         LIMIT ?2"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![filter.value(), limit as i64], row_to_app)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Error::from)
}

pub fn by_category(
    conn: &Connection,
    category: &str,
    limit: usize,
    filter: &Filter,
) -> Result<Vec<App>, Error> {
    let clause = Filter::CLAUSE;
    // Categories are stored as a `;`-delimited list; the delimiters on both
    // sides stop `Audio` matching `AudioVideo`.
    //
    // Ordered by recency, not alphabetically. Without real popularity data
    // there's no true ranking available, but "most recently updated" at least
    // surfaces maintained software — alphabetical order opens every shelf with
    // whatever is named "0 A.D." or "4KTUBE", which reads as unmaintained.
    // Apps with screenshots come first within that, as a proxy for care taken.
    let sql = format!(
        "SELECT {APP_COLUMNS}
         FROM app a
         LEFT JOIN (SELECT app, MAX(timestamp) AS ts FROM release
                    WHERE timestamp IS NOT NULL
                      AND timestamp <= strftime('%s', 'now')
                    GROUP BY app) r ON r.app = a.id
         WHERE ';' || a.categories || ';' LIKE '%;' || ?2 || ';%'{clause}
         ORDER BY (SELECT COUNT(*) FROM screenshot s WHERE s.app = a.id) > 0 DESC,
                  r.ts IS NULL,
                  r.ts DESC,
                  a.name COLLATE NOCASE
         LIMIT ?3"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![filter.value(), category, limit as i64], row_to_app)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Error::from)
}

/// One app with its screenshots and changelog attached.
pub fn get(conn: &Connection, app_id: &str) -> Result<Option<App>, Error> {
    let sql = format!("SELECT {APP_COLUMNS} FROM app a WHERE a.app_id = ?1 LIMIT 1");

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map(params![app_id], |row| {
        Ok((row.get::<_, i64>(0)?, row_to_app(row)?))
    })?;

    let Some((row_id, mut app)) = rows.next().transpose()? else {
        return Ok(None);
    };

    app.screenshots = screenshots(conn, row_id)?;
    app.releases = releases(conn, row_id)?;
    Ok(Some(app))
}

fn screenshots(conn: &Connection, row_id: i64) -> Result<Vec<Screenshot>, Error> {
    let mut stmt = conn.prepare(
        "SELECT url, caption, is_default FROM screenshot WHERE app = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map(params![row_id], |row| {
        Ok(Screenshot {
            url: row.get(0)?,
            caption: row.get(1)?,
            is_default: row.get::<_, i64>(2)? != 0,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Error::from)
}

fn releases(conn: &Connection, row_id: i64) -> Result<Vec<Release>, Error> {
    let mut stmt = conn.prepare(
        "SELECT version, timestamp, description FROM release WHERE app = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map(params![row_id], |row| {
        Ok(Release {
            version: row.get(0)?,
            timestamp: row.get(1)?,
            description: row.get(2)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Error::from)
}

fn row_to_app(row: &Row<'_>) -> rusqlite::Result<App> {
    let backend = match row.get::<_, String>(1)?.as_str() {
        "packagekit" => BackendId::PackageKit,
        "appimage" => BackendId::AppImage,
        _ => BackendId::Flatpak,
    };

    let app_ref = {
        let base = AppRef::new(backend, row.get::<_, String>(2)?);
        match row.get::<_, Option<String>>(3)? {
            Some(origin) => base.with_origin(origin),
            None => base,
        }
    };

    let categories: String = row.get(11)?;

    Ok(App {
        app_ref,
        name: row.get(4)?,
        summary: row.get(5)?,
        description: row.get(6)?,
        developer: row.get(7)?,
        license: row.get(8)?,
        version: row.get(9)?,
        icon: row
            .get::<_, Option<String>>(10)?
            .map(|path| IconSource::Cached(path.into())),
        categories: categories
            .split(';')
            .filter(|c| !c.is_empty())
            .map(str::to_owned)
            .collect(),
        screenshots: Vec::new(),
        releases: Vec::new(),
        download_size: None,
        installed_size: None,
        sandbox: None,
    })
}
