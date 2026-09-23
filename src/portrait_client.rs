//! Portraits: match each file to a Net2 user by its name, let the operator
//! check the matches, then upload. Each upload re-reads the user first, so a
//! record that changed since it was checked is not written over.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{File, FileReader, HtmlButtonElement, HtmlInputElement, Url};

use crate::{
    client::{element, on_click},
    net2::{User, check_portrait, duplicate_flags, portrait_user_id, read_user},
    net2_client::call,
    reader::{State, Tool},
};

/// Enough of each file for its format and dimensions.
const HEADER_BYTES: i32 = 65_536;
/// Space between Net2 requests, which rate limits bursts.
const PACE_MS: i32 = 600;

struct Row {
    filename: String,
    id: Option<i32>,
    /// Read again at upload, so a large batch is not held in memory.
    file: File,
    preview: Option<String>,
    user: Option<User>,
    status: String,
    problem: bool,
    done: bool,
    /// Net2 may have saved it; checking it there comes before any retry.
    unsure: bool,
}

impl Row {
    fn pending(&self) -> bool {
        !self.problem && !self.done && !self.unsure
    }

    fn ready(&self, replace: bool) -> bool {
        self.pending() && self.user.as_ref().is_some_and(|u| replace || !u.has_image)
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.problem = true;
        self.status = message.into();
    }
}

thread_local! {
    static ROWS: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static STOP: Cell<bool> = const { Cell::new(false) };
}

fn replace() -> bool {
    element("portrait-replace")
        .unchecked_into::<HtmlInputElement>()
        .checked()
}

fn button(id: &str) -> HtmlButtonElement {
    element(id).unchecked_into()
}

fn notice(text: &str) {
    element("portrait-notice").set_text_content(Some(text));
}

/// A new operator may be a different person, so every match is checked again.
pub fn forget_matches() {
    ROWS.with_borrow_mut(|rows| {
        for row in rows.iter_mut().filter(|row| row.pending()) {
            row.user = None;
            row.status = "Not checked".into();
        }
    });
}

pub fn render(state: &State) {
    element("portrait-tool").set_hidden(state.tool != Tool::Portraits);
    if state.tool != Tool::Portraits {
        return;
    }
    let busy = BUSY.get();
    let connected = state.net2.is_some();
    let replace = replace();
    ROWS.with_borrow(|rows| {
        let ready = rows.iter().filter(|row| row.ready(replace)).count();
        button("portrait-review")
            .set_disabled(busy || !connected || !rows.iter().any(Row::pending));
        let upload = button("portrait-upload");
        upload.set_disabled(busy || !connected || ready == 0);
        upload.set_text_content(Some(&match ready {
            0 => "Upload portraits".to_string(),
            1 => "Upload 1 portrait".to_string(),
            n => format!("Upload {n} portraits"),
        }));
        element("portrait-list").set_hidden(rows.is_empty());
        let mut node = element("portrait-rows").first_element_child();
        for row in rows {
            let Some(tr) = node else { break };
            let cells: Vec<_> =
                std::iter::successors(tr.first_element_child(), |cell| cell.next_element_sibling())
                    .collect();
            let user = row.user.as_ref();
            for (index, text) in [
                (2, user.map_or("—".into(), User::name)),
                (
                    3,
                    user.map_or("—", |u| if u.has_image { "Yes" } else { "No" })
                        .into(),
                ),
                (4, row.status.clone()),
            ] {
                let cell = &cells[index];
                if cell.text_content().as_deref() != Some(&text) {
                    cell.set_text_content(Some(&text));
                }
            }
            cells[4].set_class_name(if row.problem || row.unsure {
                "problem"
            } else if row.done {
                "done"
            } else {
                ""
            });
            node = tr.next_element_sibling();
        }
    });
    element("portrait-files")
        .unchecked_into::<HtmlInputElement>()
        .set_disabled(busy);
    element("portrait-replace")
        .unchecked_into::<HtmlInputElement>()
        .set_disabled(busy);
    button("portrait-stop").set_hidden(!busy);
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    let _ = JsFuture::from(promise).await;
}

