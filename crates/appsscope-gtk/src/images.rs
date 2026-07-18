//! Screenshot fetching with a disk cache.
//!
//! Screenshots are the difference between a list of names and something worth
//! browsing, but they're remote and slow. Everything here is off the main
//! thread, cached on disk by URL, and best-effort: a screenshot that fails to
//! load leaves a placeholder rather than an error, because it is never the
//! reason someone opened the page.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

/// Refuse absurdly large images rather than filling the cache with one file.
const MAX_BYTES: u64 = 12 * 1024 * 1024;

/// Widest we ever need to draw a screenshot. Flathub serves originals — some
/// are 4000px+ wide, which is a 14-megapixel decode on the GTK main thread and
/// a visible stutter every time a hero card scrolls into view. Downscaling
/// once at fetch time makes every later decode cheap.
const MAX_WIDTH: u32 = 1600;

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("AppsScope/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build the HTTP client")
    })
}

/// `$XDG_CACHE_HOME/appsscope/images`.
fn cache_dir() -> Option<PathBuf> {
    let base = if let Some(cache) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(cache)
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".cache")
    };
    Some(base.join("appsscope").join("images"))
}

/// Cache filename for a URL.
///
/// A hash rather than the URL itself: screenshot URLs routinely exceed the
/// 255-byte filename limit and contain path separators.
fn cache_path(url: &str) -> Option<PathBuf> {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    Some(cache_dir()?.join(format!("{:016x}", hasher.finish())))
}

/// Fetch `url`, returning the path to the cached file.
///
/// Returns `None` on any failure — network, HTTP status, oversized body, or
/// disk. Callers show a placeholder.
pub async fn fetch(url: String) -> Option<PathBuf> {
    let path = cache_path(&url)?;

    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return Some(path);
    }

    let response = client().get(&url).send().await.ok()?;
    if !response.status().is_success() {
        tracing::debug!(%url, status = %response.status(), "screenshot fetch failed");
        return None;
    }

    // Trust the declared length only to reject early; the body is checked
    // again after reading in case the header lied.
    if response.content_length().is_some_and(|len| len > MAX_BYTES) {
        return None;
    }

    let bytes = response.bytes().await.ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }

    let parent = path.parent()?;
    tokio::fs::create_dir_all(parent).await.ok()?;

    // Decoding and resizing are CPU-bound, so they go to a blocking thread —
    // never the runtime's async workers and certainly never the GTK thread.
    let encoded = tokio::task::spawn_blocking(move || downscale(&bytes))
        .await
        .ok()?
        .unwrap_or_default();

    if encoded.is_empty() {
        return None;
    }

    // Write to a temporary name and rename into place, so a torn write from a
    // crash or a concurrent fetch can't leave a truncated image in the cache.
    let temp = path.with_extension("part");
    tokio::fs::write(&temp, &encoded).await.ok()?;
    tokio::fs::rename(&temp, &path).await.ok()?;

    Some(path)
}

/// Decode, bound the width, and re-encode as PNG.
///
/// PNG rather than JPEG: these are UI screenshots with text and hard edges,
/// where JPEG ringing is obvious. Images already within the limit are
/// re-encoded rather than passed through, so every cache entry is known-good
/// and decodable — a truncated download that happens to parse partially would
/// otherwise be cached and drawn broken forever.
fn downscale(bytes: &[u8]) -> Option<Vec<u8>> {
    let decoded = image::load_from_memory(bytes).ok()?;

    let resized = if decoded.width() > MAX_WIDTH {
        let height = decoded.height() * MAX_WIDTH / decoded.width().max(1);
        decoded.resize(MAX_WIDTH, height, image::imageops::FilterType::Lanczos3)
    } else {
        decoded
    };

    let mut out = std::io::Cursor::new(Vec::new());
    resized.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}
