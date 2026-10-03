//! Add people: new Net2 users from a spreadsheet, each given their
//! department, access levels and portrait as they are created. Check reads
//! Net2 first and holds back anyone whose name it already has, unless the
//! operator says it is someone else; the downloaded sheet notes each new User
//! ID, so a re-run never adds them again.
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Blob, File, HtmlButtonElement, HtmlInputElement};

use crate::{
    client::{build_map, element, on_click, read_map},
    net2::{check_portrait, created_id, failure, read_custom_fields, read_named, read_users},
    net2_client::call,
    people::{
        Custom, DEPARTMENT, FIRST, MIDDLE, PHOTO, Person, SURNAME, USER_ID, fields, name_key,
    },
    portrait_client::{PACE_MS, base64, convert, header, sleep},
    reader::{State, Tool},
    sheet::{Sheet, write_csv},
};

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Unchecked,
    Ready,
    /// Not sent, for the reason in the row's text.
    Held,
    Added,
    /// Net2 may have added them; check there before running the sheet again.
    Unsure,
}

#[derive(Clone)]
struct Row {
    status: Status,
    text: String,
    person: Option<Person>,
    department: Option<(i32, String)>,
    access_levels: Vec<i32>,
    image: Option<Blob>,
    user_id: Option<i32>,
    /// Held only because Net2 or the sheet already has this name.
    same_name: bool,
    /// The operator says it is someone else: add them anyway.
    anyway: bool,
}

impl Row {
    fn ready(&self) -> bool {
        self.status == Status::Ready
            || (self.status == Status::Held && self.same_name && self.anyway)
    }
}

struct Loaded {
    sheet: Sheet,
    filename: String,
    custom: Vec<Custom>,
    rows: Vec<Row>,
}

