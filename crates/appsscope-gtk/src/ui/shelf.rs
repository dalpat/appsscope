//! One horizontal shelf: a heading plus a scrolling row of tiles.

use appsscope_core::{App, AppRef};
use relm4::factory::FactoryVecDeque;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use super::app_tile::{AppTile, AppTileInit, AppTileOutput};

pub struct Shelf {
    tiles: FactoryVecDeque<AppTile>,
    title: String,
    /// Category this shelf represents, for the "See all" jump.
    category: Option<String>,
    empty: bool,
}

#[derive(Debug)]
pub struct ShelfInit {
    pub title: String,
    pub category: Option<String>,
}

#[derive(Debug)]
pub enum ShelfMsg {
    SetApps(Vec<(App, bool)>, bool),
    Activated(AppRef),
    SeeAll,
}

#[derive(Debug)]
pub enum ShelfOutput {
    OpenDetail(AppRef),
    /// "See all" was clicked — the shell switches to the filtered grid.
    SeeAll(Option<String>),
}

#[relm4::component(pub)]
impl SimpleComponent for Shelf {
    type Init = ShelfInit;
    type Input = ShelfMsg;
    type Output = ShelfOutput;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 4,
            // A shelf with nothing in it is hidden rather than shown empty —
            // an empty row reads as a loading failure.
            #[watch]
            set_visible: !model.empty,

            gtk::Box {
                set_margin_start: 16,
                set_margin_end: 16,
                set_margin_top: 8,

                gtk::Label {
                    add_css_class: "shelf-title",
                    set_label: &model.title,
                    set_xalign: 0.0,
                    set_hexpand: true,
                },

                gtk::Button {
                    add_css_class: "flat",
                    add_css_class: "shelf-more",
                    set_label: "See all",
                    set_valign: gtk::Align::Center,
                    set_visible: model.category.is_some(),
                    connect_clicked => ShelfMsg::SeeAll,
                },
            },

            gtk::ScrolledWindow {
                set_vscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,

                #[local_ref]
                tile_box -> gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 4,
                    set_margin_start: 12,
                    set_margin_end: 12,
                    set_margin_bottom: 4,
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let tiles = FactoryVecDeque::builder()
            .launch(gtk::Box::default())
            .forward(sender.input_sender(), |output| match output {
                AppTileOutput::Activated(id) => ShelfMsg::Activated(id),
            });

        let model = Shelf {
            tiles,
            title: init.title,
            category: init.category,
            empty: true,
        };

        let tile_box = model.tiles.widget();
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            ShelfMsg::SetApps(apps, show_format) => {
                let mut guard = self.tiles.guard();
                guard.clear();
                for (app, installed) in &apps {
                    guard.push_back(AppTileInit {
                        app: app.clone(),
                        installed: *installed,
                        show_format,
                    });
                }
                drop(guard);
                self.empty = apps.is_empty();
            }
            ShelfMsg::Activated(id) => {
                let _ = sender.output(ShelfOutput::OpenDetail(id));
            }
            ShelfMsg::SeeAll => {
                let _ = sender.output(ShelfOutput::SeeAll(self.category.clone()));
            }
        }
    }
}
