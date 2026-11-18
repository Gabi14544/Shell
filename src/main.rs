mod editor;
mod extensions;
mod lsp;
mod terminal;

use crate::{editor::Editor, extensions::Extension, terminal::Terminal};
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Page {
    Terminal,
    Editor,
}

struct ExtensionPanel {
    dialog: gtk::Dialog,
    list: gtk::Box,
}

struct App {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    terminal: Terminal,
    editor: RefCell<Option<Rc<Editor>>>,
    terminal_button: gtk::Button,
    editor_button: gtk::Button,
    page: Cell<Page>,
    extensions: Rc<RefCell<Vec<Extension>>>,
    extension_load_error: Option<String>,
    extension_window: RefCell<Option<ExtensionPanel>>,
    installing_python: Cell<bool>,
}

fn main() -> glib::ExitCode {
    let application = gtk::Application::new(
        Some("ar.brasa.Terminal"),
        gio::ApplicationFlags::HANDLES_OPEN,
    );
    let state: Rc<RefCell<Option<Rc<App>>>> = Rc::default();
    application.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        if let Err(err) = css.load_from_data(include_bytes!("theme.css")) {
            eprintln!("{err}");
        } else if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(true);
        }
    });
    application.connect_activate(glib::clone!(@strong state => move |application| {
        App::get(application, &state).window.present();
    }));
    application.connect_open(glib::clone!(@strong state => move |application, files, _| {
        let app = App::get(application, &state);
        app.window.present();
        if let Some(path) = files.first().and_then(|file| file.path()) {
            app.switch(Page::Editor);
            app.editor().open(&path);
        }
    }));
    application.connect_shutdown(move |_| {
        if let Some(app) = state.borrow_mut().take() {
            if let Some(editor) = app.editor.borrow().as_ref() {
                editor.activate(false);
            }
        }
    });
    application.run()
}

impl App {
    fn get(application: &gtk::Application, state: &RefCell<Option<Rc<Self>>>) -> Rc<Self> {
        if let Some(app) = state.borrow().as_ref() {
            return app.clone();
        }
        let app = Self::new(application);
        state.replace(Some(app.clone()));
        app
    }

