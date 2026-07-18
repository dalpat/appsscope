//! App detail page — the screen that decides whether someone installs.
//!
//! Layout follows what people actually scan: identity and the action button
//! first, then screenshots, then the summary, then facts, then changelog. The
//! changelog is present in AppStream for most apps and almost never surfaced
//! by existing software centres.

use adw::prelude::*;
use appsscope_core::{App, AppRef};
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::prelude::*;

use super::app_row::RowAction;
use super::permissions::{Permissions, PermissionsInit, PermissionsMsg, PermissionsOutput};
use crate::runtime;

/// Screenshots shown before the carousel stops loading more. Apps ship up to
/// a dozen; the first few carry the impression.
const MAX_SCREENSHOTS: usize = 6;

/// Rail slot size. Fixed so the row doesn't jolt as images stream in at
/// different aspect ratios; `ContentFit::Contain` letterboxes within it.
const SHOT_WIDTH: i32 = 460;
const SHOT_HEIGHT: i32 = 280;

pub struct Detail {
    app: App,
    installed: bool,
    busy: bool,
    /// Carousel slides, held so loaded textures can be dropped in later.
    slides: Vec<gtk::Picture>,
    permissions: Controller<Permissions>,
}

#[derive(Debug)]
pub struct DetailInit {
    pub app: App,
    pub installed: bool,
}

#[derive(Debug)]
pub enum DetailMsg {
    Action,
    SetBusy(bool),
    SetInstalled(bool),
    /// A screenshot finished downloading and is ready at this path.
    ScreenshotReady(usize, std::path::PathBuf),
    /// Sandbox profile arrived from the backend.
    SandboxLoaded(Option<appsscope_core::Sandbox>),
    PermissionChanged(String, bool),
}

#[derive(Debug)]
pub enum DetailOutput {
    Action(RowAction, AppRef),
    /// Ask the shell to look up this app's sandbox profile.
    NeedSandbox(AppRef),
    /// (app, override key, granted)
    SetPermission(AppRef, String, bool),
}

#[relm4::component(pub)]
impl SimpleComponent for Detail {
    type Init = DetailInit;
    type Input = DetailMsg;
    type Output = DetailOutput;

    view! {
        // A NavigationPage only gets a back button if its content has a
        // header bar — without this the detail page was a dead end.
        adw::ToolbarView {
            add_top_bar = &adw::HeaderBar {
                #[wrap(Some)]
                set_title_widget = &adw::WindowTitle {
                    set_title: &model.app.name,
                },
            },

            #[wrap(Some)]
            set_content = &gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_vexpand: true,

            adw::Clamp {
                set_maximum_size: 860,
                set_margin_top: 20,
                set_margin_bottom: 40,
                set_margin_start: 16,
                set_margin_end: 16,

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 28,

                    // ---- Hero: icon, name, developer, action -------------
                    gtk::Box {
                        add_css_class: "detail-hero",
                        set_spacing: 20,
                        set_valign: gtk::Align::Start,

                        gtk::Image {
                            add_css_class: "icon-tile",
                            set_overflow: gtk::Overflow::Hidden,
                            set_valign: gtk::Align::Center,
                            set_pixel_size: 96,
                            set_paintable: super::icon::paintable(&model.app, 96).as_ref(),
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 6,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,

                            gtk::Label {
                                add_css_class: "detail-name",
                                set_label: &model.app.name,
                                set_xalign: 0.0,
                                set_wrap: true,
                            },

                            gtk::Box {
                                set_spacing: 10,

                                gtk::Label {
                                    add_css_class: "detail-developer",
                                    set_label: &model.byline(),
                                    set_xalign: 0.0,
                                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                                },

                                // Beside the developer rather than down in the
                                // metrics strip: what kind of package this is
                                // should be legible before you reach for the
                                // Install button, not after.
                                gtk::Label {
                                    add_css_class: "format-badge",
                                    add_css_class: model.app.app_ref.backend.css_class(),
                                    set_label: model.app.app_ref.backend.badge_label(),
                                    set_valign: gtk::Align::Center,
                                },
                            },

                            gtk::Button {
                                set_halign: gtk::Align::Start,
                                set_margin_top: 10,
                                set_width_request: 150,

                                #[watch]
                                set_label: model.button_label(),
                                #[watch]
                                set_sensitive: !model.busy,
                                #[watch]
                                set_css_classes: &model.button_classes(),

                                connect_clicked => DetailMsg::Action,
                            },
                        },
                    },

                    // ---- Screenshots -------------------------------------
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 10,
                        set_visible: !model.app.screenshots.is_empty(),

                        gtk::ScrolledWindow {
                            set_vscrollbar_policy: gtk::PolicyType::Never,
                            set_propagate_natural_height: true,

                            #[name = "shots"]
                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 12,
                            },
                        },
                    },

                    gtk::Label {
                        set_label: model.app.summary.as_deref().unwrap_or_default(),
                        set_visible: model.app.summary.is_some(),
                        add_css_class: "title-4",
                        set_xalign: 0.0,
                        set_wrap: true,
                    },

                    // AppStream descriptions carry an HTML subset that Pango
                    // rejects, so tags are stripped rather than rendered.
                    gtk::Label {
                        set_label: &model.description(),
                        set_visible: model.app.description.is_some(),
                        set_wrap: true,
                        set_xalign: 0.0,
                        set_selectable: true,
                    },

                    // ---- Permissions -------------------------------------
                    // Placed above the facts and changelog: this is the
                    // information that should change your mind about
                    // installing, so it must not sit below the fold.
                    #[local_ref]
                    permissions_widget -> gtk::Box {},

                    // ---- Metrics -----------------------------------------
                    gtk::ScrolledWindow {
                        set_vscrollbar_policy: gtk::PolicyType::Never,
                        set_propagate_natural_height: true,

                        #[name = "facts"]
                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_halign: gtk::Align::Center,
                            set_spacing: 0,
                        },
                    },

