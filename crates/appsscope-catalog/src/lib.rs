//! The app catalog: AppStream metadata indexed into SQLite for instant search.
//!
//! This is the piece that decides whether the app feels fast. Parsing Flathub's
//! 48 MB AppStream XML takes seconds; doing it on every launch — or worse, on
//! every keystroke — is why existing software centres feel sluggish. So it is
//! parsed once into SQLite with an FTS5 index, fingerprinted against the source
//! commit, and thereafter queried in microseconds.
//!
//! The catalog is a pure cache. It can be deleted at any time and rebuilt.

mod ingest;
mod query;
mod schema;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub use ingest::{Source, discover_flatpak_sources};
pub use query::Filter;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("catalog database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("failed to parse catalog: {0}")]
    Parse(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct Catalog {
    conn: Connection,
}

impl Catalog {
    /// Open (creating if needed) the catalog at the default cache location.
    pub fn open_default() -> Result<Self, Error> {
        let path = default_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::open(&path)
    }

    pub fn open(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path)?;
        schema::init(&conn)?;
        Ok(Self { conn })
    }

    /// In-memory catalog, for tests.
    pub fn open_in_memory() -> Result<Self, Error> {
        let conn = Connection::open_in_memory()?;
        schema::init(&conn)?;
        Ok(Self { conn })
    }

    /// Index every Flatpak catalog found on disk.
    ///
    /// Sources whose fingerprint is unchanged are skipped, so calling this on
    /// every launch is cheap once warm. Returns apps newly indexed.
    pub fn sync(&mut self) -> Result<usize, Error> {
        let sources = discover_flatpak_sources();

        if sources.is_empty() {
            tracing::warn!(
                "no flatpak appstream catalogs found — run `flatpak update --appstream`"
            );
        }

        let mut total = 0;
        for source in &sources {
            match ingest::ingest(&mut self.conn, source) {
                Ok(count) => total += count,
                // One malformed catalog shouldn't stop the others indexing.
                Err(err) => tracing::warn!(key = %source.key, %err, "catalog ingest failed"),
            }
        }
        Ok(total)
    }

    /// Number of apps indexed. Used to decide whether a sync is needed at all.
    pub fn len(&self) -> Result<usize, Error> {
        query::count(&self.conn)
    }

    pub fn is_empty(&self) -> Result<bool, Error> {
        Ok(self.len()? == 0)
    }

    /// Full-text search, ranked by relevance.
    pub fn search(
        &self,
        term: &str,
        limit: usize,
        filter: &Filter,
    ) -> Result<Vec<appsscope_core::App>, Error> {
        query::search(&self.conn, term, limit, filter)
    }

    /// Apps to show on the Explore page when nothing is being searched.
    pub fn browse(
        &self,
        limit: usize,
        filter: &Filter,
    ) -> Result<Vec<appsscope_core::App>, Error> {
        query::browse(&self.conn, limit, filter)
    }

    /// Featured apps for the hero banner, with their default screenshot.
    pub fn featured(&self, limit: usize) -> Result<Vec<appsscope_core::App>, Error> {
        query::featured(&self.conn, limit)
    }

    /// Apps with the most recent releases.
    pub fn recently_updated(
        &self,
        limit: usize,
        filter: &Filter,
    ) -> Result<Vec<appsscope_core::App>, Error> {
        query::recently_updated(&self.conn, limit, filter)
    }

    /// Apps in a freedesktop category, e.g. `Development`.
    pub fn by_category(
        &self,
        category: &str,
        limit: usize,
        filter: &Filter,
    ) -> Result<Vec<appsscope_core::App>, Error> {
        query::by_category(&self.conn, category, limit, filter)
    }

    /// Full record for one app, including screenshots and changelog.
    pub fn get(&self, app_id: &str) -> Result<Option<appsscope_core::App>, Error> {
        query::get(&self.conn, app_id)
    }
}

/// `$XDG_CACHE_HOME/appsscope/catalog.db`, falling back to `~/.cache`.
///
/// Deliberately in the cache directory, not data — losing it costs a re-index,
/// nothing more.
fn default_path() -> Result<PathBuf, Error> {
    let base = if let Some(cache) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(cache)
    } else {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "neither XDG_CACHE_HOME nor HOME set")
        })?;
        PathBuf::from(home).join(".cache")
    };
    Ok(base.join("appsscope").join("catalog.db"))
}
