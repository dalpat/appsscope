mod backends;
mod images;
mod runtime;
mod screenshot;
mod ui;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use appsscope_catalog::{Catalog, Filter};
use appsscope_core::{
    App, AppRef, BackendId, InstallPlan, Phase, Progress, Risk, Sandbox, format_size,
};
use backends::Registry;
use relm4::gtk;
use relm4::prelude::*;

use ui::app_grid::{AppGrid, AppGridMsg, AppGridOutput};
use ui::home::{Home, HomeMsg, HomeOutput, SHELVES};
use ui::app_list::{AppList, AppListInit, AppListMsg, AppListOutput};
use ui::app_row::RowAction;
use ui::detail::{Detail, DetailInit, DetailMsg, DetailOutput};
use ui::sidebar::{Destination, Sidebar, SidebarMsg, SidebarOutput};

const APP_ID: &str = "io.github.dalpat.AppsScope";

/// How many apps Explore shows before you search or filter.
///
/// Flathub has ~3300; building a card for every one costs a visibly slow first
/// paint for a grid nobody scrolls to the end of. Search reaches the rest.
const BROWSE_LIMIT: usize = 200;
const SEARCH_LIMIT: usize = 100;
/// Tiles per shelf. Enough to fill the row and imply more beyond the edge.
const SHELF_LIMIT: usize = 16;
/// Hero cards. Each downloads a full-size screenshot, so this stays small.
const FEATURED_LIMIT: usize = 6;

struct Shell {
    registry: Option<Arc<Registry>>,
    catalog: Option<Arc<Mutex<Catalog>>>,
    /// App ids known to be installed, for badging cards and choosing actions.
    installed_ids: HashSet<String>,

    /// Shelf landing page — the default Explore view.
    home: Controller<Home>,
    /// Uniform grid, used for search results and category filters.
    explore: Controller<AppGrid>,
    installed: Controller<AppList>,
    updates: Controller<AppList>,
    detail: Option<Controller<Detail>>,
    /// Which app the open detail page is showing, if any.
    open_app: Option<AppRef>,

    search: String,
    /// Which sidebar destination is current. Search overrides it while active.
    destination: Destination,
    /// Packaging format filter. `None` shows every format.
    format_filter: Option<BackendId>,
    /// Formats offered by the filter, in dropdown order. Index 0 is "All".
    format_options: Vec<Option<BackendId>>,
    sidebar: Controller<Sidebar>,

    /// True when the window is narrow enough that the sidebar is an overlay.
    collapsed: bool,
    show_sidebar: bool,

    /// Set while a transaction is in flight — blocks starting a second one.
    busy: bool,
    progress: Option<f64>,
    status: String,

    // `update()` never sees the widget tree, so the widgets it drives
    // imperatively are held here. These are refcounted handles, not copies.
    nav: adw::NavigationView,
    split: adw::NavigationSplitView,
    format_box: gtk::Box,
    toasts: adw::ToastOverlay,
    /// Needed to parent modal dialogs.
    window: adw::ApplicationWindow,
}

#[derive(Debug)]
enum Msg {
    Ready(Arc<Registry>, Option<Arc<Mutex<Catalog>>>, usize),
    ReloadInstalled,
    ReloadExplore,
    InstalledLoaded(Result<Vec<App>, String>),
    UpdatesLoaded(Result<Vec<App>, String>),
    ExploreLoaded(Result<Vec<App>, String>),
    ShelvesLoaded(Vec<Vec<App>>),
    FeaturedLoaded(Vec<App>),
    SearchChanged(String),
    Navigate(Destination),
    SetCollapsed(bool),
    ToggleSidebar(bool),
    FormatFilterChanged(u32),
    OpenDetail(AppRef),
    Act(RowAction, AppRef),
    /// Show what an app will be able to do, before installing it.
    ConfirmInstall(AppRef, Option<Sandbox>, Option<InstallPlan>),
    /// User accepted; actually run the transaction.
    StartTransaction(RowAction, AppRef),
    NeedSandbox(AppRef),
    SandboxLoaded(Option<Sandbox>),
    SetPermission(AppRef, String, bool),
    TransactionTick(Result<Progress, String>),
    TransactionFinished,
    Toast(String),
}

#[relm4::component]
impl SimpleComponent for Shell {
    type Init = screenshot::Job;
    type Input = Msg;
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_title: Some("AppsScope"),
            set_default_width: 1240,
            set_default_height: 820,

