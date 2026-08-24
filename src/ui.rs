//! The launcher surface: window, search field, result list, footer.
//!
//! This module renders what `search` produced and reports what `actions`
//! returned. It does not decide either.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::accessible::{Property, State};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};

use crate::actions::{self, Outcome};
use crate::search::{self, Item};

const WIDTH: i32 = 720;
const MAX_RESULT_HEIGHT: i32 = 392;

pub fn load_styles() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

pub struct Launcher {
    window: gtk::Window,
    entry: gtk::Entry,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    empty: gtk::Box,
    empty_title: gtk::Label,
    hints: gtk::Box,
    status: gtk::Box,
    status_icon: gtk::Image,
    status_label: gtk::Label,

    items: RefCell<Vec<Item>>,
    /// Held so the installed-applications signal stays connected.
    _apps: gio::AppInfoMonitor,
    hits: RefCell<Vec<usize>>,
    rows: RefCell<Vec<gtk::ListBoxRow>>,
    selected: Cell<usize>,
}

impl Launcher {
    pub fn build(app: &gtk::Application) -> Rc<Self> {
        let entry = gtk::Entry::builder()
            .placeholder_text("Search Scene…")
            .hexpand(true)
            .has_frame(false)
            .css_classes(["query"])
            .build();
        entry.update_property(&[Property::Label("Search Scene")]);

        let search_row = row_box(15);
        search_row.add_css_class("search");
        search_row.append(&icon("edit-find-symbolic", "search-icon"));
        search_row.append(&entry);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["results"])
            .build();

        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(MAX_RESULT_HEIGHT)
            .css_classes(["results"])
            .build();

        let empty_title = gtk::Label::new(None);
        empty_title.add_css_class("empty-title");
        let empty_hint = gtk::Label::new(Some(
            "Try a different word, or clear the search with Escape.",
        ));
        empty_hint.add_css_class("empty-hint");
        let empty = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .halign(gtk::Align::Center)
            .css_classes(["empty"])
            .visible(false)
            .build();
        empty.append(&icon("edit-find-symbolic", "empty-icon"));
        empty.append(&empty_title);
        empty.append(&empty_hint);

        let brand = gtk::Label::new(Some("Scene"));
        brand.add_css_class("brand");

        let status_icon = gtk::Image::new();
        let status_label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .max_width_chars(56)
            .build();
        let status = row_box(0);
        status.set_css_classes(&["status"]);
        status.set_hexpand(true);
        status.set_visible(false);
        status.set_margin_start(12);
        status.append(&status_icon);
        status.append(&status_label);

        let hints = row_box(0);
        hints.add_css_class("hints");
        hints.set_halign(gtk::Align::End);
        hints.set_hexpand(true);
        for (keys, what) in [
            (vec!["↑", "↓"], "Navigate"),
            (vec!["↵"], "Open"),
            (vec!["esc"], "Close"),
        ] {
            for k in keys {
                hints.append(&kbd(k));
            }
            let label = gtk::Label::new(Some(what));
            label.add_css_class("hint");
            hints.append(&label);
        }

        let footer = row_box(0);
        footer.add_css_class("footer");
        footer.append(&brand);
        footer.append(&status);
        footer.append(&hints);

        let rule = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        rule.add_css_class("rule");

        let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
        surface.add_css_class("surface");
        surface.append(&search_row);
        surface.append(&rule);
        surface.append(&scroller);
        surface.append(&empty);
        surface.append(&footer);
        surface.set_size_request(WIDTH, -1);
        apply_preferences(&surface);

        let window = gtk::Window::builder()
            .application(app)
            .title("Scene")
            .decorated(false)
            .resizable(false)
            .hide_on_close(true)
            .css_classes(["scene"])
            .child(&surface)
            .build();

        let launcher = Rc::new(Self {
            window,
            entry,
            list,
            scroller,
            empty,
            empty_title,
            hints,
            status,
            status_icon,
            status_label,
            items: RefCell::new(search::index()),
            _apps: gio::AppInfoMonitor::get(),
            hits: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(0),
        });

