//! One row in an app list.
//!
//! Deliberately dumb: it renders an [`App`] and emits intent. It never talks to
//! a backend itself, so the same row works in Explore, Installed, and Updates.

use adw::prelude::*;
use appsscope_core::{App, AppRef};
use relm4::factory::{DynamicIndex, FactoryComponent, FactorySender};
use relm4::gtk;
use relm4::prelude::*;

/// What the row's trailing button does, which is the only thing that differs
/// between the three lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    Install,
    Remove,
    Update,
    /// No button — the row is only a navigation target.
    None,
}

impl RowAction {
    fn label(self) -> Option<&'static str> {
        match self {
            Self::Install => Some("Install"),
            Self::Remove => Some("Remove"),
            Self::Update => Some("Update"),
            Self::None => None,
        }
    }

    /// Destructive actions get the red style; primary ones get the accent.
    fn css_class(self) -> &'static str {
        match self {
            Self::Remove => "destructive-action",
            _ => "suggested-action",
        }
    }
}

#[derive(Debug)]
pub struct AppRow {
    app: App,
    action: RowAction,
}

#[derive(Debug)]
pub struct AppRowInit {
    pub app: App,
    pub action: RowAction,
}

#[derive(Debug)]
pub enum AppRowOutput {
    /// The row body was clicked — open the detail page.
    Activated(AppRef),
    /// The trailing button was clicked.
    Action(RowAction, AppRef),
}

#[relm4::factory(pub)]
impl FactoryComponent for AppRow {
    type Init = AppRowInit;
    type Input = ();
    type Output = AppRowOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::ListBox;

    view! {
        adw::ActionRow {
            // Off by default these are parsed as Pango markup, and app names
            // and summaries are arbitrary text from a third-party catalog —
            // a bare `&` in "Trading & Combat" is enough to blank the row.
            set_use_markup: false,
            set_title: &self.app.name,
            set_subtitle: &self.subtitle(),
            set_title_lines: 1,
            set_subtitle_lines: 2,
            set_activatable: true,

            add_prefix = &gtk::Image {
                add_css_class: "icon-tile",
                set_overflow: gtk::Overflow::Hidden,
                set_pixel_size: 44,
                set_paintable: super::icon::paintable(&self.app, 44).as_ref(),
                set_margin_top: 8,
                set_margin_bottom: 8,
            },

            // What kind of package this is. Always shown, including when only
            // one backend is active — "which of these is a Flatpak" is not a
            // question the user should have to remember the answer to.
            add_suffix = &gtk::Label {
                add_css_class: "format-badge",
                add_css_class: self.app.app_ref.backend.css_class(),
                set_label: self.app.app_ref.backend.badge_label(),
                set_valign: gtk::Align::Center,
            },

            // Sandbox verdict, shown only when we actually know it. Installed
            // apps carry it for free from local metadata, which is what makes
            // the Installed list the fastest way to spot an app that can read
            // everything you own.
            add_suffix = &gtk::Box {
                add_css_class: "risk-chip",
                set_css_classes: &self.risk_classes(),
                set_valign: gtk::Align::Center,
                set_visible: self.app.sandbox.is_some(),
                set_spacing: 5,

                gtk::Image {
                    set_icon_name: self.risk_icon(),
                    set_pixel_size: 13,
                },
                gtk::Label {
                    set_label: &self.risk_label(),
                },
            },

            add_suffix = &gtk::Button {
                set_valign: gtk::Align::Center,
                #[watch]
                set_visible: self.action != RowAction::None,
                set_label: self.action.label().unwrap_or_default(),
                add_css_class: self.action.css_class(),
                connect_clicked[sender, action = self.action, id = self.app.app_ref.clone()] => move |_| {
                    let _ = sender.output(AppRowOutput::Action(action, id.clone()));
                },
            },

            connect_activated[sender, id = self.app.app_ref.clone()] => move |_| {
                let _ = sender.output(AppRowOutput::Activated(id.clone()));
            },
        }
    }

    fn init_model(init: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self { app: init.app, action: init.action }
    }
}

impl AppRow {
    fn risk_label(&self) -> String {
        self.app
            .sandbox
            .as_ref()
            .map(|s| s.risk().label().to_owned())
            .unwrap_or_default()
    }

    fn risk_icon(&self) -> Option<&'static str> {
        self.app.sandbox.as_ref().map(|s| s.risk().icon())
    }

    fn risk_classes(&self) -> Vec<&'static str> {
        match &self.app.sandbox {
            Some(sandbox) => vec!["risk-chip", sandbox.risk().css_class()],
            None => vec!["risk-chip"],
        }
    }

    /// Summary line, with the backend appended when it's ambiguous which
    /// package system an app came from.
    fn subtitle(&self) -> String {
        let summary = self.app.summary.as_deref().unwrap_or("No description available");
        match &self.app.version {
            Some(version) => format!("{summary}\n{version}"),
            None => summary.to_owned(),
        }
    }

}