            #[local_ref]
            toasts -> adw::ToastOverlay {

                #[local_ref]
                split -> adw::NavigationSplitView {
                    set_min_sidebar_width: 220.0,
                    set_max_sidebar_width: 280.0,

                    #[wrap(Some)]
                    set_sidebar = &adw::NavigationPage {
                        set_title: "AppsScope",

                        #[wrap(Some)]
                        set_child = &adw::ToolbarView {
                            add_top_bar = &adw::HeaderBar {
                                #[wrap(Some)]
                                set_title_widget = &adw::WindowTitle {
                                    set_title: "AppsScope",
                                },

                                // Scope for the whole window, so it lives with
                                // the app title rather than beside the search
                                // box — it is not a property of the search.
                                pack_end = &gtk::MenuButton {
                                    // `view-filter-symbolic` and
                                    // `funnel-symbolic` are both absent from
                                    // Adwaita and rendered as a broken-image
                                    // glyph; every icon name in this codebase
                                    // is now checked against the theme.
                                    set_icon_name: "selection-mode-symbolic",
                                    set_tooltip_text: Some("Filter by package format"),
                                    #[watch]
                                    set_css_classes: if model.format_filter.is_some() {
                                        &["flat", "accent"]
                                    } else {
                                        &["flat"]
                                    },

                                    #[wrap(Some)]
                                    set_popover = &gtk::Popover {
                                        #[local_ref]
                                        format_box -> gtk::Box {
                                            set_orientation: gtk::Orientation::Vertical,
                                            set_spacing: 2,
                                            set_margin_top: 4,
                                            set_margin_bottom: 4,
                                            set_margin_start: 4,
                                            set_margin_end: 4,
                                        },
                                    },
                                },
                            },

                            #[wrap(Some)]
                            #[local_ref]
                            set_content = sidebar_widget -> gtk::ScrolledWindow {},
                        },
                    },

                    #[wrap(Some)]
                    set_content = &adw::NavigationPage {
                        set_title: "Apps",

                        #[wrap(Some)]
                        #[local_ref]
                        set_child = nav -> adw::NavigationView {

                            add = &adw::NavigationPage {
                                #[watch]
                                set_title: &model.page_title(),
                                set_tag: Some("main"),

                                #[wrap(Some)]
                                set_child = &adw::ToolbarView {
                                    add_top_bar = &adw::HeaderBar {
                                        // Shown only when the split view has
                                        // collapsed the sidebar away.
                                        pack_start = &gtk::ToggleButton {
                                            set_icon_name: "sidebar-show-symbolic",
                                            #[watch]
                                            set_visible: model.collapsed,
                                            #[watch]
                                            set_active: model.show_sidebar,
                                            connect_toggled[sender] => move |button| {
                                                sender.input(Msg::ToggleSidebar(button.is_active()));
                                            },
                                        },

                                        // Search lives in the header so it
                                        // reaches the whole catalog from any
                                        // destination, not just one tab.
                                        #[wrap(Some)]
                                        set_title_widget = &gtk::SearchEntry {
                                            set_placeholder_text: Some("Search 3,000+ apps"),
                                            set_width_request: 320,
                                            connect_search_changed[sender] => move |entry| {
                                                sender.input(Msg::SearchChanged(entry.text().to_string()));
                                            },
                                        },
                                    },

                                    // One shared progress strip rather than
                                    // per-row spinners: only one transaction
                                    // runs at a time, and it is the whole
                                    // window's state.
                                    add_top_bar = &gtk::Box {
                                        add_css_class: "transaction-bar",
                                        set_orientation: gtk::Orientation::Vertical,
                                        set_spacing: 6,
                                        #[watch]
                                        set_visible: model.busy,

                                        gtk::Label {
                                            #[watch]
                                            set_label: &model.status,
                                            set_xalign: 0.0,
                                            add_css_class: "caption",
                                        },
                                        gtk::ProgressBar {
                                            // No fraction means the backend
                                            // genuinely cannot estimate yet;
                                            // the bar sits at zero and the
                                            // label says what is happening
                                            // rather than inventing a number.
                                            #[watch]
                                            set_fraction: model.progress.unwrap_or(0.0),
                                        },
                                    },

                                    #[wrap(Some)]
                                    set_content = &gtk::Stack {
                                        set_vexpand: true,
                                        set_transition_type: gtk::StackTransitionType::Crossfade,

                                        add_named[Some("home")] = &gtk::Box {
                                            // Without these the page collapses
                                            // to its natural width and the
                                            // content sits in a narrow column
                                            // with the window empty beside it.
                                            set_hexpand: true,
                                            set_vexpand: true,
                                            #[local_ref]
                                            home_widget -> gtk::Stack { set_hexpand: true, set_vexpand: true },
                                        },

                                        add_named[Some("results")] = &gtk::Box {
                                            // Without these the page collapses
                                            // to its natural width and the
                                            // content sits in a narrow column
                                            // with the window empty beside it.
                                            set_hexpand: true,
                                            set_vexpand: true,
                                            #[local_ref]
                                            explore_widget -> gtk::Stack { set_hexpand: true, set_vexpand: true },
                                        },

                                        add_named[Some("installed")] = &gtk::Box {
                                            // Without these the page collapses
                                            // to its natural width and the
                                            // content sits in a narrow column
                                            // with the window empty beside it.
                                            set_hexpand: true,
                                            set_vexpand: true,
                                            #[local_ref]
                                            installed_widget -> gtk::Stack { set_hexpand: true, set_vexpand: true },
                                        },

                                        add_named[Some("updates")] = &gtk::Box {
                                            // Without these the page collapses
                                            // to its natural width and the
                                            // content sits in a narrow column
                                            // with the window empty beside it.
                                            set_hexpand: true,
                                            set_vexpand: true,
                                            #[local_ref]
                                            updates_widget -> gtk::Stack { set_hexpand: true, set_vexpand: true },
                                        },

                                        #[watch]
                                        set_visible_child_name: model.visible_page(),
                                    },
                                },
                            },
                        },
                    },
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        ui::style::load();

        let home = Home::builder()
            .launch(())
            .forward(sender.input_sender(), |output| match output {
                HomeOutput::OpenDetail(id) => Msg::OpenDetail(id),
                HomeOutput::SeeAll(category) => match category {
                    Some(category) => Msg::Navigate(Destination::Category(category)),
                    None => Msg::Navigate(Destination::Home),
                },
            });

        let explore = AppGrid::builder()
            .launch(())
            .forward(sender.input_sender(), |output| match output {
                AppGridOutput::OpenDetail(id) => Msg::OpenDetail(id),
            });

        let installed = AppList::builder()
            .launch(AppListInit {
                empty_title: "Nothing installed",
                empty_description: "Apps you install will appear here.",
                empty_icon: "computer-symbolic",
            })
            .forward(sender.input_sender(), forward_list);

        let updates = AppList::builder()
            .launch(AppListInit {
                empty_title: "Everything is up to date",
                empty_description: "You're running the latest version of every app.",
                empty_icon: "emblem-ok-symbolic",
            })
            .forward(sender.input_sender(), forward_list);

        let sidebar = Sidebar::builder()
            .launch(())
            .forward(sender.input_sender(), |output| match output {
                SidebarOutput::Navigate(destination) => Msg::Navigate(destination),
            });

        let model = Shell {
            registry: None,
            catalog: None,
            installed_ids: HashSet::new(),
            home,
            explore,
            installed,
            updates,
            detail: None,
            open_app: None,
            search: String::new(),
            destination: Destination::Home,
            format_filter: None,
            format_options: vec![None],
            sidebar,
            collapsed: false,
            show_sidebar: true,
            busy: false,
            progress: None,
            status: String::new(),
            nav: adw::NavigationView::new(),
            split: adw::NavigationSplitView::new(),
            format_box: gtk::Box::new(gtk::Orientation::Vertical, 2),
            toasts: adw::ToastOverlay::new(),
            window: root.clone(),
        };

        let home_widget = model.home.widget();
        let explore_widget = model.explore.widget();
        let installed_widget = model.installed.widget();
        let updates_widget = model.updates.widget();
        let sidebar_widget = model.sidebar.widget();
        let format_box = &model.format_box;
        let nav = &model.nav;
        let split = &model.split;
        let toasts = &model.toasts;
        let widgets = view_output!();

        // Collapse the sidebar into an overlay on narrow windows. Adwaita
        // drives this from a breakpoint rather than a resize handler so the
        // switch happens during layout, not a frame late.
        let breakpoint = adw::Breakpoint::new(
            adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                860.0,
                adw::LengthUnit::Sp,
            ),
        );
        breakpoint.add_setter(&model.split, "collapsed", Some(&true.to_value()));
        {
            let sender = sender.clone();
            breakpoint.connect_apply(move |_| sender.input(Msg::SetCollapsed(true)));
        }
        {
            let sender = sender.clone();
            breakpoint.connect_unapply(move |_| sender.input(Msg::SetCollapsed(false)));
        }
        root.add_breakpoint(breakpoint);

        // Probing backends and indexing the catalog both touch disk, so
        // neither runs before the first frame.
        runtime::spawn(startup(), {
            let sender = sender.clone();
            move |(registry, catalog, count)| {
                sender.input(Msg::Ready(Arc::new(registry), catalog, count));
            }
        });

        if let Some(destination) = init.request().and_then(screenshot::Request::destination) {
            // Driven through the sidebar rather than by setting state
            // directly, so the selected row matches the visible page.
            model.sidebar.emit(SidebarMsg::Select(destination));
        }
        if let Some(job) = init.into_request() {
            screenshot::schedule(&root, job);
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            Msg::Ready(registry, catalog, indexed) => {
                // Only offer formats that actually exist on this machine.
                // "All" is only meaningful alongside two or more of them,
                // which is also what reveals the control.
                self.format_options = std::iter::once(None)
                    .chain(registry.backend_ids().into_iter().map(Some))
                    .collect();

                let mut group: Option<gtk::CheckButton> = None;
                for (index, option) in self.format_options.iter().enumerate() {
                    let label = match option {
                        None => "All formats",
                        Some(backend) => backend.badge_label(),
                    };

                    let choice = gtk::CheckButton::with_label(label);
                    choice.set_active(index == 0);
                    // Radio behaviour: one scope at a time.
                    match &group {
                        Some(first) => choice.set_group(Some(first)),
                        None => group = Some(choice.clone()),
                    }

                    let sender = sender.clone();
                    choice.connect_toggled(move |button| {
                        if button.is_active() {
                            sender.input(Msg::FormatFilterChanged(index as u32));
                        }
                    });
                    self.format_box.append(&choice);
                }

                self.registry = Some(registry);
                self.catalog = catalog;

                if indexed > 0 {
                    sender.input(Msg::Toast(format!("Indexed {indexed} apps")));
                }
                if self.catalog.is_none() {
                    self.explore.emit(AppGridMsg::SetApps(Err(
                        "No app catalog found. Run `flatpak update --appstream` \
                         to download one."
                            .to_owned(),
                    )));
                }

                sender.input(Msg::ReloadInstalled);
            }

            // Installed state loads first because Explore cards depend on it
            // for their badges.
            Msg::ReloadInstalled => {
                let Some(registry) = self.registry.clone() else { return };
                self.installed.emit(AppListMsg::SetLoading);
                self.updates.emit(AppListMsg::SetLoading);

                let for_updates = Arc::clone(&registry);
                let updates_sender = sender.clone();

                runtime::spawn(async move { registry.installed().await }, {
                    let sender = sender.clone();
                    move |result| sender.input(Msg::InstalledLoaded(result))
                });

                runtime::spawn(async move { for_updates.updates().await }, move |result| {
                    updates_sender.input(Msg::UpdatesLoaded(result))
                });
            }

            Msg::InstalledLoaded(result) => {
                // Filtered here rather than in the query: installed apps come
                // from the backends, not the catalog, so the SQL filter never
                // sees them. A global scope has to mean global.
                let result = result.map(|apps| self.enrich(self.apply_format_filter(apps)));
                if let Ok(apps) = &result {
                    self.installed_ids = apps.iter().map(|a| a.app_ref.id.clone()).collect();

                    // An open detail page may have just had its app installed
                    // or removed; flip its button to match reality.
                    if let (Some(detail), Some(app_ref)) = (&self.detail, &self.open_app) {
                        detail.emit(DetailMsg::SetInstalled(
                            self.installed_ids.contains(&app_ref.id),
                        ));
                    }
                }
                self.installed.emit(AppListMsg::SetApps(result.map(|apps| {
                    apps.into_iter().map(|a| (a, RowAction::Remove)).collect()
                })));

                sender.input(Msg::ReloadExplore);
            }

            Msg::UpdatesLoaded(result) => {
                let result = result.map(|apps| self.enrich(self.apply_format_filter(apps)));
                if let Ok(apps) = &result {
                    self.sidebar.emit(SidebarMsg::SetUpdateCount(apps.len()));
                }
                self.updates.emit(AppListMsg::SetApps(result.map(|apps| {
                    apps.into_iter().map(|a| (a, RowAction::Update)).collect()
                })));
            }

            Msg::ReloadExplore => {
                let Some(catalog) = self.catalog.clone() else { return };

                // The shelf page and the results grid are fed by different
                // queries, so only the visible one is loaded.
                if self.browsing() {
                    self.home.emit(HomeMsg::SetLoading);

                    // Loaded together so the shelves can exclude whatever the
                    // hero is already showing. Both are microsecond SQLite
                    // queries; only the hero's artwork touches the network,
                    // and that happens separately.
                    runtime::spawn(load_home(Arc::clone(&catalog), self.filter()), {
                        let sender = sender.clone();
                        move |(featured, rows)| {
                            sender.input(Msg::FeaturedLoaded(featured));
                            sender.input(Msg::ShelvesLoaded(rows));
                        }
                    });
                    return;
                }

                let query = self.search.clone();
                let category = match &self.destination {
                    Destination::Category(category) => Some(category.clone()),
                    _ => None,
                };
                self.explore.emit(AppGridMsg::SetLoading);

                let filter = self.filter();
                runtime::spawn(async move { load_catalog(catalog, query, category, filter).await }, {
                    let sender = sender.clone();
                    move |result| sender.input(Msg::ExploreLoaded(result))
                });
            }

            Msg::FeaturedLoaded(apps) => {
                self.home.emit(HomeMsg::SetFeatured(apps));
            }

            Msg::ShelvesLoaded(rows) => {
                let installed = self.installed_ids.clone();
                let marked = rows
                    .into_iter()
                    .map(|apps| {
                        apps.into_iter()
                            .map(|app| {
                                let is_installed = installed.contains(&app.app_ref.id);
                                (app, is_installed)
                            })
                            .collect()
                    })
                    .collect();
                // Only worth showing once formats can differ; see `AppTile`.
                let show_format = self
                    .registry
                    .as_ref()
                    .is_some_and(|registry| registry.backend_count() > 1);
                self.home.emit(HomeMsg::SetShelves(marked, show_format));
            }

            Msg::ExploreLoaded(result) => {
                let installed = self.installed_ids.clone();
                self.explore.emit(AppGridMsg::SetApps(result.map(|apps| {
                    apps.into_iter()
                        .map(|app| {
                            let is_installed = installed.contains(&app.app_ref.id);
                            (app, is_installed)
                        })
                        .collect()
                })));
            }

            Msg::SearchChanged(query) => {
                if query == self.search {
                    return;
                }
                self.search = query;
                sender.input(Msg::ReloadExplore);
            }

            Msg::Navigate(destination) => {
                if destination == self.destination {
                    return;
                }
                self.destination = destination;

                // Navigating clears an active search: otherwise picking
                // "Games" while a search is running silently does nothing,
                // because search takes precedence over the destination.
                self.search.clear();

                // On a narrow window the sidebar is an overlay covering the
                // content, so choosing a destination should dismiss it.
                if self.collapsed {
                    self.show_sidebar = false;
                    self.split.set_show_content(true);
                }

                sender.input(Msg::ReloadExplore);
            }

            Msg::FormatFilterChanged(index) => {
                let selected = self
                    .format_options
                    .get(index as usize)
                    .copied()
                    .flatten();
                if selected == self.format_filter {
                    return;
                }
                self.format_filter = selected;
                // Every surface is scoped, so every surface reloads.
                sender.input(Msg::ReloadExplore);
                sender.input(Msg::ReloadInstalled);
            }

            Msg::SetCollapsed(collapsed) => {
                self.collapsed = collapsed;
            }

            Msg::ToggleSidebar(show) => {
                self.show_sidebar = show;
                self.split.set_show_content(!show);
            }

            Msg::OpenDetail(app_ref) => {
                let Some(app) = self.lookup(&app_ref) else {
                    sender.input(Msg::Toast("Couldn't load app details".into()));
                    return;
                };

                let installed = self.installed_ids.contains(&app_ref.id);
                let title = app.name.clone();

                let detail = Detail::builder()
                    .launch(DetailInit { app, installed })
                    .forward(sender.input_sender(), |output| match output {
                        DetailOutput::Action(action, id) => Msg::Act(action, id),
                        DetailOutput::NeedSandbox(id) => Msg::NeedSandbox(id),
                        DetailOutput::SetPermission(id, key, granted) => {
                            Msg::SetPermission(id, key, granted)
                        }
                    });

                let page = adw::NavigationPage::new(detail.widget(), &title);
                self.nav.push(&page);
                self.detail = Some(detail);
                self.open_app = Some(app_ref);
            }

            // Installing is the one action that grants an app access to your
            // machine, so it is gated behind an explicit look at what that
            // access is. Removing and updating need no such disclosure.
            Msg::Act(RowAction::Install, app_ref) => {
                if self.busy {
                    sender.input(Msg::Toast("Another operation is already running".into()));
                    return;
                }
                let Some(registry) = self.registry.clone() else { return };

                // Both answers are needed before the dialog can be honest:
                // what it will be able to do, and what it will actually pull
                // down. Resolved together so the dialog appears once.
                self.status = format!("Checking {}", app_ref.id);
                let for_plan = app_ref.clone();
                runtime::spawn(
                    async move {
                        let sandbox = registry.sandbox(&for_plan).await;
                        let plan = registry.install_plan(&for_plan).await;
                        (sandbox, plan)
                    },
                    {
                        let sender = sender.clone();
                        move |(sandbox, plan)| {
                            sender.input(Msg::ConfirmInstall(app_ref, sandbox, plan))
                        }
                    },
                );
            }

            Msg::Act(action, app_ref) => {
                sender.input(Msg::StartTransaction(action, app_ref));
            }

            Msg::ConfirmInstall(app_ref, sandbox, plan) => {
                let dialog = consent_dialog(&app_ref, sandbox.as_ref(), plan.as_ref());
                let sender = sender.clone();
                let app_ref = app_ref.clone();

                dialog.connect_response(None, move |_, response| {
                    if response == "install" {
                        sender.input(Msg::StartTransaction(RowAction::Install, app_ref.clone()));
                    }
                });
                dialog.present(Some(&self.window));
            }

            Msg::NeedSandbox(app_ref) => {
                let Some(registry) = self.registry.clone() else { return };
                runtime::spawn(async move { registry.sandbox(&app_ref).await }, {
                    let sender = sender.clone();
                    move |sandbox| sender.input(Msg::SandboxLoaded(sandbox))
                });
            }

            Msg::SandboxLoaded(sandbox) => {
                if let Some(detail) = &self.detail {
                    detail.emit(DetailMsg::SandboxLoaded(sandbox));
                }
            }

            Msg::SetPermission(app_ref, key, granted) => {
                let Some(registry) = self.registry.clone() else { return };
                runtime::spawn(
                    async move { registry.set_permission(&app_ref, &key, granted).await },
                    {
                        let sender = sender.clone();
                        move |result| {
                            if let Err(err) = result {
                                sender.input(Msg::Toast(format!("Couldn't save: {err}")));
                            }
                        }
                    },
                );
            }

            Msg::StartTransaction(action, app_ref) => {
                if self.busy {
                    sender.input(Msg::Toast("Another operation is already running".into()));
                    return;
                }
                let Some(registry) = self.registry.clone() else { return };

                self.busy = true;
                self.progress = None;
                self.status = format!("{} {}", action_verb(action), app_ref.id);
                if let Some(detail) = &self.detail {
                    detail.emit(DetailMsg::SetBusy(true));
                }

                let stream = match action {
                    RowAction::Install => registry.install(&app_ref),
                    RowAction::Remove => registry.remove(&app_ref),
                    RowAction::Update => registry.update(&app_ref),
                    RowAction::None => return,
                };

                // The stream ending is the success signal — backends report
                // failure as an item and success by simply finishing.
                let tick_sender = sender.clone();
                let done_sender = sender.clone();
                runtime::spawn_stream(
                    stream,
                    move |item| {
                        tick_sender.input(Msg::TransactionTick(item.map_err(|e| e.to_string())));
                    },
                    move || done_sender.input(Msg::TransactionFinished),
                );
            }

            Msg::TransactionTick(Ok(progress)) => {
                self.progress = progress.fraction;
                self.status = progress_label(&progress);
            }

            Msg::TransactionTick(Err(err)) => {
                self.busy = false;
                self.progress = None;
                if let Some(detail) = &self.detail {
                    detail.emit(DetailMsg::SetBusy(false));
                }
                sender.input(Msg::Toast(err));
            }

            Msg::TransactionFinished => {
                if !self.busy {
                    return;
                }
                self.busy = false;
                self.progress = None;
                if let Some(detail) = &self.detail {
                    detail.emit(DetailMsg::SetBusy(false));
                }
                sender.input(Msg::Toast("Done".into()));
                sender.input(Msg::ReloadInstalled);
            }

            Msg::Toast(text) => {
                self.toasts.add_toast(adw::Toast::new(&text));
            }
        }
    }
}

