//! The featured banner at the top of Explore.
//!
//! A swipeable carousel of large cards, each using the app's own screenshot as
//! full-bleed artwork with the icon, name and summary laid over a gradient
//! scrim. This is the single biggest visual difference between a storefront
//! and a package list: without artwork, a home page is a directory.
//!
//! Cards are laid out immediately with a placeholder background and filled in
//! as images arrive, so the carousel never resizes under the reader.

use appsscope_core::{App, AppRef};
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use crate::runtime;

const HERO_HEIGHT: i32 = 320;
/// Height of the solid information bar under the artwork.
const HERO_BAR_HEIGHT: i32 = 92;

pub struct Hero {
    /// Background pictures by carousel position, so images can be dropped in
    /// once they finish downloading.
    slides: Vec<gtk::Picture>,
    carousel: adw::Carousel,
    empty: bool,
}

#[derive(Debug)]
pub enum HeroMsg {
    SetApps(Vec<App>),
    ImageReady(usize, std::path::PathBuf),
    Activated(AppRef),
}

#[derive(Debug)]
pub enum HeroOutput {
    OpenDetail(AppRef),
}

#[relm4::component(pub)]
impl SimpleComponent for Hero {
    type Init = ();
    type Input = HeroMsg;
    type Output = HeroOutput;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
            #[watch]
            set_visible: !model.empty,

            #[local_ref]
            carousel -> adw::Carousel {
                set_height_request: HERO_HEIGHT,
                set_spacing: 14,
                set_margin_start: 16,
                set_margin_end: 16,
                set_margin_top: 10,
            },

            adw::CarouselIndicatorDots {
                set_carousel: Some(carousel),
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = Hero {
            slides: Vec::new(),
            carousel: adw::Carousel::new(),
            empty: true,
        };
        let carousel = &model.carousel;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            HeroMsg::SetApps(apps) => {
                while let Some(child) = self.carousel.first_child() {
                    self.carousel.remove(&child);
                }
                self.slides.clear();

                for (index, app) in apps.iter().enumerate() {
                    let (card, picture) = build_card(app, &sender);
                    self.carousel.append(&card);
                    self.slides.push(picture);

                    if let Some(shot) = app.screenshots.first() {
                        let url = shot.url.clone();
                        let sender = sender.clone();
                        runtime::spawn(crate::images::fetch(url), move |path| {
                            if let Some(path) = path {
                                sender.input(HeroMsg::ImageReady(index, path));
                            }
                        });
                    }
                }

                self.empty = apps.is_empty();
            }

            HeroMsg::ImageReady(index, path) => {
                let Some(picture) = self.slides.get(index) else { return };
                match gdk::Texture::from_filename(&path) {
                    Ok(texture) => picture.set_paintable(Some(&texture)),
                    Err(err) => tracing::debug!(?path, %err, "hero image decode failed"),
                }
            }

            HeroMsg::Activated(id) => {
                let _ = sender.output(HeroOutput::OpenDetail(id));
            }
        }
    }
}

/// Build one hero card, returning it along with its artwork picture.
///
/// Artwork on top, a solid information bar beneath — rather than text laid
/// over the image behind a gradient scrim.
///
/// The scrim version was the obvious design and it did not survive contact
/// with real data: these are arbitrary app screenshots, not editorial artwork
/// commissioned to have a quiet lower third. Many contain their own UI text
/// exactly where the caption sits, so no gradient is strong enough to
/// guarantee legibility without dimming the artwork into mud. A solid bar is
/// readable against every screenshot, including ones nobody has seen yet.
fn build_card(app: &App, sender: &ComponentSender<Hero>) -> (gtk::Widget, gtk::Picture) {
    // The whole card is one button: a nested "Install" button inside a
    // clickable card is ambiguous to both mouse and screen reader, so the
    // call to action is styled text and the card itself is the target.
    let button = gtk::Button::new();
    button.add_css_class("hero-card");
    button.add_css_class("flat");
    button.set_overflow(gtk::Overflow::Hidden);
    button.set_hexpand(true);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.add_css_class("hero-art");
    column.append(&picture);

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    bar.add_css_class("hero-bar");
    bar.set_height_request(HERO_BAR_HEIGHT);

    bar.append(&super::icon::tile(app, 60));

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_halign(gtk::Align::Start);
    text.set_hexpand(true);

    let name = gtk::Label::new(Some(&app.name));
    name.add_css_class("hero-name");
    name.set_xalign(0.0);
    name.set_halign(gtk::Align::Start);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&name);

    if let Some(summary) = &app.summary {
        let summary_label = gtk::Label::new(Some(summary));
        summary_label.add_css_class("hero-summary");
        summary_label.set_xalign(0.0);
        summary_label.set_halign(gtk::Align::Start);
        summary_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary_label.set_max_width_chars(60);
        text.append(&summary_label);
    }

    bar.append(&text);

    let cta = gtk::Label::new(Some("View"));
    cta.add_css_class("hero-cta");
    cta.set_valign(gtk::Align::Center);
    bar.append(&cta);

    column.append(&bar);
    button.set_child(Some(&column));

    let id = app.app_ref.clone();
    let sender = sender.clone();
    button.connect_clicked(move |_| {
        sender.input(HeroMsg::Activated(id.clone()));
    });

    (button.upcast(), picture)
}

use relm4::adw;
