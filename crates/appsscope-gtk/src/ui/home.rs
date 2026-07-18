//! The Explore landing page — a vertical stack of horizontal shelves.
//!
//! This replaces the uniform grid as the default view. A grid of 200
//! alphabetically-ordered apps gives no reason to look at any particular one;
//! shelves impose an editorial shape ("Recently updated", "Games") that makes
//! browsing feel like discovery instead of scrolling a directory.
//!
//! The grid is still used, but for search results and category filters, where
//! a uniform ranked list is genuinely what you want.

use appsscope_core::{App, AppRef};
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use super::hero::{Hero, HeroMsg, HeroOutput};
use super::shelf::{Shelf, ShelfInit, ShelfMsg, ShelfOutput};

/// The shelves, in display order.
///
/// `None` as the category means the shelf is built from a bespoke query
/// rather than a category filter, and therefore has no "See all".
pub const SHELVES: &[(&str, Option<&str>)] = &[
    ("Recently updated", None),
    ("Games", Some("Game")),
    ("Audio & Video", Some("AudioVideo")),
    ("Graphics & Design", Some("Graphics")),
    ("Developer Tools", Some("Development")),
    ("Productivity", Some("Office")),
];

pub struct Home {
    hero: Controller<Hero>,
    shelves: Vec<Controller<Shelf>>,
    loading: bool,
}

#[derive(Debug)]
pub enum HomeMsg {
    SetLoading,
    /// One `Vec` per entry in [`SHELVES`], in the same order.
    SetShelves(Vec<Vec<(App, bool)>>, bool),
    SetFeatured(Vec<App>),
    OpenDetail(AppRef),
    SeeAll(Option<String>),
}

#[derive(Debug)]
pub enum HomeOutput {
    OpenDetail(AppRef),
    SeeAll(Option<String>),
}

#[relm4::component(pub)]
impl SimpleComponent for Home {
    type Init = ();
    type Input = HomeMsg;
    type Output = HomeOutput;

    view! {
        gtk::Stack {
            set_transition_type: gtk::StackTransitionType::Crossfade,
            set_vexpand: true,

            add_named[Some("loading")] = &adw::Spinner {
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
                set_width_request: 42,
                set_height_request: 42,
            },

            add_named[Some("shelves")] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_vexpand: true,

                adw::Clamp {
                    set_maximum_size: 1160,
                    set_tightening_threshold: 900,

                    #[local_ref]
                    shelf_box -> gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 18,
                        set_margin_top: 8,
                        set_margin_bottom: 28,
                    },
                },
            },

            #[watch]
            set_visible_child_name: if model.loading { "loading" } else { "shelves" },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let shelf_box = gtk::Box::new(gtk::Orientation::Vertical, 18);

        let hero = Hero::builder()
            .launch(())
            .forward(sender.input_sender(), |output| match output {
                HeroOutput::OpenDetail(id) => HomeMsg::OpenDetail(id),
            });
        shelf_box.append(hero.widget());

        let shelves: Vec<Controller<Shelf>> = SHELVES
            .iter()
            .map(|(title, category)| {
                let controller = Shelf::builder()
                    .launch(ShelfInit {
                        title: (*title).to_owned(),
                        category: category.map(str::to_owned),
                    })
                    .forward(sender.input_sender(), |output| match output {
                        ShelfOutput::OpenDetail(id) => HomeMsg::OpenDetail(id),
                        ShelfOutput::SeeAll(category) => HomeMsg::SeeAll(category),
                    });
                shelf_box.append(controller.widget());
                controller
            })
            .collect();

        let model = Home { hero, shelves, loading: true };
        let shelf_box = &shelf_box;
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            HomeMsg::SetLoading => self.loading = true,

            HomeMsg::SetFeatured(apps) => {
                self.hero.emit(HeroMsg::SetApps(apps));
            }

            HomeMsg::SetShelves(rows, show_format) => {
                for (shelf, apps) in self.shelves.iter().zip(rows) {
                    shelf.emit(ShelfMsg::SetApps(apps, show_format));
                }
                self.loading = false;
            }

            HomeMsg::OpenDetail(id) => {
                let _ = sender.output(HomeOutput::OpenDetail(id));
            }

            HomeMsg::SeeAll(category) => {
                let _ = sender.output(HomeOutput::SeeAll(category));
            }
        }
    }
}

use relm4::adw;
