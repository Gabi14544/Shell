use crate::{extensions, icon_button, message};
use gtk::prelude::*;
use gtk::{gio, glib, pango};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

const ICON: u32 = 0;
const NAME: u32 = 1;
const URI: u32 = 2;
const KIND: u32 = 3;

const FILE: u32 = 0;
const UNLOADED_DIRECTORY: u32 = 1;
const DIRECTORY: u32 = 2;
const INFO: u32 = 3;

pub struct Explorer {
    pub page: gtk::Box,
    tree: gtk::TreeView,
    store: gtk::TreeStore,
    root: RefCell<PathBuf>,
    pinned: Cell<bool>,
}

impl Explorer {
    pub fn new(window: &gtk::ApplicationWindow) -> Rc<Self> {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.style_context().add_class("file-pane");
        page.set_width_request(160);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.style_context().add_class("pane-toolbar");
        let title = gtk::Label::new(Some("Archivos"));
        title.style_context().add_class("pane-title");
        toolbar.pack_start(&title, false, false, 0);
        let open = icon_button("folder-open-symbolic", "Abrir carpeta…");
        let reload = icon_button("view-refresh-symbolic", "Actualizar archivos");
        toolbar.pack_end(&reload, false, false, 0);
        toolbar.pack_end(&open, false, false, 0);
        page.pack_start(&toolbar, false, false, 0);

        let store = gtk::TreeStore::new(&[
            String::static_type(),
            String::static_type(),
            String::static_type(),
            u32::static_type(),
        ]);
        let tree = gtk::TreeView::with_model(&store);
        tree.style_context().add_class("project-tree");
        tree.set_headers_visible(false);
        tree.set_enable_search(true);
        tree.set_search_column(NAME as i32);
        tree.set_level_indentation(1);

        let column = gtk::TreeViewColumn::new();
        column.set_expand(true);
        column.set_spacing(6);
        let icon = gtk::CellRendererPixbuf::new();
        TreeViewColumnExt::pack_start(&column, &icon, false);
        TreeViewColumnExt::add_attribute(&column, &icon, "icon-name", ICON as i32);
        let name = gtk::CellRendererText::new();
        name.set_ellipsize(pango::EllipsizeMode::End);
        name.set_ypad(2);
        TreeViewColumnExt::pack_start(&column, &name, true);
        TreeViewColumnExt::add_attribute(&column, &name, "text", NAME as i32);
        tree.append_column(&column);

        let scroller = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.add(&tree);
        page.pack_start(&scroller, true, true, 0);

        let root = std::env::current_dir()
            .ok()
            .filter(|path| path.parent().is_some())
            .unwrap_or_else(glib::home_dir);
        let explorer = Rc::new(Self {
            page,
            tree,
            store,
            root: RefCell::new(root),
            pinned: Cell::new(false),
        });

        open.connect_clicked(glib::clone!(@weak explorer, @weak window => move |_| {
            explorer.choose_root(&window);
        }));
        reload.connect_clicked(glib::clone!(@weak explorer => move |_| explorer.reload()));
        explorer.tree.connect_row_expanded(
            glib::clone!(@weak explorer => move |_, folder, path| explorer.load(folder, path)),
        );
        explorer.reload();
        explorer
    }

    pub fn connect_open(&self, open: impl Fn(&Path) + 'static) {
        let store = self.store.clone();
        self.tree.connect_row_activated(move |tree, path, _| {
            let Some(row) = store.iter(path) else { return };
            match row_kind(&store, &row) {
                FILE => {
                    if let Some(path) = row_path(&store, &row) {
                        open(&path);
                    }
                }
                DIRECTORY | UNLOADED_DIRECTORY => {
                    if tree.row_expanded(path) {
                        tree.collapse_row(path);
                    } else {
                        tree.expand_row(path, false);
                    }
                }
                _ => {}
            }
        });
    }

    pub fn follow_file(&self, path: &Path) {
        if self.pinned.get() {
            return;
        }
        let current = self.root();
        if !path.starts_with(&current) || current == glib::home_dir() {
            let root = extensions::workspace(path);
            if root != current {
                self.root.replace(root);
                self.reload();
            }
        }
    }

    pub fn root(&self) -> PathBuf {
        self.root.borrow().clone()
    }

