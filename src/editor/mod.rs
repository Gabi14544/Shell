mod language;

use crate::{
    extensions::Extension,
    lsp::{Edit, Position, Server},
    message,
};
use gtk::{gdk, gio, glib, pango, prelude::*};
use serde_json::Value;
use sourceview4::prelude::{
    BufferExt, LanguageExt, LanguageManagerExt, StyleSchemeManagerExt, ViewExt,
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

const MAX_FILE: u64 = 2 * 1024 * 1024;
const MAX_LSP_BYTES: usize = 512 * 1024;

#[derive(Default)]
struct Document {
    path: Option<PathBuf>,
    etag: Option<glib::GString>,
    crlf: bool,
    bom: bool,
}

#[derive(Clone, Copy)]
enum RequestKind {
    Completion { explicit: bool },
    Actions,
}

#[derive(Clone, Copy)]
struct Request {
    id: u64,
    kind: RequestKind,
    revision: i64,
    position: Position,
}

struct Choice {
    label: String,
    detail: String,
    edits: Vec<Edit>,
}

pub struct Editor {
    pub page: gtk::Box,
    pub view: sourceview4::View,
    window: glib::WeakRef<gtk::ApplicationWindow>,
    buffer: sourceview4::Buffer,
    filename: gtk::Label,
    position: gtk::Label,
    status: gtk::Label,
    diagnostic_button: gtk::Button,
    diagnostic_tag: gtk::TextTag,
    popover: gtk::Popover,
    list: gtk::ListBox,
    choices: RefCell<Vec<Choice>>,
    choice_request: Cell<Option<Request>>,
    extensions: Rc<RefCell<Vec<Extension>>>,
    document: RefCell<Document>,
    loading: Cell<bool>,
    active: Cell<bool>,
    revision: Cell<i64>,
    generation: Cell<u64>,
    next_id: Cell<u64>,
    server: RefCell<Option<Server>>,
    pending: Cell<Option<Request>>,
    waiting: Cell<Option<RequestKind>>,
    debounce: RefCell<Option<glib::SourceId>>,
    startup: RefCell<Option<glib::SourceId>>,
    diagnostics: RefCell<Vec<Value>>,
}

impl Editor {
    pub fn new(
        window: &gtk::ApplicationWindow,
        extensions: Rc<RefCell<Vec<Extension>>>,
    ) -> Rc<Self> {
        let buffer = sourceview4::Buffer::new(None::<&gtk::TextTagTable>);
        buffer.set_max_undo_levels(128);
        buffer.set_highlight_matching_brackets(true);
        if let Some(manager) = sourceview4::StyleSchemeManager::default() {
            if let Ok(executable) = std::env::current_exe() {
                if let Some(prefix) = executable.ancestors().nth(2) {
                    manager
                        .append_search_path(&prefix.join("share/brasa/styles").to_string_lossy());
                }
            }
            manager.append_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/styles"));
            if let Some(scheme) = manager
                .scheme("brasa")
                .or_else(|| manager.scheme("oblivion"))
            {
                buffer.set_style_scheme(Some(&scheme));
            }
        }
        let diagnostic_tag = gtk::TextTag::new(None);
        diagnostic_tag.set_underline(pango::Underline::Error);
        diagnostic_tag.set_underline_rgba(gdk::RGBA::parse("#ff8b80").ok().as_ref());
        buffer.tag_table().unwrap().add(&diagnostic_tag);
        let view = sourceview4::View::with_buffer(&buffer);
        view.style_context().add_class("source-editor");
        view.set_auto_indent(true);
        view.set_indent_on_tab(true);
        view.set_tab_width(4);
        view.set_insert_spaces_instead_of_tabs(true);
        view.set_show_line_numbers(true);
        view.set_highlight_current_line(true);
        view.set_smart_home_end(sourceview4::SmartHomeEndType::Before);
        view.set_left_margin(10);
        view.set_right_margin(10);
        view.set_top_margin(8);
        view.set_bottom_margin(8);
        view.set_monospace(true);

        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        toolbar.style_context().add_class("pane-toolbar");
        let new = gtk::Button::with_label("Nuevo");
        new.set_tooltip_text(Some("Nuevo archivo · Ctrl+N"));
        toolbar.pack_start(&new, false, false, 0);
        let open = gtk::Button::with_label("Abrir");
        open.set_tooltip_text(Some("Abrir archivo · Ctrl+O"));
        toolbar.pack_start(&open, false, false, 0);
        let save = gtk::Button::with_label("Guardar");
        save.set_tooltip_text(Some("Guardar archivo · Ctrl+S"));
        save.style_context().add_class("accent");
        toolbar.pack_start(&save, false, false, 0);
        let filename = gtk::Label::new(Some("Sin título"));
        filename.set_ellipsize(pango::EllipsizeMode::Middle);
        filename.style_context().add_class("filename");
        toolbar.pack_start(&filename, true, true, 5);
        let fixes = gtk::Button::with_label("Correcciones");
        fixes.set_tooltip_text(Some("Correcciones en el cursor · Ctrl+."));
        toolbar.pack_end(&fixes, false, false, 0);
        let suggest = gtk::Button::with_label("Sugerencias");
        suggest.set_tooltip_text(Some("Completar en el cursor · Ctrl+Espacio"));
        toolbar.pack_end(&suggest, false, false, 0);
        page.pack_start(&toolbar, false, false, 0);

        let scroller = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroller.add(&view);
        let frame = gtk::Frame::new(None);
        frame.style_context().add_class("workspace-frame");
        frame.add(&scroller);
        page.pack_start(&frame, true, true, 0);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.style_context().add_class("pane-footer");
        let status = gtk::Label::new(Some(
            "Guardá el archivo con su extensión para activar las sugerencias",
        ));
        status.set_xalign(0.0);
        status.set_ellipsize(pango::EllipsizeMode::End);
        footer.pack_start(&status, true, true, 0);
        let position = gtk::Label::new(Some("Ln 1, Col 1"));
        footer.pack_end(&position, false, false, 0);
        let diagnostic_button = gtk::Button::with_label("Avisos");
        diagnostic_button.style_context().add_class("diagnostics");
        diagnostic_button.set_no_show_all(true);
        footer.pack_end(&diagnostic_button, false, false, 0);
        page.pack_start(&footer, false, false, 0);

        let popover = gtk::Popover::new(Some(&view));
        popover.style_context().add_class("suggestions");
        popover.set_modal(false);
        popover.set_position(gtk::PositionType::Bottom);
        let items = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        items.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        items.set_propagate_natural_height(true);
        items.set_size_request(330, -1);
        items.set_max_content_height(225);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        items.add(&list);
        popover.add(&items);
        items.show_all();

        let editor = Rc::new(Self {
            page,
            view,
            buffer,
            window: window.downgrade(),
            filename,
            position,
            status,
            diagnostic_button,
            diagnostic_tag,
            popover,
            list,
            choices: RefCell::default(),
            choice_request: Cell::default(),
            extensions,
            document: RefCell::default(),
            loading: Cell::default(),
            active: Cell::default(),
            revision: Cell::default(),
            generation: Cell::default(),
            next_id: Cell::new(1),
            server: RefCell::default(),
            pending: Cell::default(),
            waiting: Cell::default(),
            debounce: RefCell::default(),
            startup: RefCell::default(),
            diagnostics: RefCell::default(),
        });
        new.connect_clicked(glib::clone!(@weak editor => move |_| editor.new_file()));
        open.connect_clicked(glib::clone!(@weak editor => move |_| editor.choose_open()));
        save.connect_clicked(glib::clone!(@weak editor => move |_| { editor.save(); }));
        fixes.connect_clicked(
            glib::clone!(@weak editor => move |_| editor.request(RequestKind::Actions)),
        );
        suggest.connect_clicked(glib::clone!(@weak editor => move |_| {
            editor.request(RequestKind::Completion { explicit: true });
        }));
        editor
            .diagnostic_button
            .connect_clicked(glib::clone!(@weak editor => move |_| {
                editor.show_diagnostics();
            }));
        editor
            .buffer
            .connect_changed(glib::clone!(@weak editor => move |_| editor.changed()));
        editor
            .buffer
            .connect_modified_changed(glib::clone!(@weak editor => move |_| {
                editor.update_filename();
            }));
        editor.buffer.connect_mark_set(glib::clone!(@weak editor => move |_, iter, mark| {
            if mark.name().as_deref() == Some("insert") {
                editor.position.set_text(&format!("Ln {}, Col {}", iter.line() + 1, iter.line_offset() + 1));
                if editor.choice_request.get().is_some_and(|request| request.position != editor.cursor_position()) {
                    editor.hide_choices();
                }
            }
        }));
        editor
            .view
            .connect_key_press_event(glib::clone!(@weak editor =>
            @default-return glib::Propagation::Proceed, move |_, event| editor.key(event)));
        editor
            .list
            .connect_row_activated(glib::clone!(@weak editor => move |_, row| {
                editor.accept_choice(row.index());
            }));
        editor
    }

    pub fn activate(self: &Rc<Self>, active: bool) {
        if self.active.replace(active) == active {
            return;
        }
        self.restart_server();
    }

    pub fn new_file(self: &Rc<Self>) {
        if self.can_discard() {
            self.set_document(Document::default(), "");
            self.view.grab_focus();
        }
    }

    pub fn can_discard(self: &Rc<Self>) -> bool {
        if !self.buffer.is_modified() {
            return true;
        }
        let window = self.window.upgrade();
        let dialog = gtk::MessageDialog::new(
            window.as_ref(),
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Question,
            gtk::ButtonsType::None,
            "¿Querés guardar los cambios?",
        );
        dialog.add_button("Cancelar", gtk::ResponseType::Cancel);
        dialog.add_button("Descartar", gtk::ResponseType::Reject);
        dialog.add_button("Guardar", gtk::ResponseType::Accept);
        dialog.set_default_response(gtk::ResponseType::Accept);
        let response = dialog.run();
        unsafe { dialog.destroy() };
        match response {
            gtk::ResponseType::Accept => self.save(),
            gtk::ResponseType::Reject => true,
            _ => false,
        }
    }

    pub fn choose_open(self: &Rc<Self>) {
        if let Some(path) = self.choose_file(false) {
            self.open(&path);
        }
    }

    pub fn open(self: &Rc<Self>, path: &Path) {
        if !self.can_discard() {
            return;
        }
        match read_document(path) {
            Ok((document, text)) => {
                self.set_document(document, &text);
                self.view.grab_focus();
            }
            Err(err) => self.error("No se pudo abrir el archivo", &err),
        }
    }

    fn choose_file(&self, save: bool) -> Option<PathBuf> {
        let window = self.window.upgrade();
        let (title, action, accept) = if save {
            ("Guardar archivo", gtk::FileChooserAction::Save, "Guardar")
        } else {
            ("Abrir archivo", gtk::FileChooserAction::Open, "Abrir")
        };
        let dialog = gtk::FileChooserNative::new(
            Some(title),
            window.as_ref(),
            action,
            Some(accept),
            Some("Cancelar"),
        );
        dialog.set_local_only(true);
        if let Some(dir) = self
            .document
            .borrow()
            .path
            .as_deref()
            .and_then(Path::parent)
        {
            dialog.set_current_folder(dir);
        }
        if save {
            dialog.set_do_overwrite_confirmation(true);
            dialog.set_current_name("sin-titulo.txt");
        }
        let path = (dialog.run() == gtk::ResponseType::Accept)
            .then(|| dialog.filename())
            .flatten();
        dialog.hide();
        path
    }

    pub fn save(self: &Rc<Self>) -> bool {
        let existing = self.document.borrow().path.clone();
        let Some(path) = existing.clone().or_else(|| self.choose_file(true)) else {
            return false;
        };
        let text = self.text();
        let result = {
            let document = self.document.borrow();
            let mut bytes = Vec::new();
            if document.bom {
                bytes.extend_from_slice(b"\xef\xbb\xbf");
            }
            if document.crlf {
                bytes.extend(text.replace('\n', "\r\n").bytes());
            } else {
                bytes.extend_from_slice(text.as_bytes());
            }
            gio::File::for_path(&path).replace_contents(
                &bytes,
                document.etag.as_deref(),
                false,
                gio::FileCreateFlags::NONE,
                gio::Cancellable::NONE,
            )
        };
        match result {
            Ok(etag) => {
                let mut document = self.document.borrow_mut();
                document.path = Some(path);
                document.etag = etag;
                drop(document);
                self.buffer.set_modified(false);
                self.update_filename();
                if existing.is_none() {
                    self.update_syntax();
                    self.restart_server();
                } else {
                    self.notify_save(&text);
                }
                true
            }
            Err(err) => {
                self.error("No se pudo guardar el archivo", err.message());
                false
            }
        }
    }

    fn set_document(self: &Rc<Self>, document: Document, text: &str) {
        self.stop_server();
        self.loading.set(true);
        *self.document.borrow_mut() = document;
        self.buffer.begin_not_undoable_action();
        self.buffer.set_text(text);
        self.buffer.place_cursor(&self.buffer.start_iter());
        self.buffer.end_not_undoable_action();
        self.buffer.set_modified(false);
        self.loading.set(false);
        self.revision.set(0);
        self.update_filename();
        self.update_syntax();
        self.restart_server();
    }

    fn update_syntax(&self) {
        let document = self.document.borrow();
        let language = document.path.as_ref().and_then(|path| {
            let manager = sourceview4::LanguageManager::default()?;
            if path
                .extension()
                .and_then(|suffix| suffix.to_str())
                .is_some_and(|suffix| {
                    suffix.eq_ignore_ascii_case("py") || suffix.eq_ignore_ascii_case("pyi")
                })
            {
                manager.language("python3")
            } else {
                manager.guess_language(Some(path), None)
            }
        });
        let indent = match language.as_ref().and_then(|lang| lang.id()) {
            Some(id)
                if ["js", "typescript", "json", "yaml", "html", "css"].contains(&id.as_str()) =>
            {
                2
            }
            _ => 4,
        };
        self.view.set_tab_width(indent);
        self.buffer.set_language(language.as_ref());
        self.buffer.set_highlight_syntax(language.is_some());
    }

    fn update_filename(&self) {
        let document = self.document.borrow();
        let name = document
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "Sin título".into());
        self.filename.set_text(&if self.buffer.is_modified() {
            format!("{name} •")
        } else {
            name.into()
        });
        self.filename
            .set_tooltip_text(document.path.as_ref().and_then(|path| path.to_str()));
    }

    fn text(&self) -> String {
        self.buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), true)
            .unwrap_or_default()
            .into()
    }

    fn cursor_iter(&self) -> gtk::TextIter {
        self.buffer.iter_at_mark(&self.buffer.get_insert().unwrap())
    }

    fn cursor_position(&self) -> Position {
        self.position_at(&self.cursor_iter())
    }

    fn position_at(&self, iter: &gtk::TextIter) -> Position {
        let line = self.buffer.iter_at_line(iter.line());
        let text = self.buffer.text(&line, iter, true).unwrap_or_default();
        Position {
            line: iter.line() as usize,
            character: text.encode_utf16().count(),
        }
    }

    fn changed(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        self.revision.set(self.revision.get() + 1);
        self.hide_choices();
        self.clear_diagnostics();
        if self.server.borrow().is_some() {
            self.cancel_debounce();
            let id = glib::timeout_add_local_once(
                std::time::Duration::from_millis(230),
                glib::clone!(@weak self as editor => move || {
                    editor.debounce.borrow_mut().take();
                    editor.sync_changed();
                }),
            );
            *self.debounce.borrow_mut() = Some(id);
        }
    }

    fn cancel_debounce(&self) {
        if let Some(id) = self.debounce.borrow_mut().take() {
            id.remove();
        }
    }

    fn key(self: &Rc<Self>, event: &gdk::EventKey) -> glib::Propagation {
        use gdk::keys::constants as key;
        if event.state().contains(gdk::ModifierType::CONTROL_MASK) {
            match event.keyval().to_lower() {
                key::space => self.request(RequestKind::Completion { explicit: true }),
                key::period => self.request(RequestKind::Actions),
                _ => return glib::Propagation::Proceed,
            }
            return glib::Propagation::Stop;
        }
        if self.choice_request.get().is_none() {
            return glib::Propagation::Proceed;
        }
        match event.keyval() {
            key::Escape => self.hide_choices(),
            key::Return | key::KP_Enter | key::Tab => {
                if let Some(row) = self.list.selected_row() {
                    self.accept_choice(row.index());
                }
            }
            key::Down => self.select_choice(1),
            key::Up => self.select_choice(-1),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    fn select_choice(&self, step: i32) {
        let max = self.choices.borrow().len().saturating_sub(1) as i32;
        let index = self.list.selected_row().map(|row| row.index()).unwrap_or(0);
        if let Some(row) = self.list.row_at_index((index + step).clamp(0, max)) {
            self.list.select_row(Some(&row));
            if let Some(scroller) = self
                .list
                .parent()
                .and_then(|widget| widget.parent())
                .and_downcast::<gtk::ScrolledWindow>()
            {
                let adjustment = scroller.vadjustment();
                let y = row.allocation().y() as f64;
                adjustment.clamp_page(y, y + f64::from(row.allocated_height()));
            }
        }
    }

    fn error(&self, title: &str, details: &str) {
        message(
            self.window.upgrade().as_ref(),
            gtk::MessageType::Error,
            title,
            details,
        );
    }
}

fn read_document(path: &Path) -> Result<(Document, String), String> {
    let path = path.canonicalize().map_err(|err| err.to_string())?;
    let info = std::fs::metadata(&path).map_err(|err| err.to_string())?;
    if !info.is_file() || info.len() > MAX_FILE {
        return Err("El editor admite archivos de texto locales de hasta 2 MiB.".into());
    }
    let file = gio::File::for_path(&path);
    let (bytes, etag) = file
        .load_contents(gio::Cancellable::NONE)
        .map_err(|err| err.to_string())?;
    if bytes.len() as u64 > MAX_FILE || bytes.contains(&0) {
        return Err("Elegí un archivo de texto UTF-8 de hasta 2 MiB.".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "Elegí un archivo de texto UTF-8.")?;
    let bom = text.starts_with('\u{feff}');
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let crlf = text.contains("\r\n");
    Ok((
        Document {
            path: Some(path),
            etag,
            crlf,
            bom,
        },
        text.replace("\r\n", "\n"),
    ))
}
