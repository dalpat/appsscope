//! Application stylesheet.
//!
//! Everything is expressed in libadwaita's named colours (`@card_bg_color`,
//! `@accent_bg_color`, …) rather than literal hex. That's what makes the app
//! track light/dark and the user's accent colour automatically — hardcoding
//! colours is the single fastest way to look out of place on a GNOME desktop.

use relm4::gtk;
use relm4::gtk::gdk;

const STYLE: &str = r#"
/* ---- Browse cards ---------------------------------------------------- */

.app-card {
    background-color: @card_bg_color;
    border-radius: 16px;
    padding: 0;
    box-shadow: 0 1px 2px alpha(black, 0.06);
    transition: box-shadow 180ms ease, background-color 180ms ease;
}

.app-card:hover {
    background-color: mix(@card_bg_color, @accent_bg_color, 0.07);
    box-shadow: 0 6px 18px alpha(black, 0.14);
}

.app-card:active {
    background-color: mix(@card_bg_color, @accent_bg_color, 0.13);
    box-shadow: 0 1px 2px alpha(black, 0.08);
}

.app-card-title {
    font-weight: 700;
    font-size: 1.02em;
}

.app-card-summary {
    font-size: 0.9em;
    opacity: 0.66;
}

/* ---- Hero banner ----------------------------------------------------- */

.hero-card {
    border-radius: 20px;
    padding: 0;
    background: alpha(@window_fg_color, 0.06);
    box-shadow: 0 4px 16px alpha(black, 0.18);
    transition: box-shadow 200ms ease;
}

.hero-card:hover {
    box-shadow: 0 8px 26px alpha(black, 0.26);
}

/* The information bar sits on the window's own surface, so its text uses the
   normal theme colours and stays legible in both light and dark — unlike the
   forced-white text the scrim version needed. */
.hero-bar {
    background-color: @card_bg_color;
    padding: 0 20px;
}

.hero-name {
    font-size: 1.35em;
    font-weight: 800;
}

.hero-summary {
    font-size: 0.95em;
    opacity: 0.68;
}

.hero-cta {
    background-color: @accent_bg_color;
    color: @accent_fg_color;
    border-radius: 999px;
    padding: 9px 22px;
    font-weight: 700;
}

/* ---- Icon tiles ------------------------------------------------------ */

/* Every app icon in the app sits in one of these. The plate is what makes a
   row of wildly inconsistent Flatpak icons read as a uniform shelf. */
.icon-tile {
    background-color: @card_bg_color;
    border-radius: 22%;
    padding: 6px;
    box-shadow: 0 2px 6px alpha(black, 0.16);
}

/* ---- Shelf tiles ----------------------------------------------------- */

.shelf-title {
    font-weight: 800;
    font-size: 1.35em;
}

.shelf-more {
    font-weight: 600;
    opacity: 0.8;
}

/* Flat rather than card-like: a shelf shows eight at once, and borders at
   that density read as visual noise. */
.app-tile {
    border-radius: 14px;
    padding: 8px;
    background: none;
    box-shadow: none;
    transition: background-color 150ms ease;
}

.app-tile:hover {
    background-color: alpha(@window_fg_color, 0.06);
}

.tile-title {
    font-weight: 650;
    font-size: 0.94em;
}

.tile-subtitle {
    font-size: 0.8em;
    opacity: 0.6;
}

/* ---- Detail metrics row ---------------------------------------------- */

/* The Play Store's rating/size/age strip: a few facts, centred, separated by
   thin rules. Denser and more scannable than a list of labelled rows. */
.metric-value {
    font-weight: 700;
    font-size: 1.0em;
}

.metric-label {
    font-size: 0.78em;
    opacity: 0.6;
}

.metric-divider {
    background-color: alpha(@window_fg_color, 0.15);
}

/* ---- Format pills ---------------------------------------------------- */

/* Neutral, not colour-coded. GTK's CSS is a subset and does not honour
   `@media (prefers-color-scheme:)` — it tolerates the block silently rather
   than reporting it, so hand-picked light/dark colour pairs would simply be
   wrong in one theme with no warning. Named colours adapt correctly instead.

   Colour-coding per format is also information-free while Flatpak is the only
   backend; it earns its keep once a list can mix formats, and the
   `.format-*` classes below are the hooks for that. */
.format-badge {
    background-color: alpha(@window_fg_color, 0.08);
    border-radius: 999px;
    padding: 2px 9px;
    font-size: 0.75em;
    font-weight: 700;
    letter-spacing: 0.02em;
    opacity: 0.85;
}