                    // ---- Changelog ---------------------------------------
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 10,
                        set_visible: !model.app.releases.is_empty(),

                        gtk::Label {
                            add_css_class: "section-heading",
                            set_label: "What's New",
                            set_xalign: 0.0,
                        },

                        #[name = "releases"]
                        gtk::ListBox {
                            set_selection_mode: gtk::SelectionMode::None,
                            add_css_class: "boxed-list",
                        },
                    },
                },
            }
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let permissions = Permissions::builder()
            .launch(PermissionsInit { installed: init.installed })
            .forward(sender.input_sender(), |output| match output {
                PermissionsOutput::PermissionChanged(key, granted) => {
                    DetailMsg::PermissionChanged(key, granted)
                }
            });

        let mut model = Detail {
            app: init.app,
            installed: init.installed,
            busy: false,
            slides: Vec::new(),
            permissions,
        };
        let permissions_widget = model.permissions.widget();
        let widgets = view_output!();

        // The profile may need a network round trip for an app that isn't
        // installed, so it is requested rather than blocking the first paint.
        let _ = sender.output(DetailOutput::NeedSandbox(model.app.app_ref.clone()));

        // Metrics are built imperatively because which ones exist depends on
        // what the catalog knows about this particular app.
        let facts = model.facts();
        for (index, (label, value)) in facts.iter().enumerate() {
            if index > 0 {
                widgets.facts.append(&metric_divider());
            }
            widgets.facts.append(&metric(label, value));
        }

        // Placeholder slides go in immediately so the carousel has its final
        // height from the first frame — images dropping in later must not
        // reflow the page under the reader.
        for (index, shot) in model.app.screenshots.iter().take(MAX_SCREENSHOTS).enumerate() {
            let picture = gtk::Picture::new();
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_hexpand(true);
            picture.set_size_request(SHOT_WIDTH, SHOT_HEIGHT);
            picture.add_css_class("screenshot-frame");
            widgets.shots.append(&picture);
            model.slides.push(picture);

            let url = shot.url.clone();
            let sender = sender.clone();
            runtime::spawn(crate::images::fetch(url), move |path| {
                if let Some(path) = path {
                    sender.input(DetailMsg::ScreenshotReady(index, path));
                }
            });
        }

        for release in model.app.releases.iter().take(5) {
            let row = adw::ExpanderRow::new();
            row.set_use_markup(false);
            row.set_title(&release.version);
            if let Some(date) = release.timestamp.and_then(format_date) {
                row.set_subtitle(&date);
            }

            let body = gtk::Label::new(Some(&strip_markup(
                release.description.as_deref().unwrap_or("No release notes."),
            )));
            body.set_wrap(true);
            body.set_xalign(0.0);
            body.set_margin_top(6);
            body.set_margin_bottom(6);
            body.set_margin_start(12);
            body.set_margin_end(12);
            body.set_selectable(true);
            row.add_row(&body);

            widgets.releases.append(&row);
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            DetailMsg::Action => {
                // Deliberately does *not* set `busy` here. Installing goes
                // through a consent dialog that the user can cancel, and only
                // the shell knows whether a transaction actually started —
                // guessing here is what left the button stuck on "Working…"
                // after a cancel.
                let action = if self.installed { RowAction::Remove } else { RowAction::Install };
                let _ = sender.output(DetailOutput::Action(action, self.app.app_ref.clone()));
            }
            DetailMsg::SetBusy(busy) => self.busy = busy,
            DetailMsg::SetInstalled(installed) => {
                self.installed = installed;
                self.busy = false;
                self.permissions.emit(PermissionsMsg::SetInstalled(installed));
            }

            DetailMsg::SandboxLoaded(sandbox) => {
                self.permissions.emit(PermissionsMsg::SetSandbox(sandbox));
            }

            DetailMsg::PermissionChanged(key, granted) => {
                let _ = sender.output(DetailOutput::SetPermission(
                    self.app.app_ref.clone(),
                    key,
                    granted,
                ));
            }
            DetailMsg::ScreenshotReady(index, path) => {
                let Some(picture) = self.slides.get(index) else { return };
                // A corrupt or truncated download shouldn't blank the slide.
                match gdk::Texture::from_filename(&path) {
                    Ok(texture) => picture.set_paintable(Some(&texture)),
                    Err(err) => tracing::debug!(?path, %err, "screenshot decode failed"),
                }
            }
        }
    }
}

