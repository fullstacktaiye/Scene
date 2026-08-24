//! The launcher surface: window, search field, result list, footer.
//!
//! This module renders what `search` produced and reports what `actions`
//! returned. It does not decide either.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::accessible::{Property, State};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};

use crate::actions::{self, Action, Outcome, RunningAction, StartedAction};
use crate::integrations;
use crate::search::{self, History, Item};

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
    /// The providers' answers to the current query. They exist only while the
    /// query asks for them, so they are rebuilt on every refresh rather than
    /// indexed.
    answers: RefCell<Vec<Item>>,
    /// Held so the installed-applications signal stays connected.
    _apps: gio::AppInfoMonitor,
    hits: RefCell<Vec<usize>>,
    rows: RefCell<Vec<gtk::ListBoxRow>>,
    selected: Cell<usize>,
    /// The action selected for confirmation is frozen here, with the result id
    /// it came from. Query changes and selection movement cannot substitute a
    /// different mutation on Enter.
    pending_confirmation: RefCell<Option<(String, Action)>>,
    running: RefCell<Option<RunningAction>>,
    /// What the user has chosen before, which adjusts ranking within a group.
    /// `search` decides how much it counts for; this module only records it.
    history: RefCell<History>,
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
            answers: RefCell::new(Vec::new()),
            _apps: gio::AppInfoMonitor::get(),
            hits: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            pending_confirmation: RefCell::new(None),
            running: RefCell::new(None),
            history: RefCell::new(History::load()),
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
            l.activate_selected(false);
        });

        // Capture phase: arrows and Enter are ours before the entry sees them,
        // everything else falls through to the search field.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| match this.upgrade() {
            Some(launcher) => launcher.key(key),
            None => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);
    }

    /// The whole keyboard contract, in one place. The controller above only
    /// delivers the key; everything the launcher does with one is here, which
    /// is also what the smoke harness drives.
    fn key(self: &Rc<Self>, key: gdk::Key) -> glib::Propagation {
        match key {
            gdk::Key::Down => self.move_selection(1),
            gdk::Key::Up => self.move_selection(-1),
            gdk::Key::Return | gdk::Key::KP_Enter => self.activate_selected(true),
            gdk::Key::Escape => self.dismiss(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// Rank against a stated set of items instead of the machine's own index,
    /// so the smoke harness drives the real widget tree without depending on
    /// what happens to be installed.
    #[cfg(test)]
    fn replace_items(&self, items: Vec<Item>) {
        *self.items.borrow_mut() = items;
        self.refresh();
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
        *self.answers.borrow_mut() = integrations::answers(&query);

        let answers = self.answers.borrow();
        let items = self.items.borrow();
        // One ranked list over both sources, so a query answer is ordered by
        // the same rules as everything else rather than pinned by the UI.
        let visible: Vec<&Item> = answers.iter().chain(items.iter()).collect();
        let hits = search::search(&query, &visible, &self.history.borrow());

        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::with_capacity(hits.len());
        let mut group = None;
        // A group is only ever trimmed in the resting state, so that is the
        // only time a heading has a count to report.
        let resting = query.trim().is_empty();

        for &index in &hits {
            let item = visible[index];
            if group != Some(item.kind) {
                group = Some(item.kind);
                let trimmed = resting
                    .then(|| withheld(item.kind, &hits, &visible))
                    .flatten();
                self.list.append(&section_row(item.kind, trimmed));
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

        drop(items);
        drop(answers);
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

    fn activate_selected(self: &Rc<Self>, confirm_pending: bool) {
        if self.running.borrow().is_some() {
            return;
        }
        if self.pending_confirmation.borrow().is_some() {
            // A result-row click may have changed the selected row, but it is
            // never an unambiguous confirmation for a previously displayed
            // mutation. Only Enter accepts the frozen confirmation payload.
            if confirm_pending {
                let (id, action) = self
                    .pending_confirmation
                    .borrow_mut()
                    .take()
                    .expect("checked pending confirmation");
                self.start_action(&id, &action, true);
            }
            return;
        }
        let index = {
            let hits = self.hits.borrow();
            match hits.get(self.selected.get()) {
                Some(&index) => index,
                None => return,
            }
        };

        let Some((id, action)) = self.chosen(index) else {
            return;
        };
        if actions::requires_confirmation(&action) {
            let Action::Process { action: process } = &action else {
                unreachable!()
            };
            let text = actions::confirmation_text(process);
            *self.pending_confirmation.borrow_mut() = Some((id, action));
            self.show_status(&Outcome::AwaitingConfirmation(text));
            return;
        }
        self.start_action(&id, &action, false);
    }

    /// Resolve a ranked position back to its item's id and action. Answers are
    /// ranked ahead of the index, so they occupy the first positions of the
    /// same list.
    fn chosen(&self, index: usize) -> Option<(String, Action)> {
        let answers = self.answers.borrow();
        if let Some(item) = answers.get(index) {
            return Some((item.id.clone(), item.action.clone()));
        }
        let index = index - answers.len();
        self.items
            .borrow()
            .get(index)
            .map(|item| (item.id.clone(), item.action.clone()))
    }

    fn start_action(self: &Rc<Self>, id: &str, action: &Action, confirmed: bool) {
        // Recorded when the action actually starts, so a confirmation the user
        // backed out of is not counted as a use.
        self.history.borrow_mut().record(id);

        let started = if confirmed {
            actions::start_confirmed(action)
        } else {
            actions::start(action)
        };
        match started {
            StartedAction::Running(running) => {
                let title = match action {
                    Action::Process { action } => action.title.clone(),
                    Action::Open { target } => format!("Opening {target}"),
                    _ => "Action".into(),
                };
                let waiting = if running.is_cancellable() {
                    format!("{title} is running. Press Escape to cancel.")
                } else {
                    format!("{title}…")
                };
                *self.running.borrow_mut() = Some(running);
                self.show_status(&Outcome::Pending(waiting));
                self.watch_running();
            }
            StartedAction::Immediate(outcome) => self.finish_outcome(outcome),
        }
    }

    fn watch_running(self: &Rc<Self>) {
        let launcher = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let Some(launcher) = launcher.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let outcome = launcher
                .running
                .borrow()
                .as_ref()
                .and_then(RunningAction::try_finish);
            if let Some(outcome) = outcome {
                launcher.running.borrow_mut().take();
                launcher.finish_outcome(outcome);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn finish_outcome(&self, outcome: Outcome) {
        if outcome == Outcome::Quit {
            if let Some(app) = self.window.application() {
                app.quit();
            }
        } else if outcome.should_dismiss() {
            self.window.set_visible(false);
        } else {
            // A watched launch reports after the launcher has already closed.
            // Something that failed has to be visible, so the window comes
            // back with the query intact rather than being reset.
            if !self.window.is_visible() {
                self.window.present();
                self.entry.grab_focus();
            }
            self.show_status(&outcome);
        }
    }

    /// Escape cancels the query first, and only then closes the launcher.
    fn dismiss(&self) {
        // A watched launch is not cancellable, so Escape falls through to
        // closing the launcher rather than killing what the user just started.
        let cancelling = self
            .running
            .borrow()
            .as_ref()
            .is_some_and(RunningAction::is_cancellable);
        if cancelling {
            if let Some(running) = self.running.borrow().as_ref() {
                running.cancel();
            }
            self.show_status(&Outcome::Pending("Cancelling action…".into()));
            return;
        }
        if self.pending_confirmation.borrow_mut().take().is_some() {
            self.clear_status();
            return;
        }
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

/// How much of one group the list is showing, when it is not showing all of
/// it. This counts what was rendered against what was there; the rule that
/// decided it lives in `search`.
fn withheld(kind: search::Kind, hits: &[usize], visible: &[&Item]) -> Option<(usize, usize)> {
    let shown = hits
        .iter()
        .filter(|&&index| visible[index].kind == kind)
        .count();
    let total = visible.iter().filter(|item| item.kind == kind).count();
    (total > shown).then_some((shown, total))
}

fn section_row(kind: search::Kind, trimmed: Option<(usize, usize)>) -> gtk::ListBoxRow {
    let heading = kind.heading();
    let label = gtk::Label::new(Some(heading));
    label.set_xalign(0.0);
    label.add_css_class("section");

    // GTK CSS has no letter-spacing, and these headings need the air.
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrInt::new_letter_spacing(pango::SCALE));
    label.set_attributes(Some(&attributes));

    let content = row_box(0);
    content.append(&label);

    // A trimmed group says so. Five of eighty-one under a heading that reads
    // "APPLICATIONS" would otherwise claim the machine has five.
    let described = match trimmed {
        Some((shown, total)) => {
            let count = gtk::Label::new(Some(&format!("{shown} of {total}")));
            count.add_css_class("section-count");
            count.set_hexpand(true);
            count.set_halign(gtk::Align::End);
            content.append(&count);
            format!("{heading}, showing {shown} of {total}. Type to search all of them.")
        }
        None => heading.to_string(),
    };

    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .selectable(false)
        .activatable(false)
        .focusable(false)
        .build();
    row.update_property(&[Property::Label(described.as_str())]);
    row
}

fn result_row(item: &Item) -> gtk::ListBoxRow {
    // The image is the tile. A box would hand its only child the child's
    // natural width and pack it against the left edge, whereas an image
    // centres its icon in whatever space CSS gives it.
    let tile = resolved_icon(item);
    tile.set_css_classes(&["tile", item.kind.slug()]);
    tile.set_halign(gtk::Align::Center);
    tile.set_valign(gtk::Align::Center);

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

/// The UI smoke suite: the keyboard-only path, driven end to end against the
/// real widget tree.
///
/// A Wayland client cannot inject its own input events and this session has no
/// injection tool, so the keys enter at [`Launcher::key`] — the same method the
/// window's key controller calls, with the same launcher state behind it. What
/// this does *not* prove is the compositor delivering a physical key press to
/// that controller; that stays a manual check, recorded in `PRODUCT_PLAN.md`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{Confirmation, ProcessAction};
    use crate::search::Kind;
    use crate::system::CommandSpec;

    /// Nothing to run, so confirming a mutation in the harness reports an
    /// unavailable tool rather than changing anything.
    const MISSING: &str = "scene-no-such-binary-for-tests";

    fn item(id: &str, title: &str, keyword: &str, action: Action) -> Item {
        Item {
            id: id.into(),
            title: title.into(),
            subtitle: "for the smoke harness".into(),
            kind: Kind::Scene,
            icon: None,
            category: None,
            keywords: vec![keyword.into()],
            action,
        }
    }

    fn mutation() -> Item {
        item(
            "smoke.mutation",
            "Mutate Something",
            "mutate",
            Action::Process {
                action: ProcessAction::mutating(
                    "smoke.mutation",
                    "Mutate Something",
                    CommandSpec::read_only(MISSING, [] as [&str; 0]),
                    Confirmation {
                        summary: "This would change the system.".into(),
                        target: MISSING.into(),
                    },
                ),
            },
        )
    }

    fn slow(program: &str) -> Item {
        item(
            "smoke.slow",
            "Wait For Something",
            "waiting",
            Action::Process {
                action: ProcessAction::read_only(
                    "smoke.slow",
                    "Wait For Something",
                    CommandSpec::read_only(program, [] as [&str; 0])
                        .with_timeout(Duration::from_secs(30)),
                ),
            },
        )
    }

    /// Give the main loop a turn, the way a running launcher gets one between
    /// two key presses.
    fn pump(context: &glib::MainContext) {
        for _ in 0..8 {
            context.iteration(false);
        }
    }

    /// Run the main loop until the action in flight reports, which is what the
    /// launcher's own `glib::timeout` does when GTK is driving.
    fn settle(launcher: &Rc<Launcher>, context: &glib::MainContext) {
        for _ in 0..600 {
            if launcher.running.borrow().is_none() {
                return;
            }
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the action never reported an outcome");
    }

    fn status(launcher: &Launcher) -> String {
        launcher.status_label.text().to_string()
    }

    fn selected_row_is_marked(launcher: &Launcher) -> bool {
        let rows = launcher.rows.borrow();
        rows.iter().enumerate().all(|(position, row)| {
            row.has_css_class("selected") == (position == launcher.selected.get())
        })
    }

    /// One test function on purpose: GTK has to be used from the thread that
    /// initialised it, and the test harness gives every test its own thread.
    #[test]
    fn the_keyboard_only_path_runs_end_to_end() {
        if gtk::init().is_err() {
            eprintln!("skipping the UI smoke suite: no display to open");
            return;
        }
        let context = glib::MainContext::default();
        let _owner = context
            .acquire()
            .expect("the test thread owns the default main context");

        load_styles();
        let app = gtk::Application::builder()
            .application_id("dev.scene.SceneSmoke")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(gio::Cancellable::NONE)
            .expect("register the harness application");

        let program = crate::system::tests::fake_program("sleep 30");
        let mut items = crate::search::tests::fixture();
        items.push(mutation());
        items.push(slow(&program.to_string_lossy()));
        // Enough applications to pass the resting limit, so the harness covers
        // the trimmed list and its heading as well as the full one.
        for index in 0..8 {
            let mut discovered = item(
                &format!("app{index}.desktop"),
                &format!("Zebra {index}"),
                "zebra",
                Action::Message {
                    text: "zebra".into(),
                },
            );
            discovered.kind = Kind::Application;
            items.push(discovered);
        }
        let applications = items
            .iter()
            .filter(|item| item.kind == Kind::Application)
            .count();
        let hidden = applications - Kind::Application.resting_limit_for_tests();
        let count = items.len();
        let resting = count - hidden;

        let launcher = Launcher::build(&app);
        // The harness must never write to the user's own history file, and a
        // stated ranking is what makes the positions below meaningful.
        *launcher.history.borrow_mut() = History::disabled();
        launcher.replace_items(items);

        // Activation: a focused search field and the whole index.
        launcher.present();
        pump(&context);
        assert!(launcher.window.is_visible(), "the launcher did not appear");
        // A GtkEntry delegates its caret to an internal GtkText, so the focus
        // lands inside the search field rather than on it.
        let focused = gtk::prelude::GtkWindowExt::focus(&launcher.window)
            .expect("something in the launcher holds the focus");
        assert!(
            focused.is_ancestor(&launcher.entry)
                || focused == launcher.entry.clone().upcast::<gtk::Widget>(),
            "the search field is not focused"
        );
        assert!(launcher.entry.text().is_empty());
        // The resting list is trimmed, and the heading says so rather than
        // implying the machine has five applications.
        assert_eq!(launcher.rows.borrow().len(), resting);
        assert!(hidden > 0, "the harness must exercise a trimmed group");
        assert!(!launcher.status.is_visible());

        // Typing still reaches every one of them.
        launcher.entry.set_text("zebra");
        assert_eq!(launcher.rows.borrow().len(), 8, "a query was trimmed");
        launcher.entry.set_text("");
        assert_eq!(launcher.rows.borrow().len(), resting);

        // Typing narrows the list and selects the best result.
        launcher.entry.set_text("system");
        let narrowed = launcher.rows.borrow().len();
        assert!(narrowed > 1 && narrowed < count, "{narrowed} of {count}");
        assert_eq!(launcher.selected.get(), 0);
        assert!(selected_row_is_marked(&launcher));

        // Down and Up move the selection, and wrap at either end.
        launcher.key(gdk::Key::Down);
        assert_eq!(launcher.selected.get(), 1);
        assert!(selected_row_is_marked(&launcher));
        launcher.key(gdk::Key::Up);
        assert_eq!(launcher.selected.get(), 0);
        launcher.key(gdk::Key::Up);
        assert_eq!(launcher.selected.get(), narrowed - 1, "Up did not wrap");
        launcher.key(gdk::Key::Down);
        assert_eq!(launcher.selected.get(), 0, "Down did not wrap");

        // A query with no results says so, and Enter on nothing does nothing.
        launcher.entry.set_text("zzzqqq");
        assert!(launcher.rows.borrow().is_empty());
        assert!(launcher.empty.is_visible());
        assert!(!launcher.scroller.is_visible());
        launcher.key(gdk::Key::Return);
        assert!(launcher.window.is_visible());
        assert!(!launcher.status.is_visible());

        // Enter runs the selected result and reports it in the footer.
        launcher.entry.set_text("about");
        launcher.key(gdk::Key::Return);
        assert!(launcher.status.is_visible());
        assert!(
            status(&launcher).starts_with("Scene — "),
            "{}",
            status(&launcher)
        );
        assert!(
            launcher.window.is_visible(),
            "a report must not close the launcher"
        );

        // Escape clears the query first, and only then closes.
        launcher.key(gdk::Key::Escape);
        assert!(launcher.entry.text().is_empty());
        assert!(launcher.window.is_visible());
        launcher.key(gdk::Key::Escape);
        assert!(
            !launcher.window.is_visible(),
            "Escape did not close the launcher"
        );

        // Re-activation resets the query, the status and the selection.
        launcher.entry.set_text("system");
        launcher.key(gdk::Key::Down);
        launcher.present();
        assert!(launcher.entry.text().is_empty());
        assert!(!launcher.status.is_visible());
        assert_eq!(launcher.selected.get(), 0);
        assert_eq!(launcher.rows.borrow().len(), resting);

        // A mutation asks first, and only Enter answers.
        launcher.entry.set_text("mutate");
        launcher.key(gdk::Key::Return);
        assert!(launcher.pending_confirmation.borrow().is_some());
        assert!(
            status(&launcher).starts_with("Confirm — "),
            "{}",
            status(&launcher)
        );
        assert!(launcher.running.borrow().is_none(), "nothing may run yet");

        launcher.activate_selected(false);
        assert!(
            launcher.pending_confirmation.borrow().is_some(),
            "a result-row click confirmed a mutation"
        );
        assert!(launcher.running.borrow().is_none());

        launcher.key(gdk::Key::Escape);
        assert!(launcher.pending_confirmation.borrow().is_none());
        assert!(!launcher.status.is_visible());
        assert!(launcher.window.is_visible());

        launcher.key(gdk::Key::Return);
        assert!(launcher.pending_confirmation.borrow().is_some());
        launcher.key(gdk::Key::Return);
        settle(&launcher, &context);
        assert!(
            status(&launcher).starts_with("Unavailable — "),
            "{}",
            status(&launcher)
        );
        assert!(launcher.window.is_visible());

        // A long action is cancellable from the keyboard.
        launcher.entry.set_text("waiting");
        launcher.key(gdk::Key::Return);
        assert!(launcher.running.borrow().is_some());
        assert!(
            status(&launcher).starts_with("Working — "),
            "{}",
            status(&launcher)
        );
        launcher.key(gdk::Key::Escape);
        assert!(
            launcher.window.is_visible(),
            "Escape cancelled and closed at once"
        );
        settle(&launcher, &context);
        assert!(
            status(&launcher).starts_with("Cancelled — "),
            "{}",
            status(&launcher)
        );

        std::fs::remove_file(program).expect("remove fake executable");
        launcher.window.set_visible(false);
    }
}
