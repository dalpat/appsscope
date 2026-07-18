//! Turning an [`IconSource`] into something GTK can draw.
//!
//! Everything funnels into a single `Paintable` rather than switching between
//! `set_from_file` and `set_icon_name`. A `GtkImage` holds exactly one storage
//! type, so mixing the two means whichever is set last wins and the other
//! silently blanks the widget.

use appsscope_core::{App, IconSource};
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::prelude::*;

/// Best available icon for `app` at `size` logical pixels.
///
/// Falls back through: the catalog's cached PNG → the icon theme under the
/// app's own ID (which is where Flatpak installs app icons) → a generic
/// executable glyph. Never returns `None` unless there's no display at all.
pub fn paintable(app: &App, size: i32) -> Option<gdk::Paintable> {
    if let Some(IconSource::Cached(path)) = &app.icon {
        // Loading is synchronous, but these are small local PNGs already in
        // the page cache after the first read.
        if let Ok(texture) = gdk::Texture::from_filename(path) {
            return Some(texture.upcast());
        }
    }

    let name = match &app.icon {
        Some(IconSource::Themed(name)) => name.clone(),
        _ => app.app_ref.id.clone(),
    };

    themed(&name, size)
}

/// An icon presented as a rounded, shadowed tile.
///
/// Bare PNGs on a flat background is what made the grid look cheap: Flatpak
/// icons are a mix of full-bleed artwork and transparent logos at wildly
/// different visual weights, so drawing them raw gives a ragged, inconsistent
/// row. Seating every icon in an identical rounded plate is what the Play
/// Store, the App Store and macOS all do, and it is most of the difference
/// between "list of images" and "shelf of apps".
///
/// `GTK_OVERFLOW_HIDDEN` plus a CSS `border-radius` gives true rounded
/// clipping, so full-bleed artwork is actually cut to the rounded square
/// rather than overflowing its plate.
pub fn tile(app: &App, size: i32) -> gtk::Widget {
    let plate = gtk::Box::new(gtk::Orientation::Vertical, 0);
    plate.add_css_class("icon-tile");
    plate.set_overflow(gtk::Overflow::Hidden);
    plate.set_halign(gtk::Align::Center);
    plate.set_valign(gtk::Align::Center);
    plate.set_size_request(size, size);

    let image = gtk::Image::new();
    image.set_paintable(paintable(app, size).as_ref());
    // Inset slightly: transparent logos need breathing room, and full-bleed
    // art still fills the plate because the clip does the framing.
    image.set_pixel_size(size - 12);
    image.set_hexpand(true);
    image.set_vexpand(true);

    plate.append(&image);
    plate.upcast()
}

/// Look a name up in the icon theme, falling back to a generic glyph.
pub fn themed(name: &str, size: i32) -> Option<gdk::Paintable> {
    let display = gdk::Display::default()?;
    let theme = gtk::IconTheme::for_display(&display);

    let icon = theme.lookup_icon(
        name,
        &["application-x-executable"],
        size,
        1,
        gtk::TextDirection::None,
        gtk::IconLookupFlags::empty(),
    );

    Some(icon.upcast())
}