impl Shell {
    /// Apply the format scope to a list that didn't come from the catalog.
    fn apply_format_filter(&self, apps: Vec<App>) -> Vec<App> {
        let Some(backend) = self.format_filter else { return apps };
        apps.into_iter()
            .filter(|app| app.app_ref.backend == backend)
            .collect()
    }

    /// Current catalog filter, derived from the global scope control.
    fn filter(&self) -> Filter {
        Filter {
            backend: self.format_filter.map(|backend| backend.as_str().to_owned()),
        }
    }

    /// Which stack page the content area should show.
    ///
    /// An active search wins over the sidebar destination — typing is a
    /// stronger signal of intent than the last thing clicked.
    fn visible_page(&self) -> &'static str {
        if !self.search.trim().is_empty() {
            return "results";
        }
        match self.destination {
            Destination::Home => "home",
            Destination::Installed => "installed",
            Destination::Updates => "updates",
            Destination::Category(_) => "results",
        }
    }

    /// True when the home shelves are what's on screen.
    fn browsing(&self) -> bool {
        self.visible_page() == "home"
    }

    fn page_title(&self) -> String {
        if !self.search.trim().is_empty() {
            return format!("Results for \u{201c}{}\u{201d}", self.search.trim());
        }
        match &self.destination {
            Destination::Home => "Home".to_owned(),
            Destination::Installed => "Installed".to_owned(),
            Destination::Updates => "Updates".to_owned(),
            Destination::Category(category) => ui::sidebar::category_label(category).to_owned(),
        }
    }

    /// Fill in what the backend can't tell us from the catalog.
    ///
    /// Flatpak knows an app's id, version and sandbox but has no icon or
    /// description. The theme lookup that used to cover for this only works if
    /// Flatpak's export directory is in `XDG_DATA_DIRS`, which it isn't when
    /// running from `cargo run` — so installed apps rendered as generic gears.
    /// The catalog has a real icon path for nearly every app, so it is the
    /// better source. Backend fields win where both exist, since those
    /// describe what is actually on disk.
    fn enrich(&self, apps: Vec<App>) -> Vec<App> {
        let Some(catalog) = &self.catalog else { return apps };
        let Ok(guard) = catalog.lock() else { return apps };

        apps.into_iter()
            .map(|mut app| {
                let Ok(Some(entry)) = guard.get(&app.app_ref.id) else { return app };

                if app.icon.is_none() {
                    app.icon = entry.icon;
                }
                if app.summary.is_none() {
                    app.summary = entry.summary;
                }
                if app.developer.is_none() {
                    app.developer = entry.developer;
                }
                // An id-shaped name means the backend had no appdata for it.
                if app.name == app.app_ref.id {
                    app.name = entry.name;
                }
                app
            })
            .collect()
    }

    /// Find an app by ref among whatever is currently loaded.
    ///
    /// The catalog is authoritative because it carries screenshots and
    /// changelogs, which the backend doesn't expose.
    fn lookup(&self, app_ref: &AppRef) -> Option<App> {
        let catalog = self.catalog.as_ref()?;
        let guard = catalog.lock().ok()?;
        guard.get(&app_ref.id).ok().flatten()
    }
}

