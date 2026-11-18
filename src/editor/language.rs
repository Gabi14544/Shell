use super::{Choice, Editor, Request, RequestKind, MAX_LSP_BYTES};
use crate::{
    extensions,
    lsp::{self, Edit, Event, Position, Range, Server},
    message,
};
use gtk::{gdk, gio, glib, pango, prelude::*};
use serde_json::{json, Value};
use std::{rc::Rc, time::Duration};

impl Editor {
    #[allow(deprecated)]
    pub fn restart_server(self: &Rc<Self>) {
        self.stop_server();
        if !self.active.get() {
            return;
        }
        let Some(path) = self.document.borrow().path.clone() else {
            self.status
                .set_text("Guardá el archivo con su extensión para activar las sugerencias");
            return;
        };
        let language = self.language_label();
        let extension = self
            .extensions
            .borrow()
            .iter()
            .rev()
            .find(|extension| extension.matches(&path))
            .cloned();
        let Some(extension) = extension else {
            self.status
                .set_text(&format!("{language} · Sin extensión activa"));
            return;
        };
        if self.text().len() > MAX_LSP_BYTES {
            self.status
                .set_text(&format!("{language} · Límite para sugerencias: 512 KiB"));
            return;
        }
        let root = extensions::workspace(&path);
        let uri = gio::File::for_path(&root).uri().to_string();
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        let (sender, receiver) = glib::MainContext::channel(glib::Priority::DEFAULT);
        let server = match Server::start(&extension, &root, sender) {
            Ok(server) => server,
            Err(error) => {
                self.server_failed(&format!("{} · {error}", extension.name));
                return;
            }
        };
        let generation = self.generation.get();
        receiver.attach(None, glib::clone!(@weak self as editor => @default-return glib::ControlFlow::Break, move |event| {
            if editor.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            match event {
                Event::Message(value) => editor.receive(value),
                Event::Disconnected(error) => editor.server_failed(&error),
            }
            if editor.generation.get() == generation {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        }));
        self.status
            .set_text(&format!("Iniciando {}…", extension.name));
        server.request(0, "initialize", json!({
            "processId": std::process::id(),
            "clientInfo": {"name": "Brasa", "version": env!("CARGO_PKG_VERSION")},
            "rootUri": uri,
            "rootPath": root,
            "workspaceFolders": [{"uri": uri, "name": name}],
            "capabilities": {
                "general": {"positionEncodings": ["utf-16"]},
                "workspace": {"configuration": true, "workspaceFolders": true},
                "textDocument": {
                    "completion": {
                        "completionItem": {"snippetSupport": false, "insertReplaceSupport": true}
                    },
                    "codeAction": {
                        "codeActionLiteralSupport": {"codeActionKind": {"valueSet": ["quickfix"]}}
                    },
                    "publishDiagnostics": {"versionSupport": true}
                }
            }
        }));
        *self.server.borrow_mut() = Some(server);
        *self.startup.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_secs(25),
            glib::clone!(@weak self as editor => move || {
                editor.startup.borrow_mut().take();
                editor.server_failed("El servidor tardó más de 25 segundos en responder.");
            }),
        ));
    }

    pub(super) fn stop_server(&self) {
        self.generation.set(self.generation.get() + 1);
        self.cancel_debounce();
        self.cancel_startup();
        self.pending.set(None);
        self.waiting.set(None);
        self.hide_choices();
        self.clear_diagnostics();
        self.server.borrow_mut().take();
    }

    fn cancel_startup(&self) {
        if let Some(id) = self.startup.borrow_mut().take() {
            id.remove();
        }
    }

    fn language_label(&self) -> String {
        use sourceview4::prelude::{BufferExt, LanguageExt};
        self.buffer
            .language()
            .and_then(|language| language.name())
            .map(Into::into)
            .unwrap_or_else(|| "Texto".into())
    }

    fn uri(&self) -> Option<String> {
        self.document
            .borrow()
            .path
            .as_ref()
            .map(|path| gio::File::for_path(path).uri().to_string())
    }

    fn server_failed(&self, error: &str) {
        self.stop_server();
        self.status
            .set_text(&format!("Servidor detenido · {error}"));
    }

    fn initialized(self: &Rc<Self>, value: &Value) {
        self.cancel_startup();
        if let Some(error) = value["error"]["message"].as_str() {
            self.server_failed(error);
            return;
        }
        let capabilities = &value["result"]["capabilities"];
        if !capabilities.is_object() {
            self.server_failed("La respuesta de inicio del servidor no es válida.");
            return;
        }
        if capabilities["positionEncoding"]
            .as_str()
            .is_some_and(|value| value != "utf-16")
        {
            self.server_failed(
                "El servidor requiere una codificación de posiciones distinta de UTF-16.",
            );
            return;
        }
        let Some(uri) = self.uri() else { return };
        let Some(path) = self.document.borrow().path.clone() else {
            return;
        };
        let language_id = self
            .extensions
            .borrow()
            .iter()
            .rev()
            .find(|extension| extension.matches(&path))
            .map(|extension| extensions::language_id(extension, &path));
        let text = self.text();
        if text.len() > MAX_LSP_BYTES {
            self.server_failed("El archivo supera los 512 KiB.");
            return;
        }
        if let Some(server) = self.server.borrow_mut().as_mut() {
            server.capabilities(capabilities);
            server.notify("initialized", json!({}));
            if server.settings.is_object() {
                server.notify(
                    "workspace/didChangeConfiguration",
                    json!({"settings": server.settings}),
                );
            }
            if server.open_close {
                server.notify("textDocument/didOpen", json!({"textDocument": {
                    "uri": uri, "languageId": language_id, "version": self.revision.get(), "text": text
                }}));
            }
            server.synced_revision = self.revision.get();
            server.synced_text = text;
            self.status.set_text(&server.name);
        }
        if let Some(kind) = self.waiting.take() {
            self.request(kind);
        }
    }

    fn receive(self: &Rc<Self>, value: Value) {
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            if let Some(id) = value.get("id") {
                self.answer(method, id, &value["params"]);
            } else if method == "textDocument/publishDiagnostics" {
                self.receive_diagnostics(&value["params"]);
            }
            return;
        }
        let Some(id) = value["id"].as_u64() else {
            return;
        };
        if id == 0 {
            self.initialized(&value);
            return;
        }
        let Some(request) = self.pending.get().filter(|request| request.id == id) else {
            return;
        };
        self.pending.set(None);
        if request.revision != self.revision.get() || request.position != self.cursor_position() {
            return;
        }
        if let Some(error) = value["error"]["message"].as_str() {
            if matches!(
                request.kind,
                RequestKind::Actions | RequestKind::Completion { explicit: true }
            ) {
                self.status
                    .set_text(&format!("El servidor devolvió un error · {error}"));
            }
            return;
        }
        let result = &value["result"];
        let choices = match request.kind {
            RequestKind::Completion { .. } => self.completions(result, request.position),
            RequestKind::Actions => self.actions(result),
        };
        if choices.is_empty() {
            match request.kind {
                RequestKind::Completion { explicit: true } => {
                    self.status.set_text("No hay sugerencias en el cursor")
                }
                RequestKind::Actions => self
                    .status
                    .set_text("No hay correcciones de este archivo en el cursor"),
                RequestKind::Completion { explicit: false } => {}
            }
        } else {
            self.show_choices(choices, request);
        }
    }

    fn answer(&self, method: &str, id: &Value, params: &Value) {
        let guard = self.server.borrow();
        let Some(server) = guard.as_ref() else { return };
        let result = match method {
            "workspace/configuration" => {
                let items = params["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|item| {
                        let Some(section) =
                            item["section"].as_str().filter(|value| !value.is_empty())
                        else {
                            return server.settings.clone();
                        };
                        section
                            .split('.')
                            .fold(&server.settings, |value, part| &value[part])
                            .clone()
                    })
                    .collect::<Vec<_>>();
                json!(items)
            }
            "workspace/workspaceFolders" => self
                .document
                .borrow()
                .path
                .as_ref()
                .map(|path| {
                    let root = extensions::workspace(path);
                    json!([{
                        "uri": gio::File::for_path(&root).uri().as_str(),
                        "name": root.file_name().map(|name| name.to_string_lossy())
                    }])
                })
                .unwrap_or(Value::Null),
            "window/workDoneProgress/create" => Value::Null,
            "workspace/applyEdit" => {
                json!({"applied": false, "failureReason": "Las ediciones requieren una elección en Brasa."})
            }
            _ => {
                server.send(json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": -32601, "message": "Método no implementado"
                }}));
                return;
            }
        };
        server.send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    pub(super) fn notify_save(&self, text: &str) {
        if !self.sync() {
            return;
        }
        let guard = self.server.borrow();
        let Some(server) = guard.as_ref() else { return };
        if let Some(include_text) = server.save {
            let mut params = json!({"textDocument": {"uri": self.uri()}});
            if include_text {
                params["text"] = json!(text);
            }
            server.notify("textDocument/didSave", params);
        }
    }

    pub(super) fn sync_changed(self: &Rc<Self>) {
        if self.sync() && self.view.has_focus() && !self.buffer.has_selection() {
            let iter = self.cursor_iter();
            let line = self.buffer.iter_at_line(iter.line());
            let prefix = self.buffer.text(&line, &iter, true).unwrap_or_default();
            let mut tail = prefix.chars().rev();
            let complete = match tail.next() {
                Some('.' | ':' | '>') => true,
                Some(ch) if word(ch) => tail.next().is_some_and(word),
                _ => false,
            };
            if complete {
                self.request(RequestKind::Completion { explicit: false });
            }
        }
    }

    fn sync(&self) -> bool {
        let Some(uri) = self.uri() else { return false };
        let mut guard = self.server.borrow_mut();
        let Some(server) = guard.as_mut().filter(|server| server.ready) else {
            return false;
        };
        let revision = self.revision.get();
        if revision == server.synced_revision {
            return true;
        }
        let text = self.text();
        if text.len() > MAX_LSP_BYTES {
            drop(guard);
            self.server_failed("El archivo supera los 512 KiB.");
            return false;
        }
        if server.sync_kind != 0 {
            let mut change = json!({"text": text});
            if server.sync_kind == 2 {
                change["range"] = json!(Range {
                    start: Position::default(),
                    end: lsp::end_position(&server.synced_text),
                });
            }
            if !server.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": {"uri": uri, "version": revision},
                    "contentChanges": [change]
                }),
            ) {
                drop(guard);
                self.server_failed("No se pudo sincronizar el documento con el servidor.");
                return false;
            }
        }
        server.synced_text = text;
        server.synced_revision = revision;
        true
    }

    pub(super) fn request(self: &Rc<Self>, kind: RequestKind) {
        self.hide_choices();
        let explicit = !matches!(kind, RequestKind::Completion { explicit: false });
        let guard = self.server.borrow();
        let Some(server) = guard.as_ref() else {
            if explicit && self.document.borrow().path.is_none() {
                self.status
                    .set_text("Guardá el archivo con su extensión para activar las sugerencias");
            }
            return;
        };
        if !server.ready {
            if explicit {
                self.waiting.set(Some(kind));
            }
            return;
        }
        let capability = match kind {
            RequestKind::Completion { .. } => server.completion,
            RequestKind::Actions => server.actions,
        };
        if !capability {
            if explicit {
                self.status.set_text("El servidor no ofrece esa función");
            }
            return;
        }
        drop(guard);
        self.cancel_debounce();
        if !self.sync() {
            return;
        }

        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let position = self.cursor_position();
        let request = Request {
            id,
            kind,
            revision: self.revision.get(),
            position,
        };
        let document = json!({"uri": self.uri()});
        let (method, params) = match kind {
            RequestKind::Completion { .. } => (
                "textDocument/completion",
                json!({"textDocument": document, "position": position, "context": {"triggerKind": 1}}),
            ),
            RequestKind::Actions => {
                let selection = self.buffer.selection_bounds().map(|(start, end)| Range {
                    start: self.position_at(&start),
                    end: self.position_at(&end),
                });
                let range = selection.unwrap_or(Range {
                    start: position,
                    end: position,
                });
                let diagnostics = self.diagnostics.borrow();
                let nearby = diagnostics
                    .iter()
                    .filter(|item| {
                        serde_json::from_value::<Range>(item["range"].clone()).is_ok_and(|item| {
                            item.start.line <= range.end.line && item.end.line >= range.start.line
                        })
                    })
                    .collect::<Vec<_>>();
                let range = if selection.is_none() {
                    nearby
                        .first()
                        .and_then(|item| serde_json::from_value(item["range"].clone()).ok())
                        .unwrap_or(range)
                } else {
                    range
                };
                (
                    "textDocument/codeAction",
                    json!({
                        "textDocument": document, "range": range,
                        "context": {"diagnostics": nearby, "only": ["quickfix"]}
                    }),
                )
            }
        };

        let guard = self.server.borrow();
        let Some(server) = guard.as_ref() else { return };
        if let Some(prior) = self.pending.take() {
            server.notify("$/cancelRequest", json!({"id": prior.id}));
        }
        if server.request(id, method, params) {
            self.pending.set(Some(request));
        } else if explicit {
            self.status.set_text("Servidor ocupado · Volvé a intentar");
        }
    }

    fn completions(&self, result: &Value, position: Position) -> Vec<Choice> {
        let mut start = self.cursor_iter();
        let end = start;
        while start.line() == end.line() && start.line_offset() > 0 {
            let mut previous = start;
            if !previous.backward_char() || !previous.char().is_some_and(word) {
                break;
            }
            start = previous;
        }
        let fallback = Range {
            start: self.position_at(&start),
            end: position,
        };
        let defaults = &result["itemDefaults"];
        let default_range = parse_edit_range(&defaults["editRange"]);
        let items = result.as_array().or_else(|| result["items"].as_array());
        let mut items = items
            .into_iter()
            .flatten()
            .filter(|item| {
                item["insertTextFormat"]
                    .as_u64()
                    .or_else(|| defaults["insertTextFormat"].as_u64())
                    != Some(2)
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| completion_sort_key(a).cmp(completion_sort_key(b)));
        items
            .into_iter()
            .filter_map(|item| {
                let label = item["label"].as_str()?;
                let edit = &item["textEdit"];
                let range = parse_edit_range(&edit["range"])
                    .or_else(|| parse_edit_range(edit))
                    .or(default_range)
                    .unwrap_or(fallback);
                let text = edit["newText"]
                    .as_str()
                    .or_else(|| item["textEditText"].as_str())
                    .or_else(|| item["insertText"].as_str())
                    .unwrap_or(label);
                let mut edits = vec![Edit {
                    range,
                    new_text: text.into(),
                }];
                if let Some(additional) = item["additionalTextEdits"].as_array() {
                    let additional: Vec<Edit> = serde_json::from_value(json!(additional)).ok()?;
                    edits.extend(additional);
                }
                Some(Choice {
                    label: label.into(),
                    detail: item["detail"]
                        .as_str()
                        .unwrap_or_default()
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .into(),
                    edits,
                })
            })
            .take(40)
            .collect()
    }

    fn actions(&self, result: &Value) -> Vec<Choice> {
        let Some(uri) = self.uri() else {
            return Vec::new();
        };
        result
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| {
                if item["disabled"].is_object() || item["command"].is_object() {
                    return None;
                }
                let edits = workspace_edits(&item["edit"], uri.as_str(), self.revision.get())?;
                Some(Choice {
                    label: item["title"].as_str()?.into(),
                    detail: String::new(),
                    edits,
                })
            })
            .take(40)
            .collect()
    }

    fn show_choices(&self, choices: Vec<Choice>, request: Request) {
        let cursor = self.cursor_iter();
        let rect = self.view.iter_location(&cursor);
        let (x, y) =
            self.view
                .buffer_to_window_coords(gtk::TextWindowType::Widget, rect.x(), rect.y());
        self.popover.set_pointing_to(&gdk::Rectangle::new(
            x,
            y,
            rect.width().max(1),
            rect.height(),
        ));
        for child in self.list.children() {
            self.list.remove(&child);
        }
        for choice in &choices {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 1);
            row.style_context().add_class("choice");
            let label = gtk::Label::new(Some(&choice.label));
            label.set_xalign(0.0);
            label.set_ellipsize(pango::EllipsizeMode::End);
            label.set_max_width_chars(40);
            row.add(&label);
            if !choice.detail.is_empty() {
                let detail = gtk::Label::new(Some(&choice.detail));
                detail.style_context().add_class("choice-detail");
                detail.set_xalign(0.0);
                detail.set_ellipsize(pango::EllipsizeMode::End);
                detail.set_max_width_chars(40);
                row.add(&detail);
            }
            self.list.add(&row);
        }
        *self.choices.borrow_mut() = choices;
        self.choice_request.set(Some(request));
        self.view.grab_focus();
        self.list.show_all();
        self.list.select_row(self.list.row_at_index(0).as_ref());
        self.popover.popup();
    }

    pub(super) fn hide_choices(&self) {
        if self.choice_request.take().is_some() {
            self.popover.popdown();
            self.choices.borrow_mut().clear();
        }
    }

    pub(super) fn accept_choice(&self, index: i32) {
        let Some(request) = self.choice_request.get() else {
            return;
        };
        let edits = self
            .choices
            .borrow()
            .get(index as usize)
            .map(|choice| choice.edits.clone());
        self.hide_choices();
        let Some(edits) = edits.filter(|_| {
            request.revision == self.revision.get() && request.position == self.cursor_position()
        }) else {
            return;
        };
        let text = self.text();
        let resolved: Option<Vec<_>> = edits
            .into_iter()
            .map(|edit| {
                let start = lsp::text_offset(&text, edit.range.start)?;
                let end = lsp::text_offset(&text, edit.range.end)?;
                (start <= end && !edit.new_text.contains('\0')).then_some((
                    start,
                    end,
                    edit.new_text.replace("\r\n", "\n"),
                ))
            })
            .collect();
        let Some(mut resolved) = resolved else {
            self.status
                .set_text("La sugerencia contiene una posición inválida");
            return;
        };
        resolved.sort_by_key(|(start, end, _)| (*start, *end));
        if resolved
            .windows(2)
            .any(|adjacent| adjacent[0].1 > adjacent[1].0)
        {
            self.status
                .set_text("La sugerencia contiene cambios superpuestos");
            return;
        }
        self.buffer.begin_user_action();
        for (start, end, value) in resolved.into_iter().rev() {
            let mut start = self.buffer.iter_at_offset(start);
            let mut end = self.buffer.iter_at_offset(end);
            self.buffer.delete(&mut start, &mut end);
            self.buffer.insert(&mut start, &value);
        }
        self.buffer.end_user_action();
        self.view.grab_focus();
    }

    fn receive_diagnostics(&self, params: &Value) {
        let Some(uri) = self.uri() else { return };
        if params["uri"].as_str() != Some(uri.as_str())
            || params["version"]
                .as_i64()
                .is_some_and(|version| version != self.revision.get())
        {
            return;
        }
        let guard = self.server.borrow();
        let Some(server) = guard
            .as_ref()
            .filter(|server| server.synced_revision == self.revision.get())
        else {
            return;
        };
        self.clear_diagnostics();
        let mut diagnostics = self.diagnostics.borrow_mut();
        for item in params["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .take(100)
        {
            if let Ok(range) = serde_json::from_value::<Range>(item["range"].clone()) {
                let Some(start) = lsp::text_offset(&server.synced_text, range.start) else {
                    continue;
                };
                let Some(end) = lsp::text_offset(&server.synced_text, range.end) else {
                    continue;
                };
                if end > start {
                    self.buffer.apply_tag(
                        &self.diagnostic_tag,
                        &self.buffer.iter_at_offset(start),
                        &self.buffer.iter_at_offset(end),
                    );
                }
                diagnostics.push(item.clone());
            }
        }
        if !diagnostics.is_empty() {
            self.diagnostic_button
                .set_label(&format!("Avisos ({})", diagnostics.len()));
            self.diagnostic_button.show();
        }
    }

    pub(super) fn clear_diagnostics(&self) {
        let mut diagnostics = self.diagnostics.borrow_mut();
        if !diagnostics.is_empty() {
            diagnostics.clear();
            self.buffer.remove_tag(
                &self.diagnostic_tag,
                &self.buffer.start_iter(),
                &self.buffer.end_iter(),
            );
            self.diagnostic_button.hide();
        }
    }

    pub(super) fn show_diagnostics(&self) {
        let text = self
            .diagnostics
            .borrow()
            .iter()
            .take(12)
            .filter_map(|item| {
                let line = item["range"]["start"]["line"].as_u64()? + 1;
                let message = item["message"].as_str()?;
                Some(format!("Línea {line} · {message}"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        message(
            self.window.upgrade().as_ref(),
            gtk::MessageType::Info,
            "Avisos del editor",
            &text,
        );
    }
}

fn parse_edit_range(value: &Value) -> Option<Range> {
    serde_json::from_value(value["replace"].clone())
        .or_else(|_| serde_json::from_value(value.clone()))
        .ok()
}

fn word(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

fn workspace_edits(value: &Value, uri: &str, revision: i64) -> Option<Vec<Edit>> {
    if let Some(changes) = value["changes"].as_object() {
        if changes.len() == 1 {
            let edits: Vec<Edit> = serde_json::from_value(changes.get(uri)?.clone()).ok()?;
            return (!edits.is_empty()).then_some(edits);
        }
        return None;
    }
    let changes = value["documentChanges"].as_array()?;
    let [change] = changes.as_slice() else {
        return None;
    };
    let document = &change["textDocument"];
    if document["uri"].as_str()? != uri
        || document["version"]
            .as_i64()
            .is_some_and(|value| value != revision)
    {
        return None;
    }
    let edits: Vec<Edit> = serde_json::from_value(change["edits"].clone()).ok()?;
    (!edits.is_empty()).then_some(edits)
}

fn completion_sort_key(item: &Value) -> &str {
    item["sortText"]
        .as_str()
        .or_else(|| item["label"].as_str())
        .unwrap_or("")
}