        launcher.connect_signals();
        launcher.refresh();
        launcher
    }

    fn connect_signals(self: &Rc<Self>) {
        // Scene is meant to stay resident, so an application installed while
        // it is running has to show up without a restart.
        let this = Rc::downgrade(self);
        self._apps.connect_changed(move |_| {
            if let Some(l) = this.upgrade() {
                *l.items.borrow_mut() = search::index();
                l.refresh();
            }
        });

        let this = Rc::downgrade(self);
        self.entry.connect_changed(move |_| {
            if let Some(l) = this.upgrade() {
                l.clear_status();
                l.refresh();
            }
        });

        let this = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, row| {
            let Some(l) = this.upgrade() else { return };
            if let Some(position) = l.rows.borrow().iter().position(|r| r == row) {
                l.selected.set(position);
                l.apply_selection();
            }
            l.activate_selected();
        });

        // Capture phase: arrows and Enter are ours before the entry sees them,
        // everything else falls through to the search field.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(l) = this.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Down => l.move_selection(1),
                gdk::Key::Up => l.move_selection(-1),
                gdk::Key::Return | gdk::Key::KP_Enter => l.activate_selected(),
                gdk::Key::Escape => l.dismiss(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.window.add_controller(keys);
    }

    /// Show the launcher in a known-good state, however many times it is called.
    pub fn present(&self) {
        self.entry.set_text("");
        self.clear_status();
        self.refresh();
        self.window.present();
        self.entry.grab_focus();
    }

    fn refresh(&self) {
        let query = self.entry.text().to_string();
        let items = self.items.borrow();
        let hits = search::search(&query, &items);

        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::with_capacity(hits.len());
        let mut group = None;

        for &index in &hits {
            let item = &items[index];
            if group != Some(item.kind) {
                group = Some(item.kind);
                self.list.append(&section_row(item.kind.heading()));
            }
            let row = result_row(item);
            self.list.append(&row);
            rows.push(row);
        }

        let found = !hits.is_empty();
        self.scroller.set_visible(found);
        self.empty.set_visible(!found);
        if !found {
            self.empty_title
                .set_text(&format!("No results for “{}”", query.trim()));
        }

        *self.hits.borrow_mut() = hits;
        *self.rows.borrow_mut() = rows;
        self.selected.set(0);
        self.apply_selection();
        self.scroller.vadjustment().set_value(0.0);
    }

    fn move_selection(&self, delta: i32) {
        let count = self.rows.borrow().len();
        if count == 0 {
            return;
        }
        let current = self.selected.get() as i32;
        let next = (current + delta).rem_euclid(count as i32) as usize;
        self.selected.set(next);
        self.apply_selection();
        self.scroll_to_selection();
    }

    fn apply_selection(&self) {
        let selected = self.selected.get();
        for (position, row) in self.rows.borrow().iter().enumerate() {
            let is_selected = position == selected;
            if is_selected {
                row.add_css_class("selected");
            } else {
                row.remove_css_class("selected");
            }
            row.update_state(&[State::Selected(Some(is_selected))]);
        }
    }

    fn scroll_to_selection(&self) {
        let rows = self.rows.borrow();
        let Some(row) = rows.get(self.selected.get()) else {
            return;
        };
        let Some(bounds) = row.compute_bounds(&self.list) else {
            return;
        };

        let adjustment = self.scroller.vadjustment();
        let (top, bottom) = (bounds.y() as f64, (bounds.y() + bounds.height()) as f64);
        let view = adjustment.page_size();
        if top < adjustment.value() {
            adjustment.set_value(top - 6.0);
        } else if bottom > adjustment.value() + view {
            adjustment.set_value(bottom - view + 6.0);
        }
    }

    fn activate_selected(&self) {
        let index = {
            let hits = self.hits.borrow();
            match hits.get(self.selected.get()) {
                Some(&index) => index,
                None => return,
            }
        };

        let outcome = actions::execute(&self.items.borrow()[index].action);
        if outcome == Outcome::Quit {
            if let Some(app) = self.window.application() {
                app.quit();
            }
        } else if outcome.should_dismiss() {
            self.window.set_visible(false);
        } else {
            self.show_status(&outcome);
        }
    }

    /// Escape cancels the query first, and only then closes the launcher.
    fn dismiss(&self) {
        if self.entry.text().is_empty() {
            self.window.set_visible(false);
        } else {
            self.entry.set_text("");
        }
    }

    fn show_status(&self, outcome: &Outcome) {
        let text = format!("{} — {}", outcome.prefix(), outcome.message());
        self.status_icon.set_icon_name(Some(outcome.icon()));
        self.status_label.set_text(&text);
        self.status_label.set_tooltip_text(Some(&text));
        self.status.set_css_classes(&["status", outcome.tone()]);
        self.status
            .update_property(&[Property::Label(text.as_str())]);
        self.status.set_visible(true);
        self.hints.set_visible(false);
    }

    fn clear_status(&self) {
        self.status.set_visible(false);
        self.hints.set_visible(true);
    }
}