/// The pre-install disclosure.
///
/// This is the feature AppsScope exists for. Every other software centre
/// installs first and leaves you to discover afterwards — via a separate tool —
/// that the app you just installed can read your entire home directory. Here
/// that is stated before anything is downloaded, in words rather than flags.
///
/// The dialog is deliberately *not* alarmist for well-behaved apps: a properly
/// confined app gets a short, reassuring dialog, so that the loud version means
/// something when it appears.
fn consent_dialog(
    app_ref: &AppRef,
    sandbox: Option<&Sandbox>,
    plan: Option<&InstallPlan>,
) -> adw::AlertDialog {
    let risk = sandbox.map(Sandbox::risk);

    let heading = match risk {
        Some(Risk::Critical) => "This app is not sandboxed",
        Some(Risk::Caution) => "Before you install",
        Some(Risk::Safe) => "Install this app?",
        None => "Permissions unknown",
    };

    let body = match (sandbox, risk) {
        (Some(_), Some(Risk::Critical)) => format!(
            "{} can access files and services outside its sandbox. Review what it will \
             be able to do:",
            app_ref.id
        ),
        (Some(_), Some(Risk::Caution)) => {
            format!("{} will be able to do the following:", app_ref.id)
        }
        (Some(_), Some(Risk::Safe)) => format!(
            "{} runs fully confined and cannot reach your personal files.",
            app_ref.id
        ),
        _ => format!(
            "AppsScope could not read the permissions for {}. It may be able to access \
             files outside its sandbox.",
            app_ref.id
        ),
    };

    let dialog = adw::AlertDialog::new(Some(heading), Some(&body));

    let extra = gtk::Box::new(gtk::Orientation::Vertical, 14);
    extra.set_margin_top(6);

    // What is being installed, in what format, from where, and how big — the
    // questions a store should answer before asking you to commit.
    if let Some(plan) = plan {
        extra.append(&plan_summary(plan));
    }

    // List the permissions that actually warrant attention. Showing all of
    // them — including "can draw its window" — would bury the signal.
    if let Some(sandbox) = sandbox {
        let notable: Vec<_> = sandbox
            .sorted()
            .into_iter()
            .filter(|p| p.risk >= Risk::Caution)
            .collect();

        if !notable.is_empty() {
            let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
            list.set_margin_top(8);

            for permission in notable.iter().take(6) {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.add_css_class("consent-permission");

                let icon = gtk::Image::from_icon_name(permission.risk.icon());
                icon.add_css_class(permission.risk.css_class());
                icon.set_valign(gtk::Align::Start);
                row.append(&icon);

                let label = gtk::Label::new(Some(&permission.summary));
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_hexpand(true);
                row.append(&label);

                list.append(&row);
            }

            if notable.len() > 6 {
                let more = gtk::Label::new(Some(&format!(
                    "and {} more — see the app page for the full list",
                    notable.len() - 6
                )));
                more.add_css_class("dim-label");
                more.add_css_class("caption");
                more.set_xalign(0.0);
                list.append(&more);
            }

            extra.append(&list);
        }
    }

    if extra.first_child().is_some() {
        dialog.set_extra_child(Some(&extra));
    }

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("install", "Install");
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    // A risky install is styled as destructive rather than suggested: the
    // button should not invite a reflexive click.
    dialog.set_response_appearance(
        "install",
        match risk {
            Some(Risk::Critical) | None => adw::ResponseAppearance::Destructive,
            _ => adw::ResponseAppearance::Suggested,
        },
    );

    dialog
}

