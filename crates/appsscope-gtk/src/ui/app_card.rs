//! A single app tile in the browse grid.
//!
//! The whole card is a `GtkButton`. That gets hover, focus and active states
//! from the theme for free, and makes the tile keyboard-navigable and
//! screen-reader-legible without hand-rolling a gesture controller.
//!
//! There's deliberately no Install button on the card. Installing is a
//! decision made after reading the description and permissions, so the card's
//! only job is to get you to the detail page — the same reason storefronts
//! that do this well keep their grids uncluttered.

use appsscope_core::{App, AppRef};
use relm4::factory::{DynamicIndex, FactoryComponent, FactorySender};
use relm4::gtk;
use relm4::gtk::prelude::*;

#[derive(Debug)]
pub struct AppCard {
    app: App,
    installed: bool,
}

#[derive(Debug)]
pub struct AppCardInit {
    pub app: App,
    pub installed: bool,
}

#[derive(Debug)]
pub enum AppCardOutput {
    Activated(AppRef),
}

#[relm4::factory(pub)]
impl FactoryComponent for AppCard {
    type Init = AppCardInit;
    type Input = ();
    type Output = AppCardOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::FlowBox;

    view! {
        gtk::Button {
            add_css_class: "app-card",
            set_hexpand: true,

            connect_clicked[sender, id = self.app.app_ref.clone()] => move |_| {
                let _ = sender.output(AppCardOutput::Activated(id.clone()));
            },

            #[wrap(Some)]
            set_child = &gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 14,
                set_margin_top: 14,
                set_margin_bottom: 14,
                set_margin_start: 14,
                set_margin_end: 14,

                gtk::Image {
                    add_css_class: "icon-tile",
                    set_overflow: gtk::Overflow::Hidden,
                    set_valign: gtk::Align::Start,
                    set_pixel_size: 56,
                    set_paintable: super::icon::paintable(&self.app, 56).as_ref(),
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 3,
                    set_valign: gtk::Align::Center,
                    set_hexpand: true,

                    gtk::Box {
                        set_spacing: 8,

                        gtk::Label {
                            add_css_class: "app-card-title",
                            set_label: &self.app.name,
                            set_xalign: 0.0,
                            set_ellipsize: gtk::pango::EllipsizeMode::End,
                            set_single_line_mode: true,
                        },

                        gtk::Label {
                            add_css_class: "format-badge",
                            add_css_class: self.app.app_ref.backend.css_class(),
                            set_label: self.app.app_ref.backend.badge_label(),
                            set_valign: gtk::Align::Center,
                        },

                        gtk::Label {
                            add_css_class: "installed-badge",
                            set_label: "Installed",
                            set_visible: self.installed,
                            set_valign: gtk::Align::Center,
                        },
                    },

                    gtk::Label {
                        add_css_class: "app-card-summary",
                        set_label: self.app.summary.as_deref().unwrap_or(""),
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_lines: 2,
                        set_ellipsize: gtk::pango::EllipsizeMode::End,
                        set_max_width_chars: 28,
                    },
                },
            },
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self { app: init.app, installed: init.installed }
    }
}
