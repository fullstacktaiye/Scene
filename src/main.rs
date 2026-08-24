mod actions;
mod apps;
mod integrations;
mod search;
mod system;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "dev.scene.Scene";

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|app| {
        ui::load_styles();

        let quit = gio::SimpleAction::new("quit", None);
        let weak = app.downgrade();
        quit.connect_activate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.quit();
            }
        });
        app.add_action(&quit);
        app.set_accels_for_action("app.quit", &["<Control>q"]);
    });

    // One launcher for the life of the process. Activating Scene again — a
    // second `scene` invocation now, a global shortcut later — reuses and
    // resets it rather than stacking up windows.
    let launcher: Rc<RefCell<Option<Rc<ui::Launcher>>>> = Rc::new(RefCell::new(None));
    app.connect_activate(move |app| {
        let existing = launcher.borrow().clone();
        let window = match existing {
            Some(window) => window,
            None => {
                let built = ui::Launcher::build(app);
                *launcher.borrow_mut() = Some(built.clone());
                built
            }
        };
        window.present();
    });

    app.run()
}
