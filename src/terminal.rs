//! GTK 3 bindings for the subset of the VTE terminal API used here.
use crate::icon_button;
use glib::translate::{from_glib_borrow, from_glib_none, ToGlibPtr};
use gtk::{gdk, gio, glib, pango, prelude::*};
use std::{cell::Cell, ffi::CString, os::unix::ffi::OsStrExt, rc::Rc};

type TerminalPtr = *mut libc::c_void;
type SpawnCallback =
    unsafe extern "C" fn(TerminalPtr, glib::ffi::GPid, *mut glib::ffi::GError, glib::ffi::gpointer);

extern "C" {
    fn vte_terminal_new() -> *mut gtk::ffi::GtkWidget;
    fn vte_terminal_set_scrollback_lines(terminal: TerminalPtr, lines: libc::c_long);
    fn vte_terminal_set_colors(
        terminal: TerminalPtr,
        foreground: *const gdk::ffi::GdkRGBA,
        background: *const gdk::ffi::GdkRGBA,
        palette: *const gdk::ffi::GdkRGBA,
        palette_size: libc::size_t,
    );
    fn vte_terminal_set_font(terminal: TerminalPtr, font: *const pango::ffi::PangoFontDescription);
    fn vte_terminal_copy_clipboard_format(terminal: TerminalPtr, format: libc::c_int);
    fn vte_terminal_paste_clipboard(terminal: TerminalPtr);
    fn vte_terminal_spawn_async(
        terminal: TerminalPtr,
        pty_flags: libc::c_int,
        cwd: *const libc::c_char,
        argv: *mut *mut libc::c_char,
        envv: *mut *mut libc::c_char,
        spawn_flags: glib::ffi::GSpawnFlags,
        child_setup: glib::ffi::GSpawnChildSetupFunc,
        child_setup_data: glib::ffi::gpointer,
        child_setup_destroy: glib::ffi::GDestroyNotify,
        timeout: libc::c_int,
        cancellable: *mut gio::ffi::GCancellable,
        callback: Option<SpawnCallback>,
        user_data: glib::ffi::gpointer,
    );
}

fn ptr(widget: &gtk::Widget) -> TerminalPtr {
    widget.as_ptr().cast()
}

pub struct Terminal {
    pub page: gtk::Box,
    pub widget: gtk::Widget,
}

