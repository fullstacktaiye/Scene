//! The launcher surface: window, search field, result list, footer.
//!
//! This module renders what `search` produced and reports what `actions`
//! returned. It does not decide either.

use std::cell::{Cell, OnceCell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::accessible::{Property, State};
use gtk::prelude::*;
use gtk::{gdk, gio, glib, pango};

use crate::actions::{self, Action, Outcome, RunningAction, StartedAction};
use crate::integrations;
use crate::platform::{self, CopilotStatus};
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
    stack: gtk::Stack,
    entry: gtk::Entry,
    list: gtk::ListBox,
    action_popover: gtk::Popover,
    action_list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    empty: gtk::Box,
    empty_title: gtk::Label,
    hints: gtk::Box,
    status: gtk::Box,
    status_icon: gtk::Image,
    status_label: gtk::Label,
    shortcut_session: gtk::Label,
    shortcut_active: gtk::Label,
    copilot_status_label: gtk::Label,
    shortcut_recorder: gtk::Button,
    copilot_test: gtk::Button,
    settings_feedback: gtk::Label,
    provider_list: gtk::Box,
    history_switch: gtk::Switch,
    clear_history: gtk::Button,
    command_history_switch: gtk::Switch,
    clear_command_history: gtk::Button,
    file_content_switch: gtk::Switch,
    autostart_switch: gtk::Switch,

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
    action_selected: Cell<usize>,
    action_choices: RefCell<Vec<(String, Action)>>,
    /// The action selected for confirmation is frozen here, with the result id
    /// it came from. Query changes and selection movement cannot substitute a
    /// different mutation on Enter.
    pending_confirmation: RefCell<Option<(String, Action)>>,
    running: RefCell<Option<RunningAction>>,
    running_in_settings: Cell<bool>,
    copilot_status: Cell<CopilotStatus>,
    /// What the user has chosen before, which adjusts ranking within a group.
    /// `search` decides how much it counts for; this module only records it.
    history: RefCell<History>,
    config: RefCell<integrations::Config>,
    query_generation: Cell<u64>,
    self_weak: OnceCell<Weak<Launcher>>,
}

impl Launcher {
    pub fn build(app: &gtk::Application) -> Rc<Self> {
        let config = integrations::Config::load();
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
        let action_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["action-menu"])
            .build();
        let action_popover = gtk::Popover::builder()
            .child(&action_list)
            .autohide(false)
            .has_arrow(false)
            .build();
        action_popover.set_parent(&list);

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

