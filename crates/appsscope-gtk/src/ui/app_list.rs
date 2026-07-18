//! A scrollable list of apps with loading / empty / error states.
//!
//! Used for Explore, Installed, and Updates — they differ only in where the
//! apps come from and what the row buttons do, so the widget is shared.

use adw::prelude::*;
use appsscope_core::{App, AppRef};
use relm4::factory::FactoryVecDeque;
use relm4::gtk;
use relm4::prelude::*;

use super::app_row::{AppRow, AppRowInit, AppRowOutput, RowAction};

/// Which of the four states the list is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Loading,
    Ready,
    Empty,
    Failed(String),
}

pub struct AppList {
    rows: FactoryVecDeque<AppRow>,
    state: State,
    /// Shown on the empty page — differs per list ("No updates" vs "Nothing installed").
    empty_title: &'static str,
    empty_description: &'static str,
    empty_icon: &'static str,
}

#[derive(Debug)]
pub struct AppListInit {
    pub empty_title: &'static str,
    pub empty_description: &'static str,
    pub empty_icon: &'static str,
}

#[derive(Debug)]
pub enum AppListMsg {
    /// Replace the contents. `Ok(vec![])` renders the empty state, not a blank list.
    ///
    /// The action is per row, not per list, so Explore can show "Install" on
    /// apps you don't have and "Remove" on the ones you do.
    SetApps(Result<Vec<(App, RowAction)>, String>),
    /// Go back to the spinner, e.g. on refresh.
    SetLoading,
    Activated(AppRef),
    Action(RowAction, AppRef),
}

#[derive(Debug)]
pub enum AppListOutput {
    /// User tapped a row — the shell should push a detail page.
    OpenDetail(AppRef),
    /// User tapped a row button — the shell owns the transaction.
    Action(RowAction, AppRef),
}

#[relm4::component(pub)]
impl SimpleComponent for AppList {
    type Init = AppListInit;
    type Input = AppListMsg;
    type Output = AppListOutput;

    view! {
        gtk::Stack {
            set_transition_type: gtk::StackTransitionType::Crossfade,

            add_named[Some("loading")] = &adw::Spinner {
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
                set_width_request: 48,
                set_height_request: 48,
            },

            add_named[Some("list")] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_vexpand: true,

                adw::Clamp {
                    set_maximum_size: 900,
                    set_margin_top: 18,
                    set_margin_bottom: 18,
                    set_margin_start: 12,
                    set_margin_end: 12,

                    #[local_ref]
                    row_box -> gtk::ListBox {
                        set_selection_mode: gtk::SelectionMode::None,
                        set_valign: gtk::Align::Start,
                        add_css_class: "boxed-list",
                    },
                },
            },

            add_named[Some("empty")] = &adw::StatusPage {
                set_icon_name: Some(model.empty_icon),
                set_title: model.empty_title,
                set_description: Some(model.empty_description),
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

            // Must come after the `add_named` calls: the view macro applies
            // properties in source order, and selecting a child by name before
            // it has been added is a runtime warning plus a blank page.
            #[watch]
            set_visible_child_name: match &model.state {
                State::Loading => "loading",
                State::Ready => "list",
                State::Empty => "empty",
                State::Failed(_) => "error",
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let rows = FactoryVecDeque::builder()
            .launch(gtk::ListBox::default())
            .forward(sender.input_sender(), |output| match output {
                AppRowOutput::Activated(id) => AppListMsg::Activated(id),
                AppRowOutput::Action(action, id) => AppListMsg::Action(action, id),
            });

        let model = AppList {
            rows,
            state: State::Loading,
            empty_title: init.empty_title,
            empty_description: init.empty_description,
            empty_icon: init.empty_icon,
        };

        let row_box = model.rows.widget();
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            AppListMsg::SetLoading => self.state = State::Loading,

            AppListMsg::SetApps(Ok(apps)) => {
                let mut guard = self.rows.guard();
                guard.clear();
                for (app, action) in &apps {
                    guard.push_back(AppRowInit { app: app.clone(), action: *action });
                }
                drop(guard);
                self.state = if apps.is_empty() { State::Empty } else { State::Ready };
            }

            AppListMsg::SetApps(Err(err)) => {
                self.rows.guard().clear();
                self.state = State::Failed(err);
            }

            AppListMsg::Activated(id) => {
                let _ = sender.output(AppListOutput::OpenDetail(id));
            }

            AppListMsg::Action(action, id) => {
                let _ = sender.output(AppListOutput::Action(action, id));
            }
        }
    }
}
