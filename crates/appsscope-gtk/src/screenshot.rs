//! Self-capture, for documentation screenshots.
//!
//! GNOME refuses programmatic screenshots from an unfocused application, and
//! the portal requires a human to click through a dialog — neither works in a
//! build script or CI. GTK can render its own widget tree to a texture though,
//! which sidesteps the compositor entirely and produces exactly what the app
//! draws.
//!
//! Usage:
//!
//! ```text
//! APPSSCOPE_SCREENSHOT=out.png \
//! APPSSCOPE_SCREENSHOT_VIEW=home \
//! APPSSCOPE_SCREENSHOT_DELAY=8 appsscope
//! ```
//!
//! Configured by environment rather than command line because `GApplication`
//! parses `argv` itself and rejects flags it does not recognise — an unknown
//! `--screenshot` never reaches `main`.
//!
//! The delay exists because the interesting screens are populated
//! asynchronously — the catalogue is queried off-thread and hero artwork is
//! downloaded — so capturing immediately would photograph an empty window.

use std::path::PathBuf;

use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::prelude::*;

use crate::ui::sidebar::Destination;

/// A capture request, or the absence of one.
///
/// Modelled as a newtype over `Option` so the shell's `Init` type stays a
/// single value and normal launches pass `None` without ceremony.
pub struct Job(Option<Request>);

#[derive(Debug, Clone)]
pub struct Request {
    pub path: PathBuf,
    pub view: String,
    pub delay_seconds: u32,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl Job {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }

    pub fn into_request(self) -> Option<Request> {
        self.0
    }

    pub fn request(&self) -> Option<&Request> {
        self.0.as_ref()
    }

    /// Read a capture request from the environment.
    pub fn from_env() -> Self {
        let Some(path) = std::env::var_os("APPSSCOPE_SCREENSHOT") else {
            return Self(None);
        };

        Self(Some(Request {
            path: PathBuf::from(path),
            view: std::env::var("APPSSCOPE_SCREENSHOT_VIEW")
                .unwrap_or_else(|_| "home".to_owned()),
            delay_seconds: std::env::var("APPSSCOPE_SCREENSHOT_DELAY")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(8),
        }))
    }
}

impl Request {
    /// Which sidebar destination this capture wants, if not the default.
    pub fn destination(&self) -> Option<Destination> {
        match self.view.as_str() {
            "installed" => Some(Destination::Installed),
            "updates" => Some(Destination::Updates),
            "games" => Some(Destination::Category("Game".to_owned())),
            _ => None,
        }
    }
}

/// Capture `window` once its content has had time to load, then quit.
pub fn schedule(window: &adw::ApplicationWindow, request: Request) {
    let window = window.clone();

    glib::timeout_add_seconds_local_once(request.delay_seconds, move || {
        match capture(&window, &request.path) {
            Ok(()) => tracing::info!(path = %request.path.display(), "screenshot written"),
            Err(err) => tracing::error!(%err, "screenshot failed"),
        }
        window.close();
    });
}

/// Render the window's widget tree to a PNG.
///
/// Goes through the window's own `GskRenderer`, so what lands in the file is
/// what the GPU would put on screen — including CSS, rounded clipping and
/// loaded artwork.
fn capture(window: &adw::ApplicationWindow, path: &std::path::Path) -> Result<(), String> {
    let native = window.native().ok_or("window is not realised")?;
    let renderer = native.renderer().ok_or("window has no renderer")?;

    let width = window.width();
    let height = window.height();
    if width <= 0 || height <= 0 {
        return Err(format!("window has no size yet ({width}x{height})"));
    }

    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));

    let node = snapshot
        .to_node()
        .ok_or("nothing was drawn — is the window mapped?")?;

    let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let texture: gdk::Texture = renderer.render_texture(&node, Some(&bounds));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    texture.save_to_png(path).map_err(|err| err.to_string())
}

use relm4::adw;
use relm4::gtk::glib;