    fn choose_root(&self, window: &gtk::ApplicationWindow) {
        let dialog = gtk::FileChooserNative::new(
            Some("Abrir carpeta del proyecto"),
            Some(window),
            gtk::FileChooserAction::SelectFolder,
            Some("Abrir"),
            Some("Cancelar"),
        );
        dialog.set_local_only(true);
        dialog.set_current_folder(self.root());
        let result = (dialog.run() == gtk::ResponseType::Accept)
            .then(|| dialog.filename())
            .flatten();
        dialog.hide();
        if let Some(path) = result {
            match path.canonicalize() {
                Ok(path) => {
                    self.root.replace(path);
                    self.pinned.set(true);
                    self.reload();
                }
                Err(error) => message(
                    Some(window),
                    gtk::MessageType::Error,
                    "No se pudo abrir la carpeta",
                    &error.to_string(),
                ),
            }
        }
    }

    fn reload(&self) {
        self.store.clear();
        let root = self.root.borrow();
        let name = root
            .file_name()
            .unwrap_or(root.as_os_str())
            .to_string_lossy();
        let iter = self.append_entry(None, &root, &name, true);
        if let Some(path) = self.store.path(&iter) {
            self.tree.expand_row(&path, false);
        }
    }

    fn append_entry(
        &self,
        parent: Option<&gtk::TreeIter>,
        path: &Path,
        name: &str,
        directory: bool,
    ) -> gtk::TreeIter {
        let (icon, kind) = if directory {
            ("folder-symbolic", UNLOADED_DIRECTORY)
        } else {
            ("text-x-generic-symbolic", FILE)
        };
        let uri = gio::File::for_path(path).uri();
        let row = self.store.insert_with_values(
            parent,
            None,
            &[
                (ICON, &icon),
                (NAME, &name),
                (URI, &uri.as_str()),
                (KIND, &kind),
            ],
        );
        if directory {
            self.append_info(&row, "Cargando…");
        }
        row
    }

    fn append_info(&self, parent: &gtk::TreeIter, label: &str) {
        self.store
            .insert_with_values(Some(parent), None, &[(NAME, &label), (KIND, &INFO)]);
    }

    #[allow(deprecated)]
    fn load(self: &Rc<Self>, folder: &gtk::TreeIter, view_path: &gtk::TreePath) {
        if row_kind(&self.store, folder) != UNLOADED_DIRECTORY {
            return;
        }
        let Some(path) = row_path(&self.store, folder) else {
            return;
        };
        let Some(row) = gtk::TreeRowReference::new(&self.store, view_path) else {
            return;
        };
        self.store.set_value(folder, KIND, &DIRECTORY.to_value());
        let (sender, receiver) = glib::MainContext::channel(glib::Priority::DEFAULT);
        receiver.attach(
            None,
            glib::clone!(@weak self as explorer =>
                @default-return glib::ControlFlow::Break, move |entries| {
                    let Some(folder) = row.path().and_then(|path| explorer.store.iter(&path)) else {
                        return glib::ControlFlow::Break;
                    };
                    let loading = explorer.store.iter_children(Some(&folder));
                    match entries {
                        Ok(entries) => {
                            for Entry { path, name, directory, .. } in entries {
                                explorer.append_entry(Some(&folder), &path, &name, directory);
                            }
                        }
                        Err(_) => explorer.append_info(&folder, "Carpeta inaccesible"),
                    }
                    if let Some(loading) = loading {
                        explorer.store.remove(&loading);
                    }
                    glib::ControlFlow::Break
                }
            ),
        );
        std::thread::spawn(move || {
            let _ = sender.send(read_entries(&path));
        });
    }
}

struct Entry {
    path: PathBuf,
    name: String,
    sort_name: String,
    directory: bool,
}

fn read_entries(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries = fs::read_dir(path)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            Some(Entry {
                path: entry.path(),
                sort_name: name.to_lowercase(),
                name,
                directory: kind.is_dir(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.sort_name.cmp(&b.sort_name))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

fn row_kind(store: &gtk::TreeStore, row: &gtk::TreeIter) -> u32 {
    store.value(row, KIND as i32).get().unwrap_or(INFO)
}

fn row_path(store: &gtk::TreeStore, row: &gtk::TreeIter) -> Option<PathBuf> {
    let uri = store.value(row, URI as i32).get::<String>().ok()?;
    gio::File::for_uri(&uri).path()
}