/// The "what am I installing" block: format, source, sizes, and every extra
/// package that comes with it.
fn plan_summary(plan: &InstallPlan) -> gtk::Widget {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);

    let provenance = gtk::Label::new(Some(&plan.provenance()));
    provenance.add_css_class("plan-provenance");
    provenance.set_xalign(0.0);
    outer.append(&provenance);

    let sizes = gtk::Label::new(Some(&format!(
        "{} download · {} on disk",
        format_size(plan.download_size()),
        format_size(plan.installed_size())
    )));
    sizes.add_css_class("dim-label");
    sizes.set_xalign(0.0);
    outer.append(&sizes);

    // The part other stores hide: a small app can drag in a runtime many times
    // its size. Only shown when there is something extra, and only counting
    // what is genuinely missing from this machine.
    if plan.has_dependencies() {
        let heading = gtk::Label::new(Some("Also downloads"));
        heading.add_css_class("plan-heading");
        heading.set_xalign(0.0);
        heading.set_margin_top(4);
        outer.append(&heading);

        for item in plan.dependencies().take(4) {
            let row = gtk::Label::new(Some(&format!(
                "{} — {}",
                item.name,
                format_size(item.download_size)
            )));
            row.add_css_class("dim-label");
            row.add_css_class("caption");
            row.set_xalign(0.0);
            outer.append(&row);
        }

        let extra = plan.dependencies().count().saturating_sub(4);
        if extra > 0 {
            let more = gtk::Label::new(Some(&format!("and {extra} more")));
            more.add_css_class("dim-label");
            more.add_css_class("caption");
            more.set_xalign(0.0);
            outer.append(&more);
        }
    }

    outer.upcast()
}

