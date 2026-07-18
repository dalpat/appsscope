//! A compact app tile for horizontal shelves.
//!
//! Modelled on the Play Store's shelf tile: large rounded icon, name beneath,
//! one line of secondary text. Narrow and flat, because a shelf shows six to
//! eight of these at once and card chrome at that density reads as noise.

use appsscope_core::{App, AppRef};
use relm4::factory::{DynamicIndex, FactoryComponent, FactorySender};
use relm4::gtk;
use relm4::gtk::prelude::*;

/// Fixed tile width. Shelves scroll horizontally, so tiles do not flex —
/// a ragged right edge while scrolling looks broken.
const TILE_WIDTH: i32 = 136;
const ICON_SIZE: i32 = 72;

#[derive(Debug)]
pub struct AppTile {
    app: App,
    installed: bool,
    /// Whether to show the packaging-format pill.
    ///
    /// Off when only one backend is active: a shelf of 48 tiles all reading
    /// "Flatpak" is repetition, not information. It switches itself on as soon
    /// as a list can actually mix formats.
    show_format: bool,
}

#[derive(Debug)]
pub struct AppTileInit {
    pub app: App,
    pub installed: bool,
    pub show_format: bool,
}

#[derive(Debug)]
pub enum AppTileOutput {
    Activated(AppRef),
}

#[relm4::factory(pub)]
impl FactoryComponent for AppTile {
    type Init = AppTileInit;
    type Input = ();
    type Output = AppTileOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::Box;

    view! {
        gtk::Button {
            add_css_class: "app-tile",
            add_css_class: "flat",
            set_width_request: TILE_WIDTH,
            set_valign: gtk::Align::Start,

            connect_clicked[sender, id = self.app.app_ref.clone()] => move |_| {
                let _ = sender.output(AppTileOutput::Activated(id.clone()));
            },

            #[wrap(Some)]
            set_child = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,

                // The rounded plate is applied to the image itself rather
                // than a wrapper: `Overflow::Hidden` plus a CSS border-radius
                // clips full-bleed artwork to the rounded square, and the CSS
                // padding gives transparent logos room to breathe.
                gtk::Image {
                    add_css_class: "icon-tile",
                    set_overflow: gtk::Overflow::Hidden,
                    set_halign: gtk::Align::Center,
                    set_pixel_size: ICON_SIZE,
                    set_paintable: super::icon::paintable(&self.app, ICON_SIZE).as_ref(),
                },

                gtk::Label {
                    add_css_class: "tile-title",
                    set_label: &self.app.name,
                    set_xalign: 0.0,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    set_single_line_mode: true,
                    set_max_width_chars: 15,
                },

                gtk::Label {
                    add_css_class: "format-badge",
                    add_css_class: self.app.app_ref.backend.css_class(),
                    set_label: self.app.app_ref.backend.badge_label(),
                    set_halign: gtk::Align::Start,
                    set_visible: self.show_format,
                },

                gtk::Label {
                    add_css_class: "tile-subtitle",
                    set_label: &self.subtitle(),
                    set_xalign: 0.0,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    set_single_line_mode: true,
                    set_max_width_chars: 12,
                },
            },
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self { app: init.app, installed: init.installed, show_format: init.show_format }
    }
}

impl AppTile {
    /// One line under the name. "Installed" wins over the developer, since
    /// it's the thing that changes what you'd do next.
    fn subtitle(&self) -> String {
        if self.installed {
            return "Installed".to_owned();
        }
        self.app
            .developer
            .clone()
            .or_else(|| self.app.summary.clone())
            .unwrap_or_default()
    }
}