        let shortcut_session = settings_value();
        let shortcut_active = settings_value();
        let copilot_status_label = settings_value();
        let shortcut_recorder = gtk::Button::with_label("Change in KDE Shortcuts");
        shortcut_recorder.add_css_class("suggested-action");
        let copilot_test = gtk::Button::with_label("Test Copilot key");
        let settings_feedback = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["settings-feedback"])
            .build();
        let back = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text("Back to search")
            .build();

        let settings_header = row_box(12);
        settings_header.add_css_class("settings-header");
        settings_header.append(&back);
        let settings_title = gtk::Label::new(Some("Scene Settings"));
        settings_title.add_css_class("settings-title");
        settings_header.append(&settings_title);

        let settings_body = gtk::Box::new(gtk::Orientation::Vertical, 14);
        settings_body.add_css_class("settings-body");
        settings_body.append(&settings_section(
            "Desktop session",
            &shortcut_session,
            "Global activation is implemented by the desktop, not by a hidden Scene daemon.",
        ));
        settings_body.append(&settings_section(
            "Active shortcut",
            &shortcut_active,
            "Meta+Space is Scene's packaged fallback. KDE remains the source of truth for changes and conflicts.",
        ));
        settings_body.append(&shortcut_recorder);
        settings_body.append(&settings_section(
            "Copilot key",
            &copilot_status_label,
            "Scene reports support only after an event or desktop action is observed. It never infers hardware from F23 capability bits.",
        ));
        settings_body.append(&copilot_test);
        let provider_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        settings_body.append(&settings_group("Search providers", &provider_list));

        let history_switch = gtk::Switch::builder()
            .active(config.history_enabled)
            .valign(gtk::Align::Center)
            .build();
        let clear_history = gtk::Button::with_label("Clear result history");
        let command_history_switch = gtk::Switch::builder()
            .active(config.command_history_enabled)
            .valign(gtk::Align::Center)
            .build();
        let clear_command_history = gtk::Button::with_label("Clear command history");
        let file_content_switch = gtk::Switch::builder()
            .active(config.file_content_enabled)
            .valign(gtk::Align::Center)
            .build();
        let autostart_switch = gtk::Switch::builder()
            .active(platform::autostart_enabled())
            .valign(gtk::Align::Center)
            .build();
        let privacy = gtk::Box::new(gtk::Orientation::Vertical, 8);
        privacy.append(&toggle_row(
            "Recent and frequent ranking",
            "Stored locally and clearable at any time.",
            &history_switch,
        ));
        privacy.append(&clear_command_history);
        privacy.append(&clear_history);
        privacy.append(&toggle_row(
            "Command history",
            "Off by default because commands can contain private data.",
            &command_history_switch,
        ));
        privacy.append(&toggle_row(
            "File-content search",
            "Uses the existing Baloo index; Scene never builds another index.",
            &file_content_switch,
        ));
        privacy.append(&toggle_row(
            "Start Scene at login",
            "Keeps the single launcher instance resident and warm.",
            &autostart_switch,
        ));
        settings_body.append(&settings_group("Privacy and startup", &privacy));
        settings_body.append(&settings_feedback);

        let settings_surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
        settings_surface.add_css_class("surface");
        settings_surface.add_css_class("settings-surface");
        settings_surface.append(&settings_header);
        let settings_rule = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        settings_rule.add_css_class("rule");
        settings_surface.append(&settings_rule);
        let settings_scroller = gtk::ScrolledWindow::builder()
            .child(&settings_body)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(600)
            .build();
        settings_surface.append(&settings_scroller);
        settings_surface.set_size_request(WIDTH, -1);
        apply_preferences(&settings_surface);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.add_named(&surface, Some("launcher"));
        stack.add_named(&settings_surface, Some("settings"));
        stack.set_visible_child_name("launcher");

        let window = gtk::Window::builder()
            .application(app)
            .title("Scene")
            .decorated(false)
            .resizable(false)
            .hide_on_close(true)
            .css_classes(["scene"])
            .child(&stack)
            .build();

        let launcher = Rc::new(Self {
            window,
            stack,
            entry,
            list,
            action_popover,
            action_list,
            scroller,
            empty,
            empty_title,
            hints,
            status,
            status_icon,
            status_label,
            shortcut_session,
            shortcut_active,
            copilot_status_label,
            shortcut_recorder,
            copilot_test,
            settings_feedback,
            provider_list,
            history_switch,
            clear_history,
            command_history_switch,
            clear_command_history,
            file_content_switch,
            autostart_switch,
            items: RefCell::new(search::index()),
            answers: RefCell::new(Vec::new()),
            _apps: gio::AppInfoMonitor::get(),
            hits: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            action_selected: Cell::new(0),
            action_choices: RefCell::new(Vec::new()),
            pending_confirmation: RefCell::new(None),
            running: RefCell::new(None),
            running_in_settings: Cell::new(false),
            copilot_status: Cell::new(CopilotStatus::NotTested),
            history: RefCell::new(History::load_enabled(config.history_enabled)),
            config: RefCell::new(config),
            query_generation: Cell::new(0),
            self_weak: OnceCell::new(),
        });

        launcher
            .self_weak
            .set(Rc::downgrade(&launcher))
            .expect("launcher weak reference is set once");

        launcher.connect_signals();
        launcher.rebuild_provider_settings();
        let this = Rc::downgrade(&launcher);
        back.connect_clicked(move |_| {
            if let Some(launcher) = this.upgrade() {
                launcher.close_settings();
            }
        });
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

        let this = Rc::downgrade(self);
        self.action_list.connect_row_activated(move |_, row| {
            let Some(launcher) = this.upgrade() else {
                return;
            };
            launcher.action_selected.set(row.index().max(0) as usize);
            launcher.activate_action_choice();
        });

        let this = Rc::downgrade(self);
        self.shortcut_recorder.connect_clicked(move |_| {
            let Some(launcher) = this.upgrade() else {
                return;
            };
            let status = platform::ShortcutStatus::detect();
            let Some(action) = status.recorder_action() else {
                launcher.set_settings_feedback(
                    "KDE's shortcut recorder is not available in this session.",
                    "warn",
                );
                return;
            };
            launcher.running_in_settings.set(true);
            launcher.start_action("settings.shortcuts.open", &action, false);
        });

        let this = Rc::downgrade(self);
        self.copilot_test.connect_clicked(move |_| {
            if let Some(launcher) = this.upgrade() {
                launcher.toggle_copilot_test();
            }
        });

        let this = Rc::downgrade(self);
        self.history_switch.connect_state_set(move |_, enabled| {
            if let Some(launcher) = this.upgrade() {
                launcher.config.borrow_mut().history_enabled = enabled;
                launcher.history.borrow_mut().set_enabled(enabled);
                launcher.save_settings();
                launcher.refresh();
            }
            glib::Propagation::Proceed
        });

        let this = Rc::downgrade(self);
        self.clear_history.connect_clicked(move |_| {
            if let Some(launcher) = this.upgrade() {
                launcher.history.borrow_mut().clear();
                launcher.set_settings_feedback("Result history cleared.", "ok");
                launcher.refresh();
            }
        });

        let this = Rc::downgrade(self);
        self.command_history_switch
            .connect_state_set(move |_, enabled| {
                if let Some(launcher) = this.upgrade() {
                    launcher.config.borrow_mut().command_history_enabled = enabled;
                    launcher.save_settings();
                }
                glib::Propagation::Proceed
            });

        let this = Rc::downgrade(self);
        self.clear_command_history.connect_clicked(move |_| {
            let Some(launcher) = this.upgrade() else {
                return;
            };
            match actions::clear_command_history() {
                Ok(()) => launcher.set_settings_feedback("Command history cleared.", "ok"),
                Err(error) => launcher.set_settings_feedback(
                    &format!("Could not clear command history: {error}"),
                    "error",
                ),
            }
        });

        let this = Rc::downgrade(self);
        self.file_content_switch
            .connect_state_set(move |_, enabled| {
                if let Some(launcher) = this.upgrade() {
                    launcher.config.borrow_mut().file_content_enabled = enabled;
                    launcher.save_settings();
                    launcher.refresh();
                }
                glib::Propagation::Proceed
            });

        let this = Rc::downgrade(self);
        self.autostart_switch.connect_state_set(move |_, enabled| {
            let Some(launcher) = this.upgrade() else {
                return glib::Propagation::Stop;
            };
            match platform::set_autostart(enabled) {
                Ok(()) => {
                    launcher.set_settings_feedback(
                        if enabled {
                            "Scene will start in the background at login."
                        } else {
                            "Scene will no longer start automatically."
                        },
                        "ok",
                    );
                    glib::Propagation::Proceed
                }
                Err(error) => {
                    launcher.set_settings_feedback(&error, "error");
                    glib::Propagation::Stop
                }
            }
        });

        let this = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |window| {
            if window.is_active()
                && let Some(launcher) = this.upgrade()
                && launcher.in_settings()
            {
                launcher.refresh_shortcut_status();
            }
        });

        // Capture phase: arrows and Enter are ours before the entry sees them,
        // everything else falls through to the search field.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let this = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| match this.upgrade() {
            Some(launcher) => launcher.key_event(key, state),
            None => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);

        let settings = gio::SimpleAction::new("settings", None);
        let this = Rc::downgrade(self);
        settings.connect_activate(move |_, _| {
            if let Some(launcher) = this.upgrade() {
                launcher.show_settings();
            }
        });
        if let Some(app) = self.window.application() {
            app.add_action(&settings);
            app.set_accels_for_action("app.settings", &["<Control>comma"]);
        }
    }

    /// The whole keyboard contract, in one place. The controller above only
    /// delivers the key; everything the launcher does with one is here, which
    /// is also what the smoke harness drives.
    #[cfg(test)]
    fn key(self: &Rc<Self>, key: gdk::Key) -> glib::Propagation {
        self.key_event(key, gdk::ModifierType::empty())
    }

    fn key_event(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        if self.in_settings() {
            if self.copilot_status.get() == CopilotStatus::Waiting {
                if key == gdk::Key::Escape {
                    self.finish_copilot_test(CopilotStatus::NotObserved);
                    return glib::Propagation::Stop;
                }
                let name = key.name().unwrap_or_default();
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                let meta =
                    state.intersects(gdk::ModifierType::META_MASK | gdk::ModifierType::SUPER_MASK);
                if let Some(observed) = platform::classify_copilot_key(&name, shift, meta) {
                    self.finish_copilot_test(observed);
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }
            if key == gdk::Key::Escape {
                self.close_settings();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if self.action_popover.is_visible() {
            match key {
                gdk::Key::Down => self.move_action_selection(1),
                gdk::Key::Up => self.move_action_selection(-1),
                gdk::Key::Return | gdk::Key::KP_Enter => self.activate_action_choice(),
                gdk::Key::Escape => self.close_action_menu(),
                _ => return glib::Propagation::Proceed,
            }
            return glib::Propagation::Stop;
        }
        if key == gdk::Key::k && state.contains(gdk::ModifierType::CONTROL_MASK) {
            self.open_action_menu();
            return glib::Propagation::Stop;
        }
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
        self.stack.set_visible_child_name("launcher");
        self.entry.set_text("");
        self.clear_status();
        self.refresh();
        self.window.present();
        self.entry.grab_focus();
    }

    /// Run `callback` once the window has actually put a frame on the screen.
    ///
    /// `present` returning is not that moment: it hands the surface to the
    /// compositor, and the frame clock is what reports it being drawn. Only
    /// `--measure` needs the difference, and measuring the wrong instant would
    /// flatter the number.
    pub fn on_first_frame(&self, callback: impl FnOnce() + 'static) {
        let callback = RefCell::new(Some(callback));
        self.window.add_tick_callback(move |_, _| {
            if let Some(callback) = callback.borrow_mut().take() {
                callback();
            }
            glib::ControlFlow::Break
        });
    }

    /// Desktop activation is a toggle. A Copilot test is the one exception:
    /// while armed, another activation is itself the observed desktop action.
    pub fn activate(&self) {
        if self.window.is_visible() {
            if self.in_settings() && self.copilot_status.get() == CopilotStatus::Waiting {
                self.finish_copilot_test(CopilotStatus::ActivationObserved);
            } else {
                self.window.set_visible(false);
            }
        } else {
            self.present();
        }
    }

    fn in_settings(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some("settings")
    }

    fn show_settings(&self) {
        self.pending_confirmation.borrow_mut().take();
        self.clear_status();
        self.settings_feedback.set_visible(false);
        self.refresh_shortcut_status();
        self.stack.set_visible_child_name("settings");
        self.window.present();
        self.copilot_test.grab_focus();
    }

    fn close_settings(&self) {
        if self.copilot_status.get() == CopilotStatus::Waiting {
            self.finish_copilot_test(CopilotStatus::NotObserved);
        }
        self.stack.set_visible_child_name("launcher");
        self.entry.grab_focus();
    }

    fn refresh_shortcut_status(&self) {
        let status = platform::ShortcutStatus::detect();
        self.shortcut_session.set_text(&status.desktop.summary());
        self.shortcut_active.set_text(&status.shortcut_summary());
        let recorder_available = status.recorder.is_some();
        self.shortcut_recorder.set_sensitive(recorder_available);
        self.shortcut_recorder
            .set_tooltip_text(Some(if recorder_available {
                "Open KDE's native shortcut recorder"
            } else {
                "KDE's shortcut recorder is unavailable in this session"
            }));
        self.copilot_status_label
            .set_text(self.copilot_status.get().summary());
    }

    fn toggle_copilot_test(&self) {
        let next = if self.copilot_status.get() == CopilotStatus::Waiting {
            CopilotStatus::NotObserved
        } else {
            CopilotStatus::Waiting
        };
        if next == CopilotStatus::Waiting {
            self.copilot_status.set(next);
            self.copilot_status_label.set_text(next.summary());
            self.copilot_test.set_label("Finish without detection");
            self.set_settings_feedback(
                "Press the Copilot key now. Escape ends the test without claiming support.",
                "info",
            );
        } else {
            self.finish_copilot_test(next);
        }
    }

    fn finish_copilot_test(&self, status: CopilotStatus) {
        self.copilot_status.set(status);
        self.copilot_status_label.set_text(status.summary());
        self.copilot_test.set_label("Test Copilot key");
        let tone = match status {
            CopilotStatus::BindableObserved | CopilotStatus::ActivationObserved => "ok",
            CopilotStatus::UnbindableObserved | CopilotStatus::NotObserved => "warn",
            CopilotStatus::NotTested | CopilotStatus::Waiting => "info",
        };
        self.set_settings_feedback(status.summary(), tone);
    }

    fn set_settings_feedback(&self, text: &str, tone: &str) {
        self.settings_feedback
            .set_css_classes(&["settings-feedback", tone]);
        self.settings_feedback.set_text(text);
        self.settings_feedback.set_visible(true);
    }

    fn save_settings(&self) {
        if let Err(error) = self.config.borrow().save() {
            self.set_settings_feedback(&format!("Could not save settings: {error}"), "error");
        }
    }

    fn reload_index(&self) {
        *self.items.borrow_mut() = search::index();
        self.refresh();
    }

    fn rebuild_provider_settings(self: &Rc<Self>) {
        while let Some(child) = self.provider_list.first_child() {
            self.provider_list.remove(&child);
        }
        let metadata = integrations::provider_metadata();
        for id in self.config.borrow().ordered_provider_ids() {
            let Some(provider) = metadata.iter().find(|provider| provider.id == id) else {
                continue;
            };
            let enabled = self.config.borrow().provider_enabled(provider.id);
            let toggle = gtk::Switch::builder()
                .active(enabled)
                .valign(gtk::Align::Center)
                .build();
            let up = gtk::Button::builder()
                .icon_name("go-up-symbolic")
                .tooltip_text(format!("Move {} earlier", provider.title))
                .build();
            let down = gtk::Button::builder()
                .icon_name("go-down-symbolic")
                .tooltip_text(format!("Move {} later", provider.title))
                .build();
            let controls = row_box(4);
            controls.append(&up);
            controls.append(&down);
            controls.append(&toggle);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
            let title = gtk::Label::builder()
                .label(provider.title)
                .xalign(0.0)
                .build();
            title.add_css_class("settings-value");
            let description = gtk::Label::builder()
                .label(provider.description)
                .xalign(0.0)
                .wrap(true)
                .build();
            description.add_css_class("settings-explanation");
            text.append(&title);
            text.append(&description);
            let row = row_box(8);
            row.add_css_class("provider-setting");
            row.append(&text);
            row.append(&controls);
            self.provider_list.append(&row);

            let provider_id = provider.id.to_string();
            let this = Rc::downgrade(self);
            toggle.connect_state_set(move |_, enabled| {
                if let Some(launcher) = this.upgrade() {
                    launcher
                        .config
                        .borrow_mut()
                        .set_provider_enabled(&provider_id, enabled);
                    launcher.save_settings();
                    launcher.reload_index();
                }
                glib::Propagation::Proceed
            });

            let provider_id = provider.id.to_string();
            let this = Rc::downgrade(self);
            up.connect_clicked(move |_| {
                if let Some(launcher) = this.upgrade() {
                    launcher.config.borrow_mut().move_provider(&provider_id, -1);
                    launcher.save_settings();
                    launcher.reload_index();
                    launcher.rebuild_provider_settings();
                }
            });

            let provider_id = provider.id.to_string();
            let this = Rc::downgrade(self);
            down.connect_clicked(move |_| {
                if let Some(launcher) = this.upgrade() {
                    launcher.config.borrow_mut().move_provider(&provider_id, 1);
                    launcher.save_settings();
                    launcher.reload_index();
                    launcher.rebuild_provider_settings();
                }
            });
        }
    }

    fn refresh(&self) {
        let query = self.entry.text().to_string();
        *self.answers.borrow_mut() = integrations::answers(&query);
        let generation = self.query_generation.get().wrapping_add(1);
        self.query_generation.set(generation);
        let weak = self
            .self_weak
            .get()
            .expect("launcher weak reference was initialized")
            .clone();
        let callback_query = query.clone();
        integrations::answers_async(&query, move |items| {
            let Some(launcher) = weak.upgrade() else {
                return;
            };
            if launcher.query_generation.get() != generation
                || launcher.entry.text().as_str() != callback_query
            {
                return;
            }
            launcher.answers.borrow_mut().extend(items);
            launcher.render_results(&callback_query);
        });
        self.render_results(&query);
    }

    fn render_results(&self, query: &str) {
        let answers = self.answers.borrow();
        let items = self.items.borrow();
        // One ranked list over both sources, so a query answer is ordered by
        // the same rules as everything else rather than pinned by the UI.
        let visible: Vec<&Item> = answers.iter().chain(items.iter()).collect();
        let hits = search::search(query, &visible, &self.history.borrow());

        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let mut rows = Vec::with_capacity(hits.len());
        let mut group: Option<&str> = None;
        // A group is only ever trimmed in the resting state, so that is the
        // only time a heading has a count to report.
        let resting = query.trim().is_empty();

        for &index in &hits {
            let item = visible[index];
            if group != Some(item.provider.as_str()) {
                group = Some(item.provider.as_str());
                let trimmed = resting
                    .then(|| withheld(&item.provider, &hits, &visible))
                    .flatten();
                self.list
                    .append(&section_row(&item.provider_title, trimmed));
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
            let text = actions::action_confirmation_text(&action)
                .expect("an action requiring confirmation names its consequence");
            *self.pending_confirmation.borrow_mut() = Some((id, action));
            self.show_status(&Outcome::AwaitingConfirmation(text));
            return;
        }
        self.start_action(&id, &action, false);
    }

    fn open_action_menu(&self) {
        if self.running.borrow().is_some() || self.pending_confirmation.borrow().is_some() {
            return;
        }
        let index = {
            let hits = self.hits.borrow();
            let Some(index) = hits.get(self.selected.get()) else {
                return;
            };
            *index
        };
        let Some((id, primary, secondary)) = self.chosen_actions(index) else {
            return;
        };
        while let Some(child) = self.action_list.first_child() {
            self.action_list.remove(&child);
        }
        let mut choices = vec![(id, primary.clone())];
        let primary_row = action_row(&format!("Default — {}", action_label(&primary)));
        self.action_list.append(&primary_row);
        for action in secondary {
            self.action_list.append(&action_row(&action.label));
            choices.push((action.id, action.action));
        }
        *self.action_choices.borrow_mut() = choices;
        self.action_selected.set(0);
        self.apply_action_selection();
        self.action_popover.popup();
    }

    fn close_action_menu(&self) {
        self.action_popover.popdown();
        self.action_choices.borrow_mut().clear();
        self.entry.grab_focus();
    }

    fn move_action_selection(&self, delta: i32) {
        let count = self.action_choices.borrow().len();
        if count == 0 {
            return;
        }
        self.action_selected
            .set((self.action_selected.get() as i32 + delta).rem_euclid(count as i32) as usize);
        self.apply_action_selection();
    }

    fn apply_action_selection(&self) {
        let selected = self.action_selected.get();
        let mut child = self.action_list.first_child();
        let mut position = 0;
        while let Some(row) = child {
            if position == selected {
                row.add_css_class("selected");
            } else {
                row.remove_css_class("selected");
            }
            child = row.next_sibling();
            position += 1;
        }
    }

    fn activate_action_choice(self: &Rc<Self>) {
        let choice = self
            .action_choices
            .borrow()
            .get(self.action_selected.get())
            .cloned();
        self.close_action_menu();
        let Some((id, action)) = choice else { return };
        if actions::requires_confirmation(&action) {
            let text = actions::action_confirmation_text(&action)
                .expect("an action requiring confirmation names its consequence");
            *self.pending_confirmation.borrow_mut() = Some((id, action));
            self.show_status(&Outcome::AwaitingConfirmation(text));
        } else {
            self.start_action(&id, &action, false);
        }
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

    fn chosen_actions(&self, index: usize) -> Option<(String, Action, Vec<search::ItemAction>)> {
        let answers = self.answers.borrow();
        let item = if let Some(item) = answers.get(index) {
            item
        } else {
            let index = index.checked_sub(answers.len())?;
            let items = self.items.borrow();
            let item = items.get(index)?;
            return Some((
                item.id.clone(),
                item.action.clone(),
                item.secondary_actions.clone(),
            ));
        };
        Some((
            item.id.clone(),
            item.action.clone(),
            item.secondary_actions.clone(),
        ))
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
                    Action::Dbus { action } => action.title.clone(),
                    Action::Signal { action } => action.title.clone(),
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
        if outcome == Outcome::ShowSettings {
            self.show_settings();
        } else if self.running_in_settings.replace(false) {
            let tone = outcome.tone();
            let message = format!("{} — {}", outcome.prefix(), outcome.message());
            self.set_settings_feedback(&message, tone);
            self.refresh_shortcut_status();
        } else if outcome == Outcome::Quit {
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
        if self.action_popover.is_visible() {
            self.close_action_menu();
            return;
        }
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

fn settings_value() -> gtk::Label {
    gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .css_classes(["settings-value"])
        .build()
}

fn settings_section(title: &str, value: &gtk::Label, explanation: &str) -> gtk::Box {
    let title = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .css_classes(["settings-label"])
        .build();
    let explanation = gtk::Label::builder()
        .label(explanation)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["settings-explanation"])
        .build();
    let section = gtk::Box::new(gtk::Orientation::Vertical, 4);
    section.append(&title);
    section.append(value);
    section.append(&explanation);
    section
}

fn settings_group(title: &str, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Label::builder().label(title).xalign(0.0).build();
    title.add_css_class("settings-label");
    group.append(&title);
    group.append(content);
    group
}

fn toggle_row(title: &str, explanation: &str, toggle: &impl IsA<gtk::Widget>) -> gtk::Box {
    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    let title = gtk::Label::builder().label(title).xalign(0.0).build();
    title.add_css_class("settings-value");
    let explanation = gtk::Label::builder()
        .label(explanation)
        .xalign(0.0)
        .wrap(true)
        .build();
    explanation.add_css_class("settings-explanation");
    text.append(&title);
    text.append(&explanation);
    let row = row_box(8);
    row.append(&text);
    row.append(toggle);
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

fn action_row(label: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();
    gtk::ListBoxRow::builder()
        .child(&label)
        .activatable(true)
        .selectable(false)
        .build()
}

fn action_label(action: &Action) -> String {
    match action {
        Action::Launch { app } => format!("Open {}", app.display_name()),
        Action::DesktopLaunch { app, .. } => format!("Open {}", app.display_name()),
        Action::Open { target } => format!("Open {target}"),
        Action::Process { action } => action.title.clone(),
        Action::Dbus { action } => action.title.clone(),
        Action::Signal { action } => action.title.clone(),
        Action::Copy { label, .. } => format!("Copy {label}"),
        Action::Message { .. } => "Show details".into(),
        Action::ShowSettings => "Open settings".into(),
        Action::Quit => "Quit Scene".into(),
    }
}

/// How much of one group the list is showing, when it is not showing all of
/// it. This counts what was rendered against what was there; the rule that
/// decided it lives in `search`.
fn withheld(provider: &str, hits: &[usize], visible: &[&Item]) -> Option<(usize, usize)> {
    let shown = hits
        .iter()
        .filter(|&&index| visible[index].provider == provider)
        .count();
    let total = visible
        .iter()
        .filter(|item| item.provider == provider)
        .count();
    (total > shown).then_some((shown, total))
}

fn section_row(heading: &str, trimmed: Option<(usize, usize)>) -> gtk::ListBoxRow {
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
    let tile: gtk::Widget = if item.provider == "colors" {
        match gdk::RGBA::parse(&item.title) {
            Ok(color) => {
                let swatch = gtk::DrawingArea::new();
                swatch.set_content_width(34);
                swatch.set_content_height(34);
                swatch.add_css_class("color-swatch");
                swatch.update_property(&[Property::Label(
                    format!("Color swatch {}", item.title).as_str(),
                )]);
                swatch.set_draw_func(move |_, context, width, height| {
                    context.set_source_rgba(
                        f64::from(color.red()),
                        f64::from(color.green()),
                        f64::from(color.blue()),
                        f64::from(color.alpha()),
                    );
                    context.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
                    let _ = context.fill();
                });
                swatch.upcast()
            }
            Err(_) => resolved_icon(item).upcast(),
        }
    } else {
        resolved_icon(item).upcast()
    };
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
            provider: "scene".into(),
            provider_title: "Scene".into(),
            provider_priority: 90,
            title: title.into(),
            subtitle: "for the smoke harness".into(),
            kind: Kind::Scene,
            icon: None,
            category: None,
            keywords: vec![keyword.into()],
            action,
            secondary_actions: Vec::new(),
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
        // GTK initialization races with otherwise thread-safe GIO unit tests
        // when libtest runs this in its shared process. Keep the full harness
        // opt-in so CI invokes it in its own test process.
        if std::env::var_os("SCENE_UI_TEST").is_none() {
            eprintln!("skipping the UI smoke suite: set SCENE_UI_TEST=1 to run it in isolation");
            return;
        }
        if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
            eprintln!("skipping the UI smoke suite: no display environment is available");
            return;
        }
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
        items.push(item(
            "scene.settings",
            "Scene Settings",
            "settings shortcut copilot",
            Action::ShowSettings,
        ));
        items.push(mutation());
        items.push(slow(&program.to_string_lossy()));
        let about = items
            .iter_mut()
            .find(|item| item.id == "scene.about")
            .expect("fixture has an About result");
        about.secondary_actions.push(crate::search::ItemAction {
            id: "scene.about.secondary".into(),
            label: "Report secondary action".into(),
            action: Action::Message {
                text: "secondary action reached".into(),
            },
        });
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
        launcher.replace_items(items.clone());

        // Activation: a focused search field and the whole index.
        launcher.present();
        pump(&context);
        // GAppInfoMonitor can emit its initial host catalogue change while
        // the loop is first pumped. Restore the hermetic fixture after that
        // one desktop-driven event.
        launcher.replace_items(items);
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

        // Ctrl+K exposes every action and the secondary action is keyboard-only
        // reachable through the same execution path.
        launcher.entry.set_text("about");
        launcher.key_event(gdk::Key::k, gdk::ModifierType::CONTROL_MASK);
        assert!(launcher.action_popover.is_visible());
        assert_eq!(launcher.action_choices.borrow().len(), 2);
        launcher.key(gdk::Key::Down);
        launcher.key(gdk::Key::Return);
        assert!(status(&launcher).contains("secondary action reached"));

        // Enter runs the selected result's primary action and reports it.
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

        // Desktop activation toggles the one resident window rather than
        // creating another or leaving a stale query behind.
        launcher.activate();
        assert!(!launcher.window.is_visible(), "activation did not hide");
        launcher.entry.set_text("stale");
        launcher.activate();
        assert!(launcher.window.is_visible(), "activation did not present");
        assert!(launcher.entry.text().is_empty(), "activation kept a query");

        // Settings are keyboard-reachable and present observed desktop state.
        launcher.entry.set_text("copilot");
        launcher.key(gdk::Key::Return);
        assert!(launcher.in_settings());
        assert!(!launcher.shortcut_session.text().is_empty());
        assert!(!launcher.shortcut_active.text().is_empty());

        // Direct F23 observation proves only the event Scene actually saw.
        launcher.toggle_copilot_test();
        assert_eq!(launcher.copilot_status.get(), CopilotStatus::Waiting);
        launcher.key_event(
            gdk::Key::F23,
            gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::META_MASK,
        );
        assert_eq!(
            launcher.copilot_status.get(),
            CopilotStatus::BindableObserved
        );

        // A bound global shortcut may be consumed by KDE before GTK sees its
        // key event. While the test is armed, application activation is the
        // observed desktop action and must not toggle the window closed.
        launcher.toggle_copilot_test();
        launcher.activate();
        assert_eq!(
            launcher.copilot_status.get(),
            CopilotStatus::ActivationObserved
        );
        assert!(launcher.window.is_visible());
        launcher.key(gdk::Key::Escape);
        assert!(!launcher.in_settings());

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