fn forward_list(output: AppListOutput) -> Msg {
    match output {
        AppListOutput::OpenDetail(id) => Msg::OpenDetail(id),
        AppListOutput::Action(action, id) => Msg::Act(action, id),
    }
}

fn action_verb(action: RowAction) -> &'static str {
    match action {
        RowAction::Install => "Installing",
        RowAction::Remove => "Removing",
        RowAction::Update => "Updating",
        RowAction::None => "",
    }
}

fn progress_label(progress: &Progress) -> String {
    let phase = match progress.phase {
        Phase::Resolving => "Preparing",
        Phase::Downloading => "Downloading",
        Phase::Installing => "Installing",
        Phase::Removing => "Removing",
        Phase::Finalizing => "Finishing up",
    };

    match (progress.bytes_done, progress.bytes_total) {
        (Some(done), Some(total)) if total > 0 => {
            format!("{phase} — {} of {}", format_mb(done), format_mb(total))
        }
        _ => phase.to_owned(),
    }
}

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// Probe backends and open/index the catalog.
async fn startup() -> (Registry, Option<Arc<Mutex<Catalog>>>, usize) {
    let registry = Registry::detect().await;

    // Parsing a 48 MB XML catalog is CPU-bound and blocking, so it goes to a
    // blocking thread rather than stalling a runtime worker.
    let catalog = tokio::task::spawn_blocking(|| {
        let mut catalog = Catalog::open_default()?;
        let indexed = catalog.sync()?;
        Ok::<_, appsscope_catalog::Error>((catalog, indexed))
    })
    .await;

    match catalog {
        Ok(Ok((catalog, indexed))) => (registry, Some(Arc::new(Mutex::new(catalog))), indexed),
        Ok(Err(err)) => {
            tracing::error!(%err, "catalog unavailable");
            (registry, None, 0)
        }
        Err(err) => {
            tracing::error!(%err, "catalog task panicked");
            (registry, None, 0)
        }
    }
}