/// Preferences that are product requirements rather than decoration.
fn apply_preferences(surface: &gtk::Box) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    if !settings.is_gtk_enable_animations() {
        surface.add_css_class("no-motion");
    }
    if settings
        .gtk_theme_name()
        .is_some_and(|t| t.contains("HighContrast"))
    {
        surface.add_css_class("contrast");
    }
}

fn row_box(spacing: i32) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, spacing);
    row.set_valign(gtk::Align::Center);
    row
}

fn icon(name: &str, class: &str) -> gtk::Image {
    let image = gtk::Image::from_icon_name(name);
    image.add_css_class(class);
    image
}

fn kbd(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("kbd");
    label.set_valign(gtk::Align::Center);
    label
}

fn section_row(heading: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::new(Some(heading));
    label.set_xalign(0.0);
    label.add_css_class("section");

    // GTK CSS has no letter-spacing, and these headings need the air.
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrInt::new_letter_spacing(pango::SCALE));
    label.set_attributes(Some(&attributes));

    let row = gtk::ListBoxRow::builder()
        .child(&label)
        .selectable(false)
        .activatable(false)
        .focusable(false)
        .build();
    row.update_property(&[Property::Label(heading)]);
    row
}

fn result_row(item: &Item) -> gtk::ListBoxRow {
    let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tile.set_css_classes(&["tile", item.kind.slug()]);
    tile.set_halign(gtk::Align::Center);
    tile.set_valign(gtk::Align::Center);
    tile.append(&resolved_icon(item));

    let title = gtk::Label::new(Some(&item.title));
    title.set_xalign(0.0);
    title.set_ellipsize(pango::EllipsizeMode::End);
    title.set_max_width_chars(52);
    title.add_css_class("title");

    let subtitle = gtk::Label::new(Some(&item.subtitle));
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(pango::EllipsizeMode::End);
    subtitle.set_max_width_chars(52);
    subtitle.add_css_class("subtitle");

    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&title);
    text.append(&subtitle);

    let tag = gtk::Label::new(Some(item.tag()));
    tag.add_css_class("tag");

    // The selected row is marked by a bar, weight and this glyph, so the
    // selection never depends on colour alone.
    let enter = gtk::Label::new(Some("↵"));
    enter.add_css_class("enter");

    let content = row_box(0);
    content.append(&tile);
    content.append(&text);
    content.append(&tag);
    content.append(&enter);

    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .selectable(false)
        .activatable(true)
        .focusable(false)
        .css_classes(["result"])
        .build();
    // The stable id names the row, so tests and tooling can address it.
    row.set_widget_name(&item.id);

    let described = format!("{}. {}. {}", item.title, item.tag(), item.subtitle);
    row.update_property(&[Property::Label(described.as_str())]);
    row
}

/// Prefer the item's own icon, fall back to one that always exists.
fn resolved_icon(item: &Item) -> gtk::Image {
    match item.icon.as_ref().filter(|icon| is_available(icon)) {
        Some(icon) => gtk::Image::from_gicon(icon),
        None => gtk::Image::from_icon_name(item.kind.fallback_icon()),
    }
}

/// A themed icon is only usable if the current theme actually has it —
/// otherwise GTK renders a broken-image placeholder. Icons that are a file on
/// disk, as bundled applications often ship, are left to GTK to load.
fn is_available(icon: &gio::Icon) -> bool {
    let Some(themed) = icon.downcast_ref::<gio::ThemedIcon>() else {
        return true;
    };
    gdk::Display::default()
        .map(|display| gtk::IconTheme::for_display(&display))
        .is_some_and(|theme| themed.names().iter().any(|name| theme.has_icon(name)))
}