thread_local! {
    static LOADED: RefCell<Option<Loaded>> = const { RefCell::new(None) };
    static PHOTOS: RefCell<Vec<File>> = const { RefCell::new(Vec::new()) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static STOP: Cell<bool> = const { Cell::new(false) };
}

fn input(id: &str) -> HtmlInputElement {
    element(id).unchecked_into()
}

fn button(id: &str) -> HtmlButtonElement {
    element(id).unchecked_into()
}

fn notice(text: &str) {
    element("people-notice").set_text_content(Some(text));
}

/// Every row that was not sent needs checking again.
fn uncheck() {
    LOADED.with_borrow_mut(|loaded| {
        for row in loaded.iter_mut().flat_map(|l| &mut l.rows) {
            if matches!(row.status, Status::Ready | Status::Held) {
                *row = unchecked();
            }
        }
    });
}

fn unchecked() -> Row {
    Row {
        status: Status::Unchecked,
        text: "Not checked".into(),
        person: None,
        department: None,
        access_levels: Vec::new(),
        image: None,
        user_id: None,
        same_name: false,
        anyway: false,
    }
}

/// The row as the picked columns read it, before any check.
fn preview(sheet_row: &[String], map: &[Option<usize>]) -> [String; 3] {
    let get = |field: usize| {
        map.get(field)
            .copied()
            .flatten()
            .and_then(|column| sheet_row.get(column))
            .map_or("", |text| text.trim())
    };
    let name = [get(FIRST), get(MIDDLE), get(SURNAME)]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    [name, get(DEPARTMENT).into(), get(PHOTO).into()]
}

pub fn render(state: &State) {
    element("people-tool").set_hidden(state.tool != Tool::People);
    if state.tool != Tool::People {
        return;
    }
    let busy = BUSY.get();
    let connected = state.net2.is_some();
    input("people-file").set_disabled(busy || !connected);
    input("people-photos").set_disabled(busy);
    button("people-stop").set_hidden(!busy);
    let map = read_map("people-map");
    LOADED.with_borrow(|loaded| {
        element("people-columns").set_hidden(loaded.is_none());
        element("people-list").set_hidden(loaded.is_none());
        let rows = loaded.as_ref().map_or(&[][..], |l| &l.rows);
        let ready = rows.iter().filter(|r| r.ready()).count();
        let added = rows.iter().any(|r| r.user_id.is_some());
        button("people-check").set_disabled(busy || !connected || rows.is_empty());
        let create = button("people-create");
        create.set_disabled(busy || !connected || ready == 0);
        create.set_text_content(Some(&match ready {
            0 => "Add people to Net2".to_string(),
            1 => "Add 1 person to Net2".to_string(),
            n => format!("Add {n} people to Net2"),
        }));
        button("people-download").set_disabled(busy || !added);
        let Some(loaded) = loaded else { return };
        let mut node = element("people-rows").first_element_child();
        for (index, row) in rows.iter().enumerate() {
            let Some(tr) = node else { break };
            let cells: Vec<_> =
                std::iter::successors(tr.first_element_child(), |cell| cell.next_element_sibling())
                    .collect();
            let [name, department, photo] = preview(&loaded.sheet.rows[index], &map);
            let result = cells[4].first_element_child().unwrap();
            for (cell, text) in [&cells[1], &cells[2], &cells[3], &result].into_iter().zip([
                name,
                department,
                photo,
                row.text.clone(),
            ]) {
                if cell.text_content().as_deref() != Some(&text) {
                    cell.set_text_content(Some(&text));
                }
            }
            let anyway: web_sys::HtmlElement =
                result.next_element_sibling().unwrap().unchecked_into();
            anyway.set_hidden(!(row.same_name && row.status == Status::Held) || busy);
            anyway
                .first_element_child()
                .unwrap()
                .unchecked_into::<HtmlInputElement>()
                .set_checked(row.anyway);
            cells[4].set_class_name(match row.status {
                Status::Held if row.anyway => "",
                Status::Held | Status::Unsure => "problem",
                Status::Added => "done",
                _ => "",
            });
            node = tr.next_element_sibling();
        }
    });
}

async fn choose_sheet(state: Rc<RefCell<State>>) {
    let Some(file) = input("people-file").files().and_then(|files| files.get(0)) else {
        return;
    };
    BUSY.set(true);
    notice("Reading the sheet…");
    crate::client::render(&state.borrow());
    let result = crate::batch_client::read_file(&file, "sheet").await;
    input("people-file").set_value("");
    let sheet = match result {
        Ok(sheet) if sheet.rows.is_empty() => Err("The sheet has headers but no people.".into()),
        other => other,
    };
    match sheet {
        Ok(sheet) => {
            // This site's names for its custom fields, so the right column goes
            // in each; Net2's defaults if it does not say.
            let custom = match call(&state, "GET", "/users/customfieldnames", None).await {
                Ok(reply) => read_custom_fields(&reply),
                Err(_) => crate::people::default_custom(),
            };
            build_map("people-map", &fields(&custom), &sheet.headers);
            let document = web_sys::window().unwrap().document().unwrap();
            let body = element("people-rows");
            body.set_text_content(None);
            for index in 0..sheet.rows.len() {
                let tr = document.create_element("tr").unwrap();
                for column in 0..5 {
                    let td = document.create_element("td").unwrap();
                    if column == 0 {
                        td.set_text_content(Some(&(index + 2).to_string()));
                    }
                    if column == 4 {
                        // The result, then the choice a name match leaves.
                        td.set_inner_html(&format!(
                            r#"<span></span><label class="checkbox anyway" hidden=""><input type="checkbox" data-row="{index}"><span>Add anyway: someone else</span></label>"#
                        ));
                    }
                    tr.append_child(&td).unwrap();
                }
                body.append_child(&tr).unwrap();
            }
            notice(&format!(
                "{} people in {}. Check the columns, then Check against Net2.",
                sheet.rows.len(),
                file.name()
            ));
            let rows = sheet.rows.iter().map(|_| unchecked()).collect();
            LOADED.set(Some(Loaded {
                sheet,
                filename: file.name(),
                custom,
                rows,
            }));
        }
        Err(message) => notice(&message),
    }
    BUSY.set(false);
    crate::client::render(&state.borrow());
}

/// The photo the sheet names, among those chosen, as a JPG Net2 shows.
async fn photo(name: &str) -> Result<Blob, String> {
    let file = PHOTOS
        .with_borrow(|photos| {
            photos
                .iter()
                .find(|file| file.name().eq_ignore_ascii_case(name))
                .cloned()
        })
        .ok_or_else(|| format!("No photo called {name} was chosen."))?;
    let as_is = match header(&file).await {
        Some(bytes) => check_portrait(&file.name(), &bytes, file.size() as usize).is_ok(),
        None => false,
    };
    if as_is {
        Ok(file.into())
    } else {
        convert(&file).await.map(|(image, _)| image)
    }
}

async fn check(state: Rc<RefCell<State>>) {
    BUSY.set(true);
    uncheck();
    notice("Reading Net2’s people, departments and access levels…");
    crate::client::render(&state.borrow());
    let net2 = async {
        let departments = read_named(&call(&state, "GET", "/departments", None).await?);
        let levels = read_named(&call(&state, "GET", "/accesslevels", None).await?);
        let users = read_users(&call(&state, "GET", "/users", None).await?)
            .map_err(|message| failure(message, false, false))?;
        Ok::<_, crate::net2::Failure>((departments, levels, users))
    };
    let (departments, levels, users) = match net2.await {
        Ok(found) => found,
        Err(problem) => {
            BUSY.set(false);
            notice(&format!("Not checked. {}", problem.message));
            return crate::client::render(&state.borrow());
        }
    };
    let mut known: HashMap<String, String> = users
        .iter()
        .map(|user| {
            (
                name_key(&user.first, &user.last),
                format!("user {} in Net2", user.id),
            )
        })
        .collect();
    let map = read_map("people-map");
    let Some((sheet_rows, custom)) =
        LOADED.with_borrow(|l| l.as_ref().map(|l| (l.sheet.rows.clone(), l.custom.clone())))
    else {
        BUSY.set(false);
        return;
    };
    for (index, cells) in sheet_rows.iter().enumerate() {
        if LOADED.with_borrow(|l| l.as_ref().unwrap().rows[index].status) != Status::Unchecked {
            continue;
        }
        notice(&format!("Checking {} of {}…", index + 1, sheet_rows.len()));
        let mut row = unchecked();
        let outcome = async {
            let person = Person::read(cells, &map, &custom)?;
            if !person.user_id.is_empty() {
                return Err(format!("Already added: user {}.", person.user_id));
            }
            for wanted in &person.access_levels {
                let (id, _) = levels
                    .iter()
                    .find(|(_, name)| name.eq_ignore_ascii_case(wanted))
                    .ok_or_else(|| format!("Net2 has no access level {wanted}."))?;
                row.access_levels.push(*id);
            }
            if !person.department.is_empty() {
                row.department = Some(
                    departments
                        .iter()
                        .find(|(_, name)| name.eq_ignore_ascii_case(&person.department))
                        .cloned()
                        .ok_or_else(|| format!("Net2 has no department {}.", person.department))?,
                );
            }
            if !person.photo.is_empty() {
                row.image = Some(photo(&person.photo).await?);
            }
            Ok(person)
        };
        match outcome.await {
            // Otherwise ready, so the operator can say it is someone else.
            Ok(person) => {
                let key = name_key(&person.first, &person.last);
                match known.get(&key) {
                    Some(holder) => {
                        row.status = Status::Held;
                        row.same_name = true;
                        row.text = format!("Same name as {holder}.");
                    }
                    None => {
                        row.status = Status::Ready;
                        row.text = "Ready".into();
                        known.insert(key, format!("row {}", index + 2));
                    }
                }
                row.person = Some(person);
            }
            Err(message) => {
                row.status = Status::Held;
                row.text = message;
            }
        }
        LOADED.with_borrow_mut(|l| l.as_mut().unwrap().rows[index] = row);
    }
    let (ready, held) = LOADED.with_borrow(|l| {
        let rows = &l.as_ref().unwrap().rows;
        let count = |status| rows.iter().filter(|r| r.status == status).count();
        (count(Status::Ready), count(Status::Held))
    });
    BUSY.set(false);
    notice(&format!(
        "Checked: {ready} ready, {held} held back. Check each name, department and photo before adding."
    ));
    crate::client::render(&state.borrow());
}

/// Add each ready person, then their department, access levels and
/// portrait. A failure after Net2 made the user still counts as added, with
/// what is missing said.
async fn create(state: Rc<RefCell<State>>) {
    BUSY.set(true);
    STOP.set(false);
    let todo: Vec<(usize, Row)> = LOADED.with_borrow(|l| {
        let rows = l.as_ref().unwrap().rows.iter().cloned().enumerate();
        rows.filter(|(_, row)| row.ready() && row.person.is_some())
            .collect()
    });
    let mut added = 0;
    let mut halted = false;
    for (position, (index, row)) in todo.iter().enumerate() {
        let person = row.person.as_ref().unwrap();
        if STOP.get() {
            halted = true;
            break;
        }
        notice(&format!("Adding {} of {}…", position + 1, todo.len()));
        let reply = call(&state, "POST", "/users", Some(&person.body())).await;
        let (status, mut text, mut stop) =
            match reply.map(|body| created_id(&body, &person.first, &person.last)) {
                Ok(Ok(id)) => {
                    added += 1;
                    LOADED.with_borrow_mut(|l| l.as_mut().unwrap().rows[*index].user_id = Some(id));
                    (Status::Added, format!("Added as user {id}."), false)
                }
                // Net2 said yes, so someone was probably added; find out who first.
                Ok(Err(message)) => (
                    Status::Unsure,
                    format!("{message} Check Net2 for {}.", person.name()),
                    true,
                ),
                Err(problem) if problem.uncertain => (Status::Unsure, problem.message, true),
                Err(problem) => (
                    Status::Held,
                    format!("Not added. {}", problem.message),
                    problem.halt,
                ),
            };
        let id = LOADED.with_borrow(|l| l.as_ref().unwrap().rows[*index].user_id);
        if let Some(id) = id {
            if let Some((department_id, name)) = &row.department {
                sleep(PACE_MS).await;
                let body = json!({ "id": department_id, "name": name });
                let path = format!("/users/{id}/departments");
                if let Err(problem) = call(&state, "PUT", &path, Some(&body)).await {
                    text.push_str(&format!(" Not put in {name}: {}", problem.message));
                    stop |= problem.halt;
                }
            }
            if !row.access_levels.is_empty() {
                sleep(PACE_MS).await;
                // Replaces the set, which a new user has empty.
                let body =
                    json!({ "accessLevels": row.access_levels, "individualPermissions": [] });
                let path = format!("/users/{id}/doorpermissionset");
                if let Err(problem) = call(&state, "PUT", &path, Some(&body)).await {
                    text.push_str(&format!(" No access levels: {}", problem.message));
                    stop |= problem.halt;
                }
            }
            if let Some(image) = &row.image {
                sleep(PACE_MS).await;
                let sent = match base64(image).await {
                    Some(data) => {
                        let body = json!({ "base64Data": data });
                        call(&state, "PUT", &format!("/users/{id}/image"), Some(&body)).await
                    }
                    None => Err(failure("The photo could not be read.", false, false)),
                };
                if let Err(problem) = sent {
                    text.push_str(&format!(" No portrait: {}", problem.message));
                    stop |= problem.halt;
                }
            }
        }
        LOADED.with_borrow_mut(|l| {
            let row = &mut l.as_mut().unwrap().rows[*index];
            row.status = status;
            row.text = text;
        });
        crate::client::render(&state.borrow());
        if stop {
            halted = true;
            break;
        }
        sleep(PACE_MS).await;
    }
    BUSY.set(false);
    notice(&format!(
        "{} {added} of {} added. Download the sheet to keep their User IDs.",
        if halted { "Stopped." } else { "Done." },
        todo.len()
    ));
    crate::client::render(&state.borrow());
}

/// The sheet as chosen, with each new person's User ID filled in: in the
/// User ID column if one was picked, or a new one at the end.
fn download() -> Result<(), String> {
    let column = read_map("people-map").get(USER_ID).copied().flatten();
    LOADED.with_borrow(|loaded| {
        let Some(loaded) = loaded else { return Ok(()) };
        let mut headers = loaded.sheet.headers.clone();
        let column = column.unwrap_or_else(|| {
            headers.push("User ID".into());
            headers.len() - 1
        });
        let rows = loaded
            .sheet
            .rows
            .iter()
            .zip(&loaded.rows)
            .map(|(cells, row)| {
                let mut cells = cells.clone();
                cells.resize(cells.len().max(column + 1), String::new());
                if let Some(id) = row.user_id {
                    cells[column] = id.to_string();
                }
                cells
            });
        let bytes = write_csv(&headers, rows, loaded.sheet.bom)?;
        let name = &loaded.filename;
        let base = name
            .rsplit_once('.')
            .map_or(name.as_str(), |(base, _)| base);
        crate::batch_client::save(&bytes, &format!("{base}-net2.csv"))
    })
}

pub fn setup(state: Rc<RefCell<State>>) {
    let sheet_state = state.clone();
    let callback =
        Closure::<dyn FnMut()>::new(move || spawn_local(choose_sheet(sheet_state.clone())));
    element("people-file").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let photo_state = state.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        let list = input("people-photos").files();
        let files = list.map_or(Vec::new(), |list| {
            (0..list.length()).filter_map(|i| list.get(i)).collect()
        });
        notice(&format!(
            "{} photos chosen. Check against Net2 to match them.",
            files.len()
        ));
        PHOTOS.set(files);
        uncheck();
        crate::client::render(&photo_state.borrow());
    });
    element("people-photos").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let map_state = state.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        uncheck();
        crate::client::render(&map_state.borrow());
    });
    element("people-map").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let anyway_state = state.clone();
    let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(tick) = event
            .target()
            .map(|t| t.unchecked_into::<HtmlInputElement>())
        else {
            return;
        };
        let Some(index) = tick
            .get_attribute("data-row")
            .and_then(|i| i.parse::<usize>().ok())
        else {
            return;
        };
        LOADED.with_borrow_mut(|l| {
            if let Some(row) = l.as_mut().and_then(|l| l.rows.get_mut(index)) {
                row.anyway = tick.checked() && row.same_name;
            }
        });
        crate::client::render(&anyway_state.borrow());
    });
    element("people-rows").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let check_state = state.clone();
    on_click("people-check", move || {
        spawn_local(check(check_state.clone()))
    });
    on_click("people-stop", || STOP.set(true));
    on_click("people-download", || {
        if let Err(message) = download() {
            notice(&message);
        }
    });
    on_click("people-create", move || {
        let (count, twins) = LOADED.with_borrow(|l| {
            let rows = l.as_ref().map_or(&[][..], |l| &l.rows);
            let ready = rows.iter().filter(|r| r.ready());
            (ready.clone().count(), ready.filter(|r| r.same_name).count())
        });
        let origin = state
            .borrow()
            .net2
            .as_ref()
            .map(|session| session.origin.clone())
            .unwrap_or_default();
        let twins = match twins {
            0 => String::new(),
            n => format!("\n{n} of them share a name with someone already there."),
        };
        let question = format!(
            "Add {count} new people to {origin}?{twins}\nEach is given their department, access levels and portrait. Net2 cannot undo this in bulk."
        );
        if web_sys::window()
            .unwrap()
            .confirm_with_message(&question)
            .unwrap_or(false)
        {
            spawn_local(create(state.clone()));
        }
    });
}