    fn new(application: &gtk::Application) -> Rc<Self> {
        let window = gtk::ApplicationWindow::new(application);
        window.set_icon_name(Some("ar.brasa.Terminal"));
        window.set_default_size(1060, 620);
        window.set_size_request(635, 350);
        window.style_context().add_class("brasa");

        let header = gtk::HeaderBar::new();
        header.style_context().add_class("brasa-header");
        header.set_show_close_button(true);
        let brand = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let glyph = gtk::Label::new(Some(">_"));
        glyph.set_valign(gtk::Align::Center);
        glyph.style_context().add_class("brand-mark");
        let name = gtk::Label::new(Some("Brasa"));
        name.style_context().add_class("brand-name");
        brand.pack_start(&glyph, false, false, 0);
        brand.pack_start(&name, false, false, 0);
        header.pack_start(&brand);
        let terminal_button = gtk::Button::with_label("Terminal");
        terminal_button.set_tooltip_text(Some("Terminal · Ctrl+Mayús+T"));
        terminal_button.style_context().add_class("selected");
        let editor_button = gtk::Button::with_label("Editor");
        editor_button.set_tooltip_text(Some("Editor · Ctrl+Mayús+E"));
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        tabs.set_valign(gtk::Align::Center);
        tabs.style_context().add_class("mode-switch");
        tabs.add(&terminal_button);
        tabs.add(&editor_button);
        header.set_custom_title(Some(&tabs));
        let extension_button = gtk::Button::with_label("Extensiones");
        extension_button.set_image(Some(&gtk::Image::from_icon_name(
            Some("application-x-addon-symbolic"),
            gtk::IconSize::Button,
        )));
        extension_button.set_always_show_image(true);
        header.pack_end(&extension_button);
        window.set_titlebar(Some(&header));
        window.set_title("Brasa Terminal");

        let stage = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stage.style_context().add_class("stage");
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::None);
        stack.set_homogeneous(false);
        let terminal = Terminal::new();
        stack.add_named(&terminal.page, "terminal");
        stage.pack_start(&stack, true, true, 0);
        window.add(&stage);
        let (extensions, extension_load_error) = match extensions::load() {
            Ok(extensions) => (extensions, None),
            Err(err) => (Vec::new(), Some(err)),
        };
        let app = Rc::new(Self {
            window,
            stack,
            terminal,
            editor: RefCell::default(),
            terminal_button,
            editor_button,
            page: Cell::new(Page::Terminal),
            extensions: Rc::new(RefCell::new(extensions)),
            extension_load_error,
            extension_window: RefCell::default(),
            installing_python: Cell::default(),
        });
        app.terminal_button
            .connect_clicked(glib::clone!(@weak app => move |_| app.switch(Page::Terminal)));
        app.editor_button
            .connect_clicked(glib::clone!(@weak app => move |_| app.switch(Page::Editor)));
        extension_button.connect_clicked(glib::clone!(@weak app => move |_| app.show_extensions()));
        app.window.connect_key_press_event(glib::clone!(@weak app =>
            @default-return glib::Propagation::Proceed, move |_, event| app.key(event)));
        app.window.connect_delete_event(glib::clone!(@weak app =>
        @default-return glib::Propagation::Proceed, move |_, _| {
            if let Some(editor) = app.editor.borrow().as_ref() {
                if !editor.can_discard() {
                    return glib::Propagation::Stop;
                }
                editor.activate(false);
            }
            glib::Propagation::Proceed
        }));
        app.window.show_all();
        app.terminal.widget.grab_focus();
        app
    }

    fn editor(&self) -> Rc<Editor> {
        if let Some(editor) = self.editor.borrow().as_ref() {
            return editor.clone();
        }
        let editor = Editor::new(&self.window, self.extensions.clone());
        self.stack.add_named(&editor.page, "editor");
        editor.page.show_all();
        self.editor.replace(Some(editor.clone()));
        editor
    }

    fn switch(&self, page: Page) {
        let previous = self.page.replace(page);
        if previous != page {
            let (selected, other) = match page {
                Page::Terminal => (&self.terminal_button, &self.editor_button),
                Page::Editor => (&self.editor_button, &self.terminal_button),
            };
            selected.style_context().add_class("selected");
            other.style_context().remove_class("selected");
        }
        match page {
            Page::Terminal => {
                if let Some(editor) = self.editor.borrow().as_ref() {
                    editor.activate(false);
                }
                self.stack.set_visible_child_name("terminal");
                self.terminal.widget.grab_focus();
            }
            Page::Editor => {
                let editor = self.editor();
                self.stack.set_visible_child_name("editor");
                editor.activate(true);
                editor.view.grab_focus();
            }
        }
    }

    fn key(&self, event: &gdk::EventKey) -> glib::Propagation {
        use gdk::{keys::constants as key, ModifierType};
        let state = event.state();
        if !state.contains(ModifierType::CONTROL_MASK) || state.contains(ModifierType::MOD1_MASK) {
            return glib::Propagation::Proceed;
        }
        let key = event.keyval().to_lower();
        if state.contains(ModifierType::SHIFT_MASK) {
            match key {
                key::e => self.switch(Page::Editor),
                key::t => self.switch(Page::Terminal),
                _ => return glib::Propagation::Proceed,
            }
        } else if self.page.get() == Page::Editor {
            let editor = self.editor();
            match key {
                key::n => editor.new_file(),
                key::o => editor.choose_open(),
                key::s => {
                    editor.save();
                }
                _ => return glib::Propagation::Proceed,
            }
        } else {
            return glib::Propagation::Proceed;
        }
        glib::Propagation::Stop
    }

    fn show_extensions(self: &Rc<Self>) {
        if let Some(panel) = self.extension_window.borrow().as_ref() {
            panel.dialog.present();
            return;
        }
        if let Some(err) = &self.extension_load_error {
            message(
                Some(&self.window),
                gtk::MessageType::Error,
                "No se pudo leer extensiones.json",
                err,
            );
            return;
        }

        let dialog = gtk::Dialog::with_buttons(
            Some("Extensiones"),
            Some(&self.window),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[("Cerrar", gtk::ResponseType::Close)],
        );
        dialog.style_context().add_class("brasa");
        dialog.set_default_size(570, 400);
        let contents = gtk::Box::new(gtk::Orientation::Vertical, 9);
        contents.style_context().add_class("extension-panel");
        let scroller = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        scroller.add(&list);
        contents.pack_start(&scroller, true, true, 0);
        let custom = gtk::Button::with_label("Agregar servidor LSP…");
        custom.set_halign(gtk::Align::Start);
        contents.pack_start(&custom, false, false, 0);
        dialog.content_area().pack_start(&contents, true, true, 0);
        self.refresh_extensions(&list);
        custom.connect_clicked(glib::clone!(@weak self as app, @weak dialog => move |_| {
            app.add_custom(&dialog);
        }));
        dialog.connect_response(glib::clone!(@weak self as app => move |dialog, _| {
            app.extension_window.borrow_mut().take();
            unsafe { dialog.destroy() };
        }));
        *self.extension_window.borrow_mut() = Some(ExtensionPanel {
            dialog: dialog.clone(),
            list,
        });
        dialog.show_all();
        dialog.present();
    }

    fn refresh_extensions(self: &Rc<Self>, list: &gtk::Box) {
        for child in list.children() {
            list.remove(&child);
        }
        let enabled = self.extensions.borrow().clone();
        let mut all = extensions::catalog();
        all.extend(
            enabled
                .iter()
                .filter(|item| item.id.starts_with("local-"))
                .cloned(),
        );
        for extension in all {
            let card = gtk::Box::new(gtk::Orientation::Horizontal, 11);
            card.style_context().add_class("extension-card");
            let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
            let name = gtk::Label::new(Some(&extension.name));
            name.style_context().add_class("extension-name");
            name.set_xalign(0.0);
            let is_enabled = enabled.iter().any(|item| item.id == extension.id);
            let installing = extension.id == "python" && self.installing_python.get();
            let enabled_label = if is_enabled { "Activa · " } else { "" };
            let description = match extension.id.as_str() {
                "python" => "Sugerencias, avisos y correcciones con Ruff".to_string(),
                "rust" => "Requiere rust-analyzer instalado en el sistema".into(),
                "cpp" => "Requiere clangd instalado en el sistema".into(),
                _ => extension
                    .suffixes
                    .iter()
                    .map(|s| format!(".{s}"))
                    .collect::<Vec<_>>()
                    .join("  "),
            };
            let description = gtk::Label::new(Some(&format!("{enabled_label}{description}")));
            description
                .style_context()
                .add_class("extension-description");
            description.set_xalign(0.0);
            description.set_line_wrap(true);
            text.add(&name);
            text.add(&description);
            card.pack_start(&text, true, true, 0);
            let action = if installing {
                "Instalando…"
            } else if extension.id.starts_with("local-") {
                "Quitar"
            } else if is_enabled {
                "Desactivar"
            } else if extension.id == "python" && !extensions::python_ready() {
                "Instalar"
            } else {
                "Activar"
            };
            let button = gtk::Button::with_label(action);
            button.set_valign(gtk::Align::Center);
            button.set_sensitive(!installing);
            if action == "Instalar" {
                button.style_context().add_class("accent");
            }
            card.pack_end(&button, false, false, 0);
            list.pack_start(&card, false, false, 0);
            button.connect_clicked(glib::clone!(@weak self as app, @weak list => move |_| {
                if is_enabled {
                    app.update_extensions(|items| items.retain(|item| item.id != extension.id));
                } else if extension.id == "python" && !extensions::python_ready() {
                    app.install_python(extension.clone(), &list);
                } else if extension.id != "python" && glib::find_program_in_path(&extension.command[0]).is_none() {
                    let command = if extension.id == "rust" {
                        "rustup component add rust-analyzer\nUsá una toolchain con el componente rust-analyzer disponible."
                    } else {
                        "sudo apt install clangd"
                    };
                    app.extension_error("Instalá el servidor de lenguaje", command);
                } else {
                    app.update_extensions(|items| items.push(extension.clone()));
                }
            }));
        }
        list.show_all();
    }

    fn update_extensions(self: &Rc<Self>, change: impl FnOnce(&mut Vec<Extension>)) {
        let mut next = self.extensions.borrow().clone();
        change(&mut next);
        match extensions::save(&next) {
            Ok(()) => {
                self.extensions.replace(next);
                self.refresh_open_extensions();
                if let Some(editor) = self.editor.borrow().as_ref() {
                    editor.restart_server();
                }
            }
            Err(err) => self.extension_error("No se pudo actualizar la lista de extensiones", &err),
        }
    }

    #[allow(deprecated)]
    fn install_python(self: &Rc<Self>, extension: Extension, list: &gtk::Box) {
        if self.installing_python.replace(true) {
            return;
        }
        self.refresh_extensions(list);
        let (sender, receiver) = glib::MainContext::channel(glib::Priority::DEFAULT);
        receiver.attach(
            None,
            glib::clone!(@weak self as app =>
                @default-return glib::ControlFlow::Break, move |result: Result<(), String>| {
                    app.installing_python.set(false);
                    match result {
                        Ok(()) => app.update_extensions(|items| items.push(extension.clone())),
                        Err(err) => {
                            app.refresh_open_extensions();
                            app.extension_error("No se pudo instalar Python · Jedi y Ruff", &err);
                        }
                    }
                    glib::ControlFlow::Break
            }),
        );
        std::thread::spawn(move || {
            let _ = sender.send(extensions::install_python());
        });
    }

    fn add_custom(self: &Rc<Self>, parent: &gtk::Dialog) {
        let dialog = gtk::Dialog::with_buttons(
            Some("Agregar servidor LSP"),
            Some(parent),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[
                ("Cancelar", gtk::ResponseType::Cancel),
                ("Agregar", gtk::ResponseType::Accept),
            ],
        );
        dialog.style_context().add_class("brasa");
        dialog.set_default_size(505, -1);
        dialog.set_default_response(gtk::ResponseType::Accept);
        let form = gtk::Grid::new();
        form.set_row_spacing(10);
        form.set_column_spacing(12);
        form.set_margin_top(13);
        form.set_margin_bottom(13);
        form.set_margin_start(15);
        form.set_margin_end(15);
        let rows = [
            ("Nombre", "JavaScript · TypeScript"),
            ("ID del lenguaje", "javascript"),
            ("Extensiones", "js, mjs, cjs"),
            ("Comando", "typescript-language-server --stdio"),
        ];
        let inputs = rows
            .iter()
            .enumerate()
            .map(|(index, (label, hint))| {
                let entry = gtk::Entry::new();
                entry.set_placeholder_text(Some(hint));
                entry.set_activates_default(true);
                entry.set_hexpand(true);
                let label = gtk::Label::new(Some(label));
                label.set_xalign(0.0);
                form.attach(&label, 0, index as i32, 1, 1);
                form.attach(&entry, 1, index as i32, 1, 1);
                entry
            })
            .collect::<Vec<_>>();
        dialog.content_area().add(&form);
        dialog.show_all();
        while dialog.run() == gtk::ResponseType::Accept {
            match extensions::custom(
                &inputs[0].text(),
                &inputs[1].text(),
                &inputs[2].text(),
                &inputs[3].text(),
            ) {
                Ok(extension) => {
                    self.update_extensions(|items| items.push(extension));
                    break;
                }
                Err(err) => message(
                    Some(&dialog),
                    gtk::MessageType::Error,
                    "Revisá los datos del servidor",
                    &err,
                ),
            }
        }
        unsafe { dialog.destroy() };
    }

    fn refresh_open_extensions(self: &Rc<Self>) {
        if let Some(panel) = self.extension_window.borrow().as_ref() {
            self.refresh_extensions(&panel.list);
        }
    }

    fn extension_error(&self, title: &str, text: &str) {
        let dialog = self
            .extension_window
            .borrow()
            .as_ref()
            .map(|panel| panel.dialog.clone());
        let parent: &gtk::Window = if let Some(dialog) = &dialog {
            dialog.upcast_ref()
        } else {
            self.window.upcast_ref()
        };
        message(Some(parent), gtk::MessageType::Info, title, text);
    }
}

fn message(
    parent: Option<&impl IsA<gtk::Window>>,
    kind: gtk::MessageType,
    title: &str,
    details: &str,
) {
    let dialog = gtk::MessageDialog::new(
        parent,
        gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
        kind,
        gtk::ButtonsType::Close,
        title,
    );
    dialog.set_secondary_text(Some(details));
    dialog.run();
    unsafe { dialog.destroy() };
}
