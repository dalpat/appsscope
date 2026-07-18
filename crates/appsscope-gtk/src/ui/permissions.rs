//! The permissions panel — AppsScope's reason to exist.
//!
//! Every other software centre reduces the sandbox to a single vague label.
//! This shows each permission in plain language, grades it, explains the
//! consequence, and — for installed apps — lets you switch it off without
//! leaving the store.
//!
//! Two rules the UI must not break:
//!
//! 1. Never imply an app is safe when we simply haven't looked. "Unknown" and
//!    "sandboxed" are different states and are rendered differently.
//! 2. Never offer a switch that only breaks the app. Permissions with no
//!    `override_key` (Wayland, GPU) are shown but not toggleable.

use adw::prelude::*;
use appsscope_core::{Permission, Risk, Sandbox};
use relm4::gtk;
use relm4::prelude::*;

pub struct Permissions {
    sandbox: Option<Sandbox>,
    /// The permission rows are built imperatively, so the list box is held
    /// here — the component macro owns `update_view`, and rebuilding on every
    /// view pass would destroy a switch mid-flip.
    rows: gtk::ListBox,
    /// Toggling is only meaningful once the app is on disk.
    installed: bool,
    loading: bool,
    /// Set when the backend can't confine this format at all.
    unconfined: bool,
}

#[derive(Debug)]
pub struct PermissionsInit {
    pub installed: bool,
}

#[derive(Debug)]
pub enum PermissionsMsg {
    SetLoading,
    /// `None` means the packaging format provides no sandbox at all.
    SetSandbox(Option<Sandbox>),
    SetInstalled(bool),
    Toggle(String, bool),
}

#[derive(Debug)]
pub enum PermissionsOutput {
    /// (override key, granted)
    PermissionChanged(String, bool),
}

#[relm4::component(pub)]
impl SimpleComponent for Permissions {
    type Init = PermissionsInit;
    type Input = PermissionsMsg;
    type Output = PermissionsOutput;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            gtk::Box {
                set_spacing: 10,

                gtk::Label {
                    add_css_class: "section-heading",
                    set_label: "Permissions",
                    set_xalign: 0.0,
                    set_hexpand: true,
                },

                // The verdict badge — the single most important pixel on the
                // page, so it sits level with the heading rather than buried
                // in the list below.
                gtk::Box {
                    add_css_class: "risk-badge",
                    #[watch]
                    set_css_classes: &model.badge_classes(),
                    #[watch]
                    set_visible: !model.loading,
                    set_spacing: 6,

                    gtk::Image {
                        #[watch]
                        set_icon_name: Some(model.badge_icon()),
                        set_pixel_size: 16,
                    },
                    gtk::Label {
                        #[watch]
                        set_label: &model.badge_label(),
                    },
                },
            },

            gtk::Label {
                #[watch]
                set_label: &model.explanation(),
                #[watch]
                set_visible: !model.loading,
                set_xalign: 0.0,
                set_wrap: true,
                add_css_class: "dim-label",
            },

            adw::Spinner {
                #[watch]
                set_visible: model.loading,
                set_halign: gtk::Align::Start,
                set_width_request: 22,
                set_height_request: 22,
            },

            #[local_ref]
            rows -> gtk::ListBox {
                set_selection_mode: gtk::SelectionMode::None,
                add_css_class: "boxed-list",
                #[watch]
                set_visible: !model.loading && model.has_rows(),
            },