async fn header(file: &File) -> Option<Vec<u8>> {
    let end = (file.size() as i32).min(HEADER_BYTES);
    let blob = file.slice_with_i32_and_i32(0, end).ok()?;
    let buffer = JsFuture::from(blob.array_buffer()).await.ok()?;
    Some(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// The browser's own base64, by way of a data URL.
async fn base64(file: &File) -> Option<String> {
    let reader = FileReader::new().ok()?;
    let loaded = js_sys::Promise::new(&mut |resolve, reject| {
        reader.set_onload(Some(&resolve));
        reader.set_onerror(Some(&reject));
    });
    reader.read_as_data_url(file).ok()?;
    JsFuture::from(loaded).await.ok()?;
    let url = reader.result().ok()?.as_string()?;
    Some(url.split_once(',')?.1.to_string())
}

async fn choose(state: Rc<RefCell<State>>) {
    let Some(list) = element("portrait-files")
        .unchecked_into::<HtmlInputElement>()
        .files()
    else {
        return;
    };
    let files: Vec<File> = (0..list.length()).filter_map(|i| list.get(i)).collect();
    BUSY.set(true);
    notice("Reading images…");
    ROWS.with_borrow_mut(|rows| {
        for url in rows.drain(..).filter_map(|row| row.preview) {
            let _ = Url::revoke_object_url(&url);
        }
    });
    let mut rows = Vec::with_capacity(files.len());
    for file in files {
        let filename = file.name();
        let mut row = Row {
            id: None,
            preview: None,
            user: None,
            status: "Not checked".into(),
            problem: false,
            done: false,
            unsure: false,
            filename,
            file,
        };
        match portrait_user_id(&row.filename) {
            Ok(id) => row.id = Some(id),
            Err(message) => row.fail(message),
        }
        if row.id.is_some() {
            match header(&row.file).await {
                Some(bytes) => {
                    match check_portrait(&row.filename, &bytes, row.file.size() as usize) {
                        Ok(()) => row.preview = Url::create_object_url_with_blob(&row.file).ok(),
                        Err(message) => row.fail(message),
                    }
                }
                None => row.fail("The image could not be read."),
            }
        }
        rows.push(row);
    }
    let ids: Vec<_> = rows.iter().map(|row| row.id).collect();
    for (row, duplicate) in rows.iter_mut().zip(duplicate_flags(&ids)) {
        if duplicate {
            row.fail("More than one file has this user ID.");
        }
    }
    // The fixed cells are built once; render updates the rest in place.
    let document = web_sys::window().unwrap().document().unwrap();
    let body = element("portrait-rows");
    body.set_text_content(None);
    for row in &rows {
        let tr = document.create_element("tr").unwrap();
        let first = document.create_element("td").unwrap();
        if let Some(url) = &row.preview {
            let image = document.create_element("img").unwrap();
            image.set_attribute("src", url).unwrap();
            image.set_attribute("alt", "").unwrap();
            first.append_child(&image).unwrap();
        }
        let name = document.create_element("span").unwrap();
        name.set_text_content(Some(&row.filename));
        first.append_child(&name).unwrap();
        tr.append_child(&first).unwrap();
        let id = document.create_element("td").unwrap();
        id.set_text_content(Some(&row.id.map_or("—".into(), |id| id.to_string())));
        tr.append_child(&id).unwrap();
        for _ in 0..3 {
            tr.append_child(&document.create_element("td").unwrap())
                .unwrap();
        }
        body.append_child(&tr).unwrap();
    }
    let invalid = rows.iter().filter(|row| row.problem).count();
    notice(&format!(
        "{} chosen, {invalid} with problems. Check matches before uploading.",
        rows.len()
    ));
    ROWS.set(rows);
    BUSY.set(false);
    crate::client::render(&state.borrow());
}

fn update(state: &Rc<RefCell<State>>, index: usize, f: impl FnOnce(&mut Row)) {
    ROWS.with_borrow_mut(|rows| f(&mut rows[index]));
    crate::client::render(&state.borrow());
}

async fn review(state: Rc<RefCell<State>>) {
    BUSY.set(true);
    STOP.set(false);
    let todo: Vec<(usize, i32)> = ROWS.with_borrow(|rows| {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| row.pending())
            .filter_map(|(index, row)| Some((index, row.id?)))
            .collect()
    });
    for (position, &(index, id)) in todo.iter().enumerate() {
        if STOP.get() {
            break;
        }
        notice(&format!("Checking {} of {}…", position + 1, todo.len()));
        let result = call(&state, "GET", &format!("/users/{id}"), None).await;
        let halt = result.as_ref().is_err_and(|problem| problem.halt);
        update(&state, index, |row| match result {
            Ok(body) => match read_user(&body, id) {
                Ok(user) => {
                    row.status = if user.has_image {
                        "Has a portrait: tick Replace to overwrite it".into()
                    } else {
                        "Ready".into()
                    };
                    row.user = Some(user);
                }
                Err(message) => row.status = message,
            },
            Err(problem) => {
                row.user = None;
                row.status = problem.message;
            }
        });
        if halt {
            break;
        }
        sleep(PACE_MS).await;
    }
    BUSY.set(false);
    notice("Checked. Make sure every name matches its photo before uploading.");
    crate::client::render(&state.borrow());
}

async fn upload(state: Rc<RefCell<State>>, replace: bool) {
    BUSY.set(true);
    STOP.set(false);
    let todo: Vec<(usize, i32, String, File)> = ROWS.with_borrow(|rows| {
        rows.iter()
            .enumerate()
            .filter(|(_, row)| row.ready(replace))
            .filter_map(|(index, row)| {
                Some((index, row.id?, row.user.as_ref()?.name(), row.file.clone()))
            })
            .collect()
    });
    let mut uploaded = 0;
    let mut halted = false;
    for (position, (index, id, checked_name, file)) in todo.iter().enumerate() {
        if STOP.get() {
            halted = true;
            break;
        }
        notice(&format!("Uploading {} of {}…", position + 1, todo.len()));
        let outcome = async {
            let data = base64(file).await.ok_or_else(|| {
                crate::net2::failure("This image could not be read.", false, false)
            })?;
            let body = call(&state, "GET", &format!("/users/{id}"), None).await?;
            let now = read_user(&body, *id)
                .map_err(|message| crate::net2::failure(message, false, false))?;
            if &now.name() != checked_name {
                return Err(crate::net2::failure(
                    "This person changed in Net2 since the check. Check matches again.",
                    false,
                    false,
                ));
            }
            if now.has_image && !replace {
                return Err(crate::net2::failure(
                    "This person now has a portrait. Check matches again.",
                    false,
                    false,
                ));
            }
            sleep(PACE_MS).await;
            let image = json!({ "base64Data": data });
            call(&state, "PUT", &format!("/users/{id}/image"), Some(&image)).await
        }
        .await;
        let stop = outcome
            .as_ref()
            .is_err_and(|problem| problem.halt || problem.uncertain);
        update(&state, *index, |row| match outcome {
            Ok(_) => {
                uploaded += 1;
                row.done = true;
                row.status = "Uploaded".into();
            }
            Err(problem) => {
                row.user = None;
                row.unsure = problem.uncertain;
                row.status = problem.message;
            }
        });
        if stop {
            halted = true;
            break;
        }
        sleep(PACE_MS).await;
    }
    BUSY.set(false);
    notice(&format!(
        "{} {uploaded} of {} uploaded. Check each result before trying again.",
        if halted { "Stopped." } else { "Done." },
        todo.len()
    ));
    crate::client::render(&state.borrow());
}

pub fn setup(state: Rc<RefCell<State>>) {
    let files_state = state.clone();
    let callback = Closure::<dyn FnMut()>::new(move || spawn_local(choose(files_state.clone())));
    element("portrait-files").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let replace_state = state.clone();
    let callback =
        Closure::<dyn FnMut()>::new(move || crate::client::render(&replace_state.borrow()));
    element("portrait-replace").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let review_state = state.clone();
    on_click("portrait-review", move || {
        spawn_local(review(review_state.clone()))
    });
    on_click("portrait-stop", || STOP.set(true));
    on_click("portrait-upload", move || {
        let replace = replace();
        let count = ROWS.with_borrow(|rows| rows.iter().filter(|row| row.ready(replace)).count());
        let origin = state
            .borrow()
            .net2
            .as_ref()
            .map(|session| session.origin.clone())
            .unwrap_or_default();
        let question = format!(
            "Upload {count} portrait(s) to {origin}?\n{}\nCheck that every name matches its photo.",
            if replace {
                "Existing portraits will be replaced."
            } else {
                "People who already have a portrait are skipped."
            }
        );
        if web_sys::window()
            .unwrap()
            .confirm_with_message(&question)
            .unwrap_or(false)
        {
            spawn_local(upload(state.clone(), replace));
        }
    });
}
