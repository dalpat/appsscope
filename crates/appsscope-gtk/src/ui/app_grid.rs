//! The browse grid — a responsive wall of [`AppCard`]s.
//!
//! `GtkFlowBox` reflows columns to the window width on its own, which is what
//! makes the grid feel native when resized rather than snapping between fixed
//! breakpoints.

use adw::prelude::*;
use appsscope_core::{App, AppRef};
use relm4::factory::FactoryVecDeque;
use relm4::gtk;
use relm4::prelude::*;

use super::app_card::{AppCard, AppCardInit, AppCardOutput};

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Loading,
    Ready,
    Empty,
    Failed(String),
}

pub struct AppGrid {
    cards: FactoryVecDeque<AppCard>,
    state: State,
}

#[derive(Debug)]
pub enum AppGridMsg {
    /// Apps plus whether each is already installed.
    SetApps(Result<Vec<(App, bool)>, String>),
    SetLoading,
    Activated(AppRef),
}

#[derive(Debug)]
pub enum AppGridOutput {
    OpenDetail(AppRef),
}

#[relm4::component(pub)]
impl SimpleComponent for AppGrid {
    type Init = ();
    type Input = AppGridMsg;
    type Output = AppGridOutput;

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

            add_named[Some("grid")] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_vexpand: true,

                adw::Clamp {
                    set_maximum_size: 1160,
                    set_tightening_threshold: 900,

                    #[local_ref]
                    card_box -> gtk::FlowBox {
                        set_selection_mode: gtk::SelectionMode::None,
                        set_valign: gtk::Align::Start,
                        set_homogeneous: true,
                        set_column_spacing: 14,
                        set_row_spacing: 14,
                        set_margin_top: 6,
                        set_margin_bottom: 24,
                        set_margin_start: 16,
                        set_margin_end: 16,
                        // Bounds rather than a fixed count — FlowBox picks the
                        // column count from the available width between these.
                        set_min_children_per_line: 1,
                        set_max_children_per_line: 3,
                    },
                },
            },

            add_named[Some("empty")] = &adw::StatusPage {
                set_icon_name: Some("system-search-symbolic"),
                set_title: "No apps found",
                set_description: Some("Try a different search or category."),
            },

            add_named[Some("error")] = &adw::StatusPage {
                set_icon_name: Some("dialog-warning-symbolic"),
                set_title: "Something went wrong",
                #[watch]
                set_description: Some(match &model.state {
                    State::Failed(err) => err.as_str(),
                    _ => "",
                }),
            },

            // After the children exist — see the note in `app_list`.
            #[watch]
            set_visible_child_name: match &model.state {
                State::Loading => "loading",
                State::Ready => "grid",
                State::Empty => "empty",
                State::Failed(_) => "error",
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cards = FactoryVecDeque::builder()
            .launch(gtk::FlowBox::default())
            .forward(sender.input_sender(), |output| match output {
                AppCardOutput::Activated(id) => AppGridMsg::Activated(id),
            });

        let model = AppGrid { cards, state: State::Loading };
        let card_box = model.cards.widget();
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            AppGridMsg::SetLoading => self.state = State::Loading,

            AppGridMsg::SetApps(Ok(apps)) => {
                let mut guard = self.cards.guard();
                guard.clear();
                for (app, installed) in &apps {
                    guard.push_back(AppCardInit { app: app.clone(), installed: *installed });
                }
                drop(guard);
                self.state = if apps.is_empty() { State::Empty } else { State::Ready };
            }

            AppGridMsg::SetApps(Err(err)) => {
                self.cards.guard().clear();
                self.state = State::Failed(err);
            }

            AppGridMsg::Activated(id) => {
                let _ = sender.output(AppGridOutput::OpenDetail(id));
            }
        }
    }
}