/// Load the hero and every shelf in one blocking pass.
///
/// One task rather than one per shelf: these are microsecond SQLite queries
/// sharing one lock, so fanning out would only add contention. Doing it
/// together also lets the shelves skip whatever is already in the hero.
async fn load_home(
    catalog: Arc<Mutex<Catalog>>,
    filter: Filter,
) -> (Vec<App>, Vec<Vec<App>>) {
    tokio::task::spawn_blocking(move || {
        let Ok(guard) = catalog.lock() else { return (Vec::new(), Vec::new()) };

        let featured = guard.featured(FEATURED_LIMIT).unwrap_or_default();
        let mut seen: HashSet<String> =
            featured.iter().map(|app| app.app_ref.id.clone()).collect();

        let rows = SHELVES
            .iter()
            .map(|(_, category)| {
                // Over-fetch, then drop anything already on the page, so a
                // shelf still fills its row after deduplication.
                let result = match category {
                    Some(category) => guard.by_category(category, SHELF_LIMIT * 2, &filter),
                    // The bespoke shelf; see `SHELVES` for why it has no category.
                    None => guard.recently_updated(SHELF_LIMIT * 2, &filter),
                };

                let apps: Vec<App> = result
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|app| !seen.contains(&app.app_ref.id))
                    .take(SHELF_LIMIT)
                    .collect();

                // Each shelf claims its apps, so the same app never appears
                // twice down the page.
                seen.extend(apps.iter().map(|app| app.app_ref.id.clone()));
                apps
            })
            .collect();

        (featured, rows)
    })
    .await
    .unwrap_or_default()
}

/// Search, filter by category, or browse — in that order of precedence.
async fn load_catalog(
    catalog: Arc<Mutex<Catalog>>,
    query: String,
    category: Option<String>,
    filter: Filter,
) -> Result<Vec<App>, String> {
    tokio::task::spawn_blocking(move || {
        let guard = catalog.lock().map_err(|_| "catalog lock poisoned".to_owned())?;

        let result = match (query.trim(), category.as_deref()) {
            ("", None) => guard.browse(BROWSE_LIMIT, &filter),
            ("", Some(category)) => guard.by_category(category, BROWSE_LIMIT, &filter),
            (query, _) => guard.search(query, SEARCH_LIMIT, &filter),
        };

        result.map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "appsscope=info".into()),
        )
        .init();

    let job = screenshot::Job::from_env();

    // A capture run must not hand off to an already-running instance, or it
    // would photograph nothing and exit successfully.
    let app = if job.is_some() {
        RelmApp::new(&format!("{APP_ID}.Screenshot"))
    } else {
        RelmApp::new(APP_ID)
    };
    app.run::<Shell>(job);
}