impl Terminal {
    pub fn new() -> Self {
        // VTE returns a floating GtkWidget. The GTK binding takes its own reference.
        let widget: gtk::Widget = unsafe { from_glib_none(vte_terminal_new()) };
        widget.style_context().add_class("terminal");
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_can_focus(true);
        let font = pango::FontDescription::from_string("Hack, DejaVu Sans Mono 10");
        let foreground = color("#e0e4e9");
        let background = color("#1b1e22");
        let palette = [
            "#252c34", "#f18682", "#a5d598", "#f5bf80", "#94b8fb", "#cbb0ec", "#8fd4d7", "#d9dfe7",
            "#8591a0", "#ffa4a0", "#c4eca8", "#ffe2a8", "#b4d0ff", "#e0c9ff", "#b6edf0", "#f7f8f9",
        ]
        .map(color_to_raw);
        unsafe {
            vte_terminal_set_font(ptr(&widget), font.to_glib_none().0);
            vte_terminal_set_scrollback_lines(ptr(&widget), 2_000);
            vte_terminal_set_colors(
                ptr(&widget),
                foreground.to_glib_none().0,
                background.to_glib_none().0,
                palette.as_ptr(),
                palette.len(),
            );
        }

        let shell = std::env::var_os("SHELL")
            .filter(|path| {
                std::path::Path::new(path).is_absolute() && std::path::Path::new(path).is_file()
            })
            .unwrap_or_else(|| "/bin/bash".into());
        let shell_name = std::path::Path::new(&shell)
            .file_name()
            .unwrap_or(shell.as_os_str())
            .to_string_lossy()
            .into_owned();

        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.style_context().add_class("terminal-pane");
        page.set_width_request(225);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.style_context().add_class("pane-toolbar");
        let title = gtk::Label::new(Some("Terminal"));
        title.style_context().add_class("pane-title");
        toolbar.pack_start(&title, false, false, 0);
        let restart = icon_button("view-refresh-symbolic", "Nueva sesión");
        restart.set_no_show_all(true);
        toolbar.pack_end(&restart, false, false, 0);
        let paste = icon_button("edit-paste-symbolic", "Pegar · Ctrl+Mayús+V");
        toolbar.pack_end(&paste, false, false, 0);
        let copy = icon_button("edit-copy-symbolic", "Copiar · Ctrl+Mayús+C");
        toolbar.pack_end(&copy, false, false, 0);
        page.pack_start(&toolbar, false, false, 0);

        let scroller = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_margin_start(12);
        scroller.set_margin_end(6);
        scroller.set_margin_top(9);
        scroller.set_margin_bottom(7);
        scroller.add(&widget);
        page.pack_start(&scroller, true, true, 0);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        footer.style_context().add_class("pane-footer");
        let dot = gtk::Label::new(Some("●"));
        dot.style_context().add_class("live-dot");
        footer.pack_start(&dot, false, false, 0);
        let status = gtk::Label::new(Some(&format!("Sesión local · {shell_name}")));
        status.set_ellipsize(pango::EllipsizeMode::End);
        footer.pack_start(&status, false, false, 0);
        let directory = gtk::Label::new(None);
        directory.set_xalign(1.0);
        directory.set_ellipsize(pango::EllipsizeMode::Middle);
        directory.style_context().add_class("terminal-directory");
        footer.pack_end(&directory, true, true, 0);
        page.pack_end(&footer, false, false, 0);
        widget.connect_notify_local(
            Some("current-directory-uri"),
            glib::clone!(@weak directory => move |terminal, _| {
                let uri = terminal.property::<Option<String>>("current-directory-uri");
                let path = uri.as_deref().and_then(|uri| gio::File::for_uri(uri).path());
                let path = path.as_deref().map(std::path::Path::to_string_lossy);
                let label = path.as_deref().or(uri.as_deref());
                directory.set_text(label.unwrap_or_default());
                directory.set_tooltip_text(label);
            }),
        );

        copy.connect_clicked(glib::clone!(@weak widget => move |_| unsafe {
            vte_terminal_copy_clipboard_format(ptr(&widget), 1);
        }));
        paste.connect_clicked(glib::clone!(@weak widget => move |_| unsafe {
            vte_terminal_paste_clipboard(ptr(&widget));
        }));
        widget.connect_key_press_event(|widget, event| {
            let modifiers = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
            if event.state().contains(modifiers) {
                unsafe {
                    match event.keyval().to_lower() {
                        gdk::keys::constants::c => {
                            vte_terminal_copy_clipboard_format(ptr(widget), 1)
                        }
                        gdk::keys::constants::v => vte_terminal_paste_clipboard(ptr(widget)),
                        _ => return glib::Propagation::Proceed,
                    }
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });

        let running = Rc::new(Cell::new(false));
        widget.connect_local("child-exited", false, glib::clone!(@weak status, @weak dot,
            @weak restart, @strong running => @default-return None, move |args| {
                running.set(false);
                let raw_status = args.get(1).and_then(|arg| arg.get::<i32>().ok()).unwrap_or(-1);
                let label = if libc::WIFEXITED(raw_status) {
                    format!("Sesión finalizada · código {}", libc::WEXITSTATUS(raw_status))
                } else {
                    "Sesión finalizada".into()
                };
                status.set_text(&label);
                dot.style_context().remove_class("live-dot");
                restart.show();
                None
        }));
        let start = Rc::new(
            glib::clone!(@weak widget, @weak status, @weak dot, @weak restart => move || {
                if running.replace(true) {
                    return;
                }
                status.set_text(&format!("Sesión local · {shell_name}"));
                status.set_tooltip_text(None);
                dot.style_context().add_class("live-dot");
                restart.hide();
                let shell = CString::new(shell.as_os_str().as_bytes()).expect("válido para el sistema");
                let mut argv = [shell.as_ptr().cast_mut(), std::ptr::null_mut()];
                let state = Box::new(SpawnState {
                    status: status.downgrade(),
                    dot: dot.downgrade(),
                    restart: restart.downgrade(),
                    running: running.clone(),
                });
                unsafe {
                    vte_terminal_spawn_async(
                        ptr(&widget), 0, std::ptr::null(), argv.as_mut_ptr(), std::ptr::null_mut(),
                        glib::ffi::G_SPAWN_DEFAULT, None, std::ptr::null_mut(), None, 5_000,
                        std::ptr::null_mut(), Some(spawned), Box::into_raw(state).cast(),
                    );
                }
            }),
        );
        restart.connect_clicked(glib::clone!(@weak widget, @strong start => move |_| {
            start();
            widget.grab_focus();
        }));
        glib::idle_add_local_once(move || start());
        Self { page, widget }
    }
}

struct SpawnState {
    status: glib::WeakRef<gtk::Label>,
    dot: glib::WeakRef<gtk::Label>,
    restart: glib::WeakRef<gtk::Button>,
    running: Rc<Cell<bool>>,
}

unsafe extern "C" fn spawned(
    _: TerminalPtr,
    _: glib::ffi::GPid,
    error: *mut glib::ffi::GError,
    data: glib::ffi::gpointer,
) {
    // VTE calls the callback on the GTK thread, including for failed spawns.
    let state = Box::from_raw(data.cast::<SpawnState>());
    if !error.is_null() {
        let error: glib::translate::Borrowed<glib::Error> = from_glib_borrow(error);
        state.running.set(false);
        if let Some(status) = state.status.upgrade() {
            status.set_text("No se pudo iniciar la sesión");
            status.set_tooltip_text(Some(error.message()));
        }
        if let Some(dot) = state.dot.upgrade() {
            dot.style_context().remove_class("live-dot");
        }
        if let Some(restart) = state.restart.upgrade() {
            restart.show();
        }
    }
}

fn color(value: &str) -> gdk::RGBA {
    gdk::RGBA::parse(value).expect("color de la paleta")
}

fn color_to_raw(value: &str) -> gdk::ffi::GdkRGBA {
    let color = color(value);
    gdk::ffi::GdkRGBA {
        red: color.red(),
        green: color.green(),
        blue: color.blue(),
        alpha: color.alpha(),
    }
}