            gtk::Label {
                set_label: "Switch a permission off to revoke it. Changes apply the next \
                            time the app starts, and are shared with Flatseal and the \
                            flatpak command line.",
                #[watch]
                set_visible: model.installed && model.has_rows(),
                set_xalign: 0.0,
                set_wrap: true,
                add_css_class: "caption",
                add_css_class: "dim-label",
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = Permissions {
            sandbox: None,
            rows: gtk::ListBox::new(),
            installed: init.installed,
            loading: true,
            unconfined: false,
        };
        let rows = &model.rows;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            PermissionsMsg::SetLoading => self.loading = true,

            PermissionsMsg::SetSandbox(sandbox) => {
                self.loading = false;
                self.unconfined = sandbox.is_none();
                self.sandbox = sandbox;
                self.rebuild(&sender);
            }

            PermissionsMsg::SetInstalled(installed) => {
                self.installed = installed;
                self.rebuild(&sender);
            }

            PermissionsMsg::Toggle(key, granted) => {
                // Optimistic: flip local state so the badge updates instantly,
                // and let the shell surface an error if the write fails.
                if let Some(sandbox) = &mut self.sandbox {
                    for permission in &mut sandbox.permissions {
                        if permission.override_key.as_deref() == Some(key.as_str()) {
                            permission.revoked = !granted;
                        }
                    }
                }
                let _ = sender.output(PermissionsOutput::PermissionChanged(key, granted));
            }
        }
    }

}

impl Permissions {
    fn has_rows(&self) -> bool {
        self.sandbox.as_ref().is_some_and(|s| !s.is_empty())
    }

    fn risk(&self) -> Option<Risk> {
        self.sandbox.as_ref().map(Sandbox::risk)
    }

    fn badge_label(&self) -> String {
        match (&self.sandbox, self.unconfined) {
            (_, true) => "Not sandboxed".to_owned(),
            (Some(sandbox), _) => sandbox.risk().label().to_owned(),
            (None, _) => "Unknown".to_owned(),
        }
    }

    fn badge_icon(&self) -> &'static str {
        match self.risk() {
            Some(risk) if !self.unconfined => risk.icon(),
            _ => "dialog-question-symbolic",
        }
    }

    fn badge_classes(&self) -> Vec<&'static str> {
        let variant = match self.risk() {
            Some(risk) if !self.unconfined => risk.css_class(),
            _ => "risk-unknown",
        };
        vec!["risk-badge", variant]
    }

    fn explanation(&self) -> String {
        if self.unconfined {
            return "This package format provides no sandbox. Once installed, it runs with \
                    the same access to your files as you do."
                .to_owned();
        }
        match &self.sandbox {
            Some(sandbox) => format!("{} {}", sandbox.risk().summary(), sandbox.headline()),
            None => "AppsScope could not read this app's permissions.".to_owned(),
        }
    }

    /// Rebuild the permission rows.
    ///
    /// Done imperatively rather than through a factory because the rows carry
    /// switches whose state has to survive being re-read from the backend, and
    /// the list is short enough that rebuilding is cheaper than diffing.
    fn rebuild(&self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }

        let Some(sandbox) = &self.sandbox else { return };

        for permission in sandbox.sorted() {
            self.rows.append(&self.build_row(permission, sender));
        }
    }

    fn build_row(
        &self,
        permission: &Permission,
        sender: &ComponentSender<Self>,
    ) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&permission.summary);
        row.set_subtitle(&permission.detail);
        row.set_subtitle_lines(4);

        let icon = gtk::Image::from_icon_name(permission.risk.icon());
        icon.add_css_class(permission.risk.css_class());
        row.add_prefix(&icon);

        match (&permission.override_key, self.installed) {
            // Toggleable only when the app is installed — there is nothing to
            // override until the app exists on disk.
            (Some(key), true) => {
                let switch = gtk::Switch::new();
                switch.set_valign(gtk::Align::Center);
                switch.set_active(!permission.revoked);

                let key = key.clone();
                let sender = sender.clone();
                switch.connect_state_set(move |_, granted| {
                    sender.input(PermissionsMsg::Toggle(key.clone(), granted));
                    glib::Propagation::Proceed
                });

                row.add_suffix(&switch);
                row.set_activatable_widget(Some(&switch));
            }

            // Required to function — say so rather than showing a dead switch.
            (None, _) => {
                let label = gtk::Label::new(Some("Required"));
                label.add_css_class("dim-label");
                label.add_css_class("caption");
                row.add_suffix(&label);
            }

            (Some(_), false) => {}
        }

        row
    }
}

use relm4::gtk::glib;
