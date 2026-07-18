//! The navigation sidebar.
//!
//! Every destination lives here — the three top-level views and the category
//! filters — deliberately in one list rather than split between a sidebar and
//! a top switcher. Two parallel navigation systems means memorising which kind
//! of destination lives where, and the horizontal chip strip it replaces hid
//! most of the categories off the right edge.

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

/// Where the content area should be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Featured shelves.
    Home,
    Installed,
    Updates,
    /// A category filter, by AppStream category name.
    Category(String),
}

/// Top-level views. Built at runtime rather than declared as a `const`
/// because `Destination::Category` owns a `String`.
fn primary() -> Vec<(&'static str, &'static str, Destination)> {
    vec![
        ("Home", "go-home-symbolic", Destination::Home),
        ("Installed", "computer-symbolic", Destination::Installed),
        (
            "Updates",
            "software-update-available-symbolic",
            Destination::Updates,
        ),
    ]
}

fn categories() -> Vec<(&'static str, &'static str, Destination)> {
    vec![
        ("Games", "applications-games-symbolic", Destination::Category("Game".into())),
        ("Audio & Video", "applications-multimedia-symbolic", Destination::Category("AudioVideo".into())),
        ("Graphics & Design", "applications-graphics-symbolic", Destination::Category("Graphics".into())),
        ("Developer Tools", "applications-engineering-symbolic", Destination::Category("Development".into())),
        ("Productivity", "text-editor-symbolic", Destination::Category("Office".into())),
        ("Education", "accessories-dictionary-symbolic", Destination::Category("Education".into())),
        ("Science", "applications-science-symbolic", Destination::Category("Science".into())),
        ("System", "applications-system-symbolic", Destination::Category("System".into())),
        ("Utilities", "applications-utilities-symbolic", Destination::Category("Utility".into())),
    ]
}

/// Human-readable name for an AppStream category, for page titles.
pub fn category_label(category: &str) -> &'static str {
    categories()
        .into_iter()
        .find(|(_, _, destination)| {
            matches!(destination, Destination::Category(c) if c == category)
        })
        .map(|(label, _, _)| label)
        .unwrap_or("Apps")
}

pub struct Sidebar {
    /// Both lists are held so selecting in one can clear the other — GTK has
    /// no built-in notion of a selection shared across list boxes.
    primary_list: gtk::ListBox,
    category_list: gtk::ListBox,
    update_badge: gtk::Label,
}

#[derive(Debug)]
pub enum SidebarMsg {
    Selected(Destination),
    /// Move the selection programmatically, as a click would.
    Select(Destination),
    /// Number of pending updates, for the badge.
    SetUpdateCount(usize),
}

#[derive(Debug)]
pub enum SidebarOutput {
    Navigate(Destination),
}

#[relm4::component(pub)]
impl SimpleComponent for Sidebar {
    type Init = ();
    type Input = SidebarMsg;
    type Output = SidebarOutput;

    view! {
        gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_vexpand: true,

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,

                #[local_ref]
                primary_list -> gtk::ListBox {
                    add_css_class: "navigation-sidebar",
                },

                gtk::Label {
                    add_css_class: "sidebar-heading",
                    set_label: "Categories",
                    set_xalign: 0.0,
                    set_margin_start: 18,
                    set_margin_top: 14,
                    set_margin_bottom: 4,
                },

                #[local_ref]
                category_list -> gtk::ListBox {
                    add_css_class: "navigation-sidebar",
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let primary_list = gtk::ListBox::new();
        let category_list = gtk::ListBox::new();
        let update_badge = gtk::Label::new(None);
        update_badge.add_css_class("update-badge");
        update_badge.set_visible(false);

        for (label, icon, destination) in primary() {
            let show_badge = destination == Destination::Updates;
            primary_list.append(&row(label, icon, show_badge.then(|| update_badge.clone())));
        }
        for (label, icon, _) in categories() {
            category_list.append(&row(label, icon, None));
        }

        let model = Sidebar {
            primary_list: primary_list.clone(),
            category_list: category_list.clone(),
            update_badge,
        };

        // Home is where the app opens.
        if let Some(first) = primary_list.row_at_index(0) {
            primary_list.select_row(Some(&first));
        }

        wire_selection(&primary_list, &category_list, primary(), &sender);
        wire_selection(&category_list, &primary_list, categories(), &sender);

        let primary_list = &model.primary_list;
        let category_list = &model.category_list;
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            SidebarMsg::Selected(destination) => {
                let _ = sender.output(SidebarOutput::Navigate(destination));
            }
            SidebarMsg::Select(destination) => {
                // Selecting the row emits `Selected` through the normal
                // handler, so the highlight and the content cannot disagree.
                let primary = primary();
                if let Some(index) = primary.iter().position(|(_, _, d)| *d == destination) {
                    if let Some(row) = self.primary_list.row_at_index(index as i32) {
                        self.primary_list.select_row(Some(&row));
                    }
                } else if let Some(index) =
                    categories().iter().position(|(_, _, d)| *d == destination)
                {
                    if let Some(row) = self.category_list.row_at_index(index as i32) {
                        self.category_list.select_row(Some(&row));
                    }
                }
            }

            SidebarMsg::SetUpdateCount(count) => {
                self.update_badge.set_visible(count > 0);
                self.update_badge.set_label(&count.to_string());
            }
        }
    }
}

/// Build one sidebar row: icon, label, optional trailing badge.
fn row(label: &str, icon: &str, badge: Option<gtk::Label>) -> gtk::ListBoxRow {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.set_margin_top(9);
    content.set_margin_bottom(9);
    content.set_margin_start(6);
    content.set_margin_end(6);

    let image = gtk::Image::from_icon_name(icon);
    content.append(&image);

    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    text.set_hexpand(true);
    text.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&text);

    if let Some(badge) = badge {
        content.append(&badge);
    }

    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&content));
    row
}

/// Connect a list's selection, clearing the sibling list's selection.
///
/// The two lists are separate widgets, so without this both can show a
/// selected row at once and the sidebar reads as having two current pages.
fn wire_selection(
    list: &gtk::ListBox,
    sibling: &gtk::ListBox,
    entries: Vec<(&'static str, &'static str, Destination)>,
    sender: &ComponentSender<Sidebar>,
) {
    let sibling = sibling.clone();
    let sender = sender.clone();

    list.connect_row_selected(move |_, row| {
        // Clearing the sibling re-enters this handler with `None`, which the
        // guard below returns on — so no explicit recursion flag is needed.
        let Some(row) = row else { return };
        // `unselect_all` is a no-op in single-selection mode, which left both
        // lists showing a selected row at once.
        sibling.select_row(None::<&gtk::ListBoxRow>);

        let index = row.index();
        if let Some((_, _, destination)) = entries.get(index as usize) {
            sender.input(SidebarMsg::Selected(destination.clone()));
        }
    });
}