impl Detail {
    fn byline(&self) -> String {
        self.app
            .developer
            .clone()
            .unwrap_or_else(|| self.app.app_ref.id.clone())
    }

    /// The facts worth showing as pills, skipping anything unknown rather than
    /// printing a row of "Unknown".
    fn facts(&self) -> Vec<(String, String)> {
        let mut facts = Vec::new();

        if let Some(version) = &self.app.version {
            facts.push(("Version".to_owned(), version.clone()));
        }
        if let Some(license) = &self.app.license {
            facts.push(("Licence".to_owned(), humanise_license(license)));
        }
        // Format first: "Flatpak" answers "what am I installing" more
        // directly than the remote name does.
        facts.push((
            "Format".to_owned(),
            self.app.app_ref.backend.display_name().to_owned(),
        ));
        if let Some(origin) = &self.app.app_ref.origin {
            facts.push(("Source".to_owned(), origin.clone()));
        }
        if let Some(size) = self.app.installed_size.or(self.app.download_size) {
            facts.push(("Size".to_owned(), format_bytes(size)));
        }

        facts
    }

    fn button_label(&self) -> &'static str {
        match (self.busy, self.installed) {
            (true, _) => "Working…",
            (false, true) => "Remove",
            (false, false) => "Install",
        }
    }

    fn button_classes(&self) -> Vec<&'static str> {
        if self.installed {
            vec!["pill", "destructive-action"]
        } else {
            vec!["pill", "suggested-action"]
        }
    }

    fn description(&self) -> String {
        strip_markup(self.app.description.as_deref().unwrap_or_default())
    }
}

/// One metric: value above, caption below, centred.
fn metric(label: &str, value: &str) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 2);
    outer.set_margin_start(20);
    outer.set_margin_end(20);

    let value_widget = gtk::Label::new(Some(value));
    value_widget.add_css_class("metric-value");
    value_widget.set_ellipsize(gtk::pango::EllipsizeMode::End);
    value_widget.set_max_width_chars(16);

    let label_widget = gtk::Label::new(Some(label));
    label_widget.add_css_class("metric-label");

    outer.append(&value_widget);
    outer.append(&label_widget);
    outer.upcast()
}

/// Hairline rule between metrics.
fn metric_divider() -> gtk::Widget {
    let divider = gtk::Separator::new(gtk::Orientation::Vertical);
    divider.add_css_class("metric-divider");
    divider.set_margin_top(4);
    divider.set_margin_bottom(4);
    divider.upcast()
}

/// SPDX expressions are unreadable in a UI chip; show the common ones by their
/// familiar name and fall back to the raw expression otherwise.
fn humanise_license(spdx: &str) -> String {
    let trimmed = spdx.trim();
    match trimmed {
        "GPL-3.0-or-later" | "GPL-3.0-only" | "GPL-3.0" => "GPL 3",
        "GPL-2.0-or-later" | "GPL-2.0-only" | "GPL-2.0" => "GPL 2",
        "LGPL-3.0-or-later" | "LGPL-3.0-only" => "LGPL 3",
        "LGPL-2.1-or-later" | "LGPL-2.1-only" => "LGPL 2.1",
        "MIT" => "MIT",
        "Apache-2.0" => "Apache 2",
        "MPL-2.0" => "MPL 2",
        "AGPL-3.0-or-later" | "AGPL-3.0-only" => "AGPL 3",
        "BSD-3-Clause" => "BSD 3-Clause",
        "BSD-2-Clause" => "BSD 2-Clause",
        "LicenseRef-proprietary" => "Proprietary",
        other if other.starts_with("LicenseRef-") => "Proprietary",
        other => other,
    }
    .to_owned()
}

/// Strip the HTML subset AppStream permits.
///
/// Pango markup is not HTML — feeding it `<p>`/`<ul>` produces parse errors and
/// a blank label, so tags are removed and block elements become newlines.
fn strip_markup(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    let mut tag = String::new();

    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag.trim_start_matches('/').trim().to_lowercase();
                if matches!(name.as_str(), "p" | "br" | "li" | "ul" | "ol") {
                    out.push('\n');
                }
                if name.starts_with("li") {
                    out.push_str("• ");
                }
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }

    let mut cleaned = String::with_capacity(out.len());
    let mut blank = 0;
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        cleaned.push_str(line);
        cleaned.push('\n');
    }
    cleaned.trim().to_owned()
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "kB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_date(timestamp: i64) -> Option<String> {
    let datetime = glib::DateTime::from_unix_utc(timestamp).ok()?;
    datetime.format("%e %B %Y").ok().map(|s| s.trim().to_owned())
}

use relm4::gtk::glib;