/* Flatpak takes the user's accent so it reads as a real label rather than
   greyed-out text; the others stay neutral until they exist. */
.format-flatpak {
    background-color: alpha(@accent_bg_color, 0.18);
    color: @accent_color;
    opacity: 1;
}

/* ---- Installed badge ------------------------------------------------- */

.installed-badge {
    background-color: alpha(@success_color, 0.15);
    color: @success_color;
    border-radius: 999px;
    padding: 2px 10px;
    font-size: 0.8em;
    font-weight: 700;
}

/* ---- Sidebar --------------------------------------------------------- */

.sidebar-heading {
    font-size: 0.78em;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    opacity: 0.5;
}

/* Pending-update count on the Updates row. */
.update-badge {
    background-color: @accent_bg_color;
    color: @accent_fg_color;
    border-radius: 999px;
    padding: 1px 8px;
    font-size: 0.78em;
    font-weight: 700;
}

/* ---- Detail page ----------------------------------------------------- */

.detail-hero {
    padding: 8px 0 4px 0;
}

.detail-name {
    font-weight: 800;
    font-size: 1.9em;
}

.detail-developer {
    opacity: 0.6;
    font-size: 1.05em;
}

/* Metadata pills under the hero — lighter than a full preferences group
   for facts you scan rather than read. */
.meta-pill {
    background-color: alpha(@window_fg_color, 0.06);
    border-radius: 999px;
    padding: 6px 14px;
}

.meta-pill-label {
    font-size: 0.75em;
    opacity: 0.55;
    font-weight: 700;
}

.meta-pill-value {
    font-size: 0.92em;
    font-weight: 600;
}

.screenshot-frame {
    border-radius: 12px;
    background-color: alpha(@window_fg_color, 0.05);
    box-shadow: 0 1px 3px alpha(black, 0.10);
}

.section-heading {
    font-weight: 700;
    font-size: 1.15em;
}

/* ---- Risk badges ----------------------------------------------------- */

/* Colour alone must not carry the verdict — each badge pairs its colour with
   a distinct icon and its own words, so it still reads correctly for someone
   who cannot distinguish the hues. */
.risk-badge {
    border-radius: 999px;
    padding: 4px 12px;
    font-weight: 700;
    font-size: 0.85em;
}

.risk-safe {
    background-color: alpha(@success_color, 0.15);
    color: @success_color;
}

.risk-caution {
    background-color: alpha(@warning_color, 0.15);
    color: @warning_color;
}

.risk-critical {
    background-color: alpha(@error_color, 0.15);
    color: @error_color;
}

.risk-unknown {
    background-color: alpha(@window_fg_color, 0.08);
    opacity: 0.75;
}

/* Prefix icons in permission rows: colour only, no pill. */
image.risk-safe, image.risk-caution, image.risk-critical {
    background: none;
    padding: 0;
}

/* Compact variant for list rows. */
.risk-chip {
    border-radius: 999px;
    padding: 1px 9px;
    font-size: 0.78em;
    font-weight: 700;
}

/* ---- Consent dialog -------------------------------------------------- */

.consent-permission {
    padding: 6px 0;
}

/* "Flatpak from Flathub" — the format is the first thing the dialog says. */
.plan-provenance {
    font-weight: 700;
}

.plan-heading {
    font-weight: 700;
    font-size: 0.85em;
    opacity: 0.75;
}

/* ---- Progress -------------------------------------------------------- */

.transaction-bar {
    background-color: alpha(@accent_bg_color, 0.08);
    border-radius: 12px;
    padding: 8px 14px;
    margin: 6px 12px;
}
"#;

/// Install the stylesheet for the default display.
///
/// Uses `PRIORITY_APPLICATION` so it overrides the platform theme but still
/// loses to the user's own `gtk.css` — their overrides should win.
pub fn load() {
    let provider = gtk::CssProvider::new();

    // GTK does not fail loudly on a bad stylesheet — it skips the offending
    // rule and carries on, which leaves the UI subtly wrong with no signal.
    // Anything we get wrong should show up in the log.
    provider.connect_parsing_error(|_, section, error| {
        tracing::error!(
            location = %section.to_str(),
            %error,
            "stylesheet parse error"
        );
    });

    provider.load_from_string(STYLE);

    let Some(display) = gdk::Display::default() else {
        tracing::warn!("no display; skipping stylesheet");
        return;
    };

    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
