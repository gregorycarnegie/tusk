//! Manage: people already in Net2. Move the ticked ones to another
//! department, mark them as leavers, or move every year group up. Every
//! change is confirmed first and made one person at a time, so Stop leaves
//! Net2 as the table says.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlButtonElement, HtmlInputElement, HtmlSelectElement};

use crate::{
    client::{element, on_click},
    manage::{Step, promotion, undo_expiry, with_expiry, with_level},
    net2::{Failure, TOKEN_TYPES, User, failure, read_named, read_user, read_users},
    net2_client::{call, card_type, fill, load_departments, save_card, today},
    portrait_client::{PACE_MS, sleep},
    reader::{State, Tone, Tool},
};

struct Row {
    user: User,
    ticked: bool,
    result: String,
    /// Net2 refused the change, or may not have made it.
    problem: bool,
}

struct PurgeReview {
    cutoff: String,
    candidates: Vec<User>,
    downloaded: bool,
}

thread_local! {
    static ROWS: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
    /// Who the table shows; empty until people are shown.
    static SHOWN: RefCell<String> = const { RefCell::new(String::new()) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
    static STOP: Cell<Option<bool>> = const { Cell::new(None) };
    static PURGE: RefCell<Option<PurgeReview>> = const { RefCell::new(None) };
}

fn input(id: &str) -> HtmlInputElement {
    element(id).unchecked_into()
}

fn button(id: &str) -> HtmlButtonElement {
    element(id).unchecked_into()
}

fn notice(text: &str) {
    element("manage-notice").set_text_content(Some(text));
}

pub(super) fn confirmed(question: &str) -> bool {
    web_sys::window()
        .unwrap()
        .confirm_with_message(question)
        .unwrap_or(false)
}

/// The department picked in a list, with its name, or None for the list's
/// first option.
pub(super) fn picked(id: &str) -> Option<(i32, String)> {
    let list = element(id).unchecked_into::<HtmlSelectElement>();
    let value = list.value().parse().ok()?;
    let option = list.query_selector("option:checked").ok().flatten()?;
    Some((value, option.text_content()?))
}

fn people(count: usize) -> String {
    match count {
        1 => "1 person".into(),
        n => format!("{n} people"),
    }
}

pub(super) fn selected_person() -> Option<User> {
    ROWS.with_borrow(|rows| {
        let mut ticked = rows.iter().filter(|row| row.ticked);
        let person = ticked.next()?.user.clone();
        ticked.next().is_none().then_some(person)
    })
}

fn ends(user: &User, today: &str) -> String {
    match &user.expires {
        Some(date) if user.expired(today) => format!("{date} (ended)"),
        Some(date) => date.clone(),
        None => String::new(),
    }
}

pub(crate) fn busy() -> bool {
    BUSY.get()
}

pub(super) fn site_busy(state: &Rc<RefCell<State>>, busy: bool) {
    BUSY.set(busy);
    crate::client::render(&state.borrow());
}

pub(crate) fn forget_people() {
    PURGE.set(None);
    SHOWN.set(String::new());
    ROWS.with_borrow_mut(|rows| rows.iter_mut().for_each(|row| row.ticked = false));
    fill("manage-level", &[]);
    crate::manage_site_client::forget();
}

pub fn render(state: &State) {
    element("manage-tool").set_hidden(state.tool != Tool::Manage);
    if state.tool != Tool::Manage {
        return;
    }
    let busy = BUSY.get();
    let connected = state.net2.is_some();
    crate::manage_site_client::render(connected && !busy, selected_person().is_some());
    button("manage-stop").set_hidden(!busy || STOP.get().is_none());
    button("manage-load").set_disabled(busy || !connected);
    button("manage-promote").set_disabled(busy || !connected);
    button("manage-load-levels").set_disabled(busy || !connected);
    SHOWN.with_borrow(|shown| {
        element("manage-list").set_hidden(shown.is_empty());
        element("manage-heading").set_text_content(Some(shown));
    });
    let today = today();
    ROWS.with_borrow(|rows| {
        let ticked = rows.iter().filter(|r| r.ticked).count();
        let ready = !busy && connected && ticked > 0;
        button("manage-preview-purge").set_disabled(!ready);
        PURGE.with_borrow(|review| {
            button("manage-download-purge").set_disabled(!ready || review.is_none());
            button("manage-purge")
                .set_disabled(!ready || !review.as_ref().is_some_and(|review| review.downloaded));
        });
        button("manage-add-level").set_disabled(!ready);
        button("manage-remove-level").set_disabled(!ready);
        button("manage-undo").set_disabled(!ready);
        let card = state
            .card
            .as_ref()
            .filter(|_| state.status.0 == Tone::Ready);
        button("manage-replace").set_disabled(!ready || ticked != 1 || card.is_none());
        let card_note = match (ticked, card) {
            (1, Some(card)) => format!("Card {} is on the reader.", card.number),
            _ => "Tick one person and tap their new card.".into(),
        };
        element("manage-card").set_text_content(Some(&card_note));
        let moving = button("manage-move");
        moving.set_disabled(!ready);
        moving.set_text_content(Some(&match ticked {
            0 => "Move to department".into(),
            n => format!("Move {}", people(n)),
        }));
        let leaving = button("manage-leave");
        leaving.set_disabled(!ready);
        leaving.set_text_content(Some(&match ticked {
            0 => "Mark as leavers".into(),
            n => format!("Mark {} as leavers", people(n)),
        }));
        let all = input("manage-all");
        all.set_disabled(busy || rows.is_empty());
        all.set_checked(!rows.is_empty() && ticked == rows.len());
        all.set_indeterminate(ticked > 0 && ticked < rows.len());
        let mut node = element("manage-rows").first_element_child();
        for row in rows {
            let Some(tr) = node else { break };
            let cells: Vec<_> =
                std::iter::successors(tr.first_element_child(), |cell| cell.next_element_sibling())
                    .collect();
            let tick: HtmlInputElement = cells[0].first_element_child().unwrap().unchecked_into();
            tick.set_checked(row.ticked);
            tick.set_disabled(busy);
            for (cell, text) in [
                (&cells[3], ends(&row.user, &today)),
                (&cells[4], row.result.clone()),
            ] {
                if cell.text_content().as_deref() != Some(&text) {
                    cell.set_text_content(Some(&text));
                }
            }
            cells[4].set_class_name(match (row.problem, row.result.is_empty()) {
                (true, _) => "problem",
                (false, false) => "done",
                _ => "",
            });
            node = tr.next_element_sibling();
        }
    });
}

async fn show(state: Rc<RefCell<State>>) {
    PURGE.set(None);
    let department = picked("manage-department");
    let (path, label) = match &department {
        Some((id, name)) => (format!("/departments/{id}/users"), name.clone()),
        None => ("/users".to_string(), "All users".to_string()),
    };
    BUSY.set(true);
    notice(&format!("Reading {label} from Net2…"));
    crate::client::render(&state.borrow());
    let reply = call(&state, "GET", &path, None).await;
    match reply
        .map_err(|problem| problem.message)
        .and_then(|body| read_users(&body))
    {
        Ok(mut users) => {
            users.sort_by_key(|user| (user.last.to_lowercase(), user.first.to_lowercase()));
            let document = web_sys::window().unwrap().document().unwrap();
            let body = element("manage-rows");
            body.set_text_content(None);
            for (index, user) in users.iter().enumerate() {
                let tr = document.create_element("tr").unwrap();
                for column in 0..5 {
                    let td = document.create_element("td").unwrap();
                    match column {
                        0 => td.set_inner_html(&format!(
                            r#"<input type="checkbox" data-row="{index}" aria-label="Tick">"#
                        )),
                        1 => td.set_text_content(Some(&user.name())),
                        2 => td.set_text_content(Some(&user.id.to_string())),
                        _ => {}
                    }
                    tr.append_child(&td).unwrap();
                }
                body.append_child(&tr).unwrap();
            }
            notice(&format!(
                "{} in {label}. Tick the people to change.",
                people(users.len())
            ));
            ROWS.set(
                users
                    .into_iter()
                    .map(|user| Row {
                        user,
                        ticked: false,
                        result: String::new(),
                        problem: false,
                    })
                    .collect(),
            );
            SHOWN.set(label);
        }
        Err(message) => notice(&message),
    }
    BUSY.set(false);
    crate::client::render(&state.borrow());
}

/// Makes `change` for each ticked person in turn, paced as the other tools
/// are. Stops when asked, or when Net2 says to or may have half made it.
async fn each_ticked<F, Fut>(state: &Rc<RefCell<State>>, doing: &str, change: F)
where
    F: Fn(i32) -> Fut,
    Fut: Future<Output = Result<String, Failure>>,
{
    PURGE.set(None);
    BUSY.set(true);
    STOP.set(Some(false));
    crate::client::render(&state.borrow());
    let todo: Vec<(usize, i32)> = ROWS.with_borrow(|rows| {
        let ticked = rows.iter().enumerate().filter(|(_, row)| row.ticked);
        ticked.map(|(index, row)| (index, row.user.id)).collect()
    });
    let mut done = 0;
    let mut halted = false;
    for (position, &(index, id)) in todo.iter().enumerate() {
        if position > 0 {
            sleep(PACE_MS).await;
        }
        if STOP.get() == Some(true) {
            halted = true;
            break;
        }
        notice(&format!("{doing} {} of {}…", position + 1, todo.len()));
        let outcome = change(id).await;
        let stop = matches!(&outcome, Err(problem) if problem.halt || problem.uncertain);
        done += usize::from(outcome.is_ok());
        ROWS.with_borrow_mut(|rows| {
            let row = &mut rows[index];
            row.problem = outcome.is_err();
            row.result = outcome.unwrap_or_else(|problem| problem.message);
        });
        crate::client::render(&state.borrow());
        if stop {
            halted = true;
            break;
        }
    }
    BUSY.set(false);
    STOP.set(None);
    notice(&format!(
        "{} {done} of {} done.",
        if halted { "Stopped." } else { "Done." },
        todo.len()
    ));
    crate::client::render(&state.borrow());
}

fn preview_purge() {
    PURGE.set(None);
    let Ok(years) = input("manage-retention").value().parse::<u32>() else {
        return notice("Enter how many years to keep expired records.");
    };
    if !(1..=100).contains(&years) {
        return notice("Enter a retention period from 1 to 100 years.");
    }
    let now = js_sys::Date::new_0();
    let year = now.get_full_year() - years;
    let month = now.get_month() + 1;
    let day = now.get_date();
    let mut cutoff = format!("{year:04}-{month:02}-{day:02}");
    if crate::people::read_date(&cutoff).is_err() {
        cutoff = format!("{year:04}-02-28");
    }
    let candidates = ROWS.with_borrow(|rows| {
        rows.iter()
            .filter(|row| row.ticked && row.user.expired(&cutoff))
            .map(|row| row.user.clone())
            .collect::<Vec<_>>()
    });
    if candidates.is_empty() {
        return notice(&format!("No ticked records expired before {cutoff}."));
    }
    if !confirmed(&format!(
        "Review {} whose Net2 access expired before {cutoff}? Review only people whose records you intend to purge.",
        people(candidates.len())
    )) {
        return;
    }
    ROWS.with_borrow_mut(|rows| {
        for row in rows.iter_mut().filter(|row| row.ticked) {
            row.result = if row.user.expired(&cutoff) {
                "Eligible for purge"
            } else {
                "Outside purge cutoff"
            }
            .into();
            row.problem = false;
        }
    });
    notice(&format!(
        "{} eligible. Download the review before purging.",
        people(candidates.len())
    ));
    PURGE.set(Some(PurgeReview {
        cutoff,
        candidates,
        downloaded: false,
    }));
}

fn download_purge() {
    PURGE.with_borrow_mut(|review| {
        let Some(review) = review else { return };
        let headers = ["User ID", "Access ends", "Purge cutoff", "Result"].map(String::from);
        let rows = review.candidates.iter().map(|user| {
            vec![
                user.id.to_string(),
                user.expires.clone().unwrap_or_default(),
                review.cutoff.clone(),
                "Eligible for purge".into(),
            ]
        });
        match crate::sheet::write_csv(&headers, rows, false)
            .and_then(|bytes| crate::batch_client::save(&bytes, "net2-purge-review.csv"))
        {
            Ok(()) => {
                review.downloaded = true;
                notice("Review sent to Downloads. Check the file before confirming the purge.");
            }
            Err(text) => notice(&text),
        }
    });
}

async fn purge_leavers(state: Rc<RefCell<State>>, review: PurgeReview) {
    each_ticked(&state, "Checking purge for", |id| {
        let state = state.clone();
        let reviewed = review.candidates.iter().find(|user| user.id == id).cloned();
        async move {
            let Some(reviewed) = reviewed else {
                return Ok("Outside purge cutoff.".into());
            };
            let path = format!("/users/{id}");
            let body = call(&state, "GET", &path, None).await?;
            let current = read_user(&body, id).map_err(|text| failure(text, false, false))?;
            if current.expires != reviewed.expires || current.name() != reviewed.name() {
                return Err(failure(
                    "User changed since the review. Review again.",
                    false,
                    false,
                ));
            }
            call(&state, "DELETE", &format!("{path}?purge=true"), None).await?;
            ROWS.with_borrow_mut(|rows| {
                if let Some(row) = rows.iter_mut().find(|row| row.user.id == id) {
                    row.ticked = false;
                }
            });
            Ok("Purged user and history.".into())
        }
    })
    .await;
}

async fn put_in(
    state: &Rc<RefCell<State>>,
    user: i32,
    department: &(i32, String),
) -> Result<(), Failure> {
    let (id, name) = department;
    let body = json!({ "id": id, "name": name });
    call(
        state,
        "PUT",
        &format!("/users/{user}/departments"),
        Some(&body),
    )
    .await
    .map(drop)
}

async fn move_ticked(state: Rc<RefCell<State>>, department: (i32, String)) {
    each_ticked(&state, "Moving", |user| {
        let (state, department) = (state.clone(), department.clone());
        async move {
            put_in(&state, user, &department).await?;
            Ok(format!("Moved to {}.", department.1))
        }
    })
    .await;
}

async fn load_levels(state: Rc<RefCell<State>>) {
    BUSY.set(true);
    crate::client::render(&state.borrow());
    match call(&state, "GET", "/accesslevels", None).await {
        Ok(body) => {
            let levels = read_named(&body);
            fill("manage-level", &levels);
            notice(&format!("{} access levels available.", levels.len()));
        }
        Err(problem) => notice(&problem.message),
    }
    BUSY.set(false);
    crate::client::render(&state.borrow());
}

async fn change_level(state: Rc<RefCell<State>>, level: (i32, String), add: bool) {
    each_ticked(&state, "Updating access for", |user| {
        let (state, (id, name)) = (state.clone(), level.clone());
        async move {
            let path = format!("/users/{user}/doorpermissionset");
            let body = call(&state, "GET", &path, None).await?;
            let change = with_level(&body, id, add).map_err(|text| failure(text, false, false))?;
            let Some(change) = change else {
                return Ok(if add {
                    format!("Already has {name}.")
                } else {
                    format!("Already without {name}.")
                });
            };
            call(&state, "PUT", &path, Some(&change)).await?;
            // Net2 answers 204 but may keep only the first access level.
            let kept = call(&state, "GET", &path, None)
                .await
                .is_ok_and(|reply| with_level(&reply, id, add) == Ok(None));
            if !kept {
                return Err(failure(
                    format!(
                        "Net2 did not save the change to {name}. It may allow only one access level per person: check them in Net2."
                    ),
                    false,
                    true,
                ));
            }
            Ok(if add {
                format!("Added {name}.")
            } else {
                format!("Removed {name}.")
            })
        }
    })
    .await;
}

async fn replace_card(state: Rc<RefCell<State>>, number: u32, kind: String) {
    let value = number.to_string();
    each_ticked(&state, "Replacing card for", |user| {
        let (state, value, kind) = (state.clone(), value.clone(), kind.clone());
        async move {
            let result = save_card(&state, user, &value, &kind, true, true).await?;
            let retired = result
                .map_err(|card| failure(format!("Already has card {card}."), false, false))?;
            if state
                .borrow()
                .card
                .as_ref()
                .is_some_and(|card| card.number == number)
            {
                state.borrow_mut().manage_assigned = Some(number);
            }
            Ok(format!("Saved card {value}.{retired}"))
        }
    })
    .await;
}

async fn undo_leavers(state: Rc<RefCell<State>>, date: Option<String>) {
    each_ticked(&state, "Updating expiry for", |user| {
        let (state, date) = (state.clone(), date.clone());
        async move {
            let path = format!("/users/{user}");
            let original = call(&state, "GET", &path, None).await?;
            let record = undo_expiry(&original, user, date.as_deref())
                .map_err(|message| failure(message, false, false))?;
            let Some(record) = record else {
                return Ok("Access already has no expiry.".into());
            };
            sleep(PACE_MS).await;
            call(&state, "PUT", &path, Some(&record)).await?;
            sleep(PACE_MS).await;
            let verified = call(&state, "GET", &path, None)
                .await
                .ok()
                .and_then(|reply| read_user(&reply, user).ok());
            if !verified.as_ref().is_some_and(|user| user.expires == date) {
                return Err(failure(
                    "Net2 did not confirm the new expiry. Check the user in Net2.",
                    true,
                    true,
                ));
            }
            ROWS.with_borrow_mut(|rows| {
                if let Some(row) = rows.iter_mut().find(|row| row.user.id == user) {
                    row.user.expires = date.clone();
                }
            });
            Ok(date.map_or_else(
                || "Expiry cleared.".into(),
                |day| format!("Access ends after {day}."),
            ))
        }
    })
    .await;
}

/// The department first, so a leaver whose record cannot be read is at
/// least out of their year group, and the result says which half happened.
async fn mark_leavers(
    state: Rc<RefCell<State>>,
    last_day: String,
    department: Option<(i32, String)>,
) {
    each_ticked(&state, "Marking", |user| {
        let (state, last_day, department) = (state.clone(), last_day.clone(), department.clone());
        async move {
            let mut said = String::new();
            if let Some(department) = &department {
                put_in(&state, user, department).await?;
                said = format!("Moved to {}. ", department.1);
                sleep(PACE_MS).await;
            }
            let path = format!("/users/{user}");
            let marked = async {
                let body = call(&state, "GET", &path, None).await?;
                let record = with_expiry(&body, user, &last_day)
                    .map_err(|message| failure(message, false, false))?;
                if let Some(record) = &record {
                    sleep(PACE_MS).await;
                    call(&state, "PUT", &path, Some(record)).await?;
                }
                Ok::<_, Failure>(record.is_some())
            };
            match marked.await {
                Ok(true) => {
                    ROWS.with_borrow_mut(|rows| {
                        if let Some(row) = rows.iter_mut().find(|row| row.user.id == user) {
                            row.user.expires = Some(last_day.clone());
                        }
                    });
                    Ok(format!("{said}Access ends after {last_day}."))
                }
                Ok(false) => Ok(format!(
                    "{said}Access already ends sooner, so it was left as it is."
                )),
                Err(mut problem) => {
                    problem.message = format!("{said}Access not ended: {}", problem.message);
                    Err(problem)
                }
            }
        }
    })
    .await;
}

/// Reads Net2's departments at the moment of asking, so the plan confirmed
/// is the plan made, and a stopped change is carried on rather than redone.
async fn year_change(state: Rc<RefCell<State>>) {
    let year = |id: &str| {
        input(id)
            .value()
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|n| (1..=14).contains(n))
    };
    let leavers = input("manage-leavers-name").value().trim().to_string();
    let (Some(lowest), Some(top)) = (year("manage-from"), year("manage-to")) else {
        return notice("Enter the lowest and top years as numbers from 1 to 14.");
    };
    if lowest >= top || leavers.is_empty() {
        return notice("The top year must be above the lowest, and the top year needs a new name.");
    }
    BUSY.set(true);
    STOP.set(Some(false));
    notice("Reading Net2’s departments…");
    crate::client::render(&state.borrow());
    let finish = |message: &str| {
        BUSY.set(false);
        STOP.set(None);
        notice(message);
        crate::client::render(&state.borrow());
    };
    let departments = match call(&state, "GET", "/departments", None).await {
        Ok(reply) => read_named(&reply),
        Err(problem) => return finish(&problem.message),
    };
    let steps = match promotion(&departments, lowest..=top, &leavers) {
        Ok(steps) => steps,
        Err(message) => return finish(&message),
    };
    if steps.is_empty() {
        return finish(&if departments
            .iter()
            .any(|(_, name)| name.eq_ignore_ascii_case(&leavers))
        {
            format!(
                "Nothing to change: Net2 already has {leavers}, so this year’s change looks done. To change it again, rename {leavers} in Net2 first."
            )
        } else {
            format!("Nothing to change: Net2 has no year group departments from {lowest} to {top}.")
        });
    }
    let plan: Vec<_> = steps
        .iter()
        .map(|step| format!("• {}", step.describe()))
        .collect();
    let question = format!(
        "Make these {} changes in Net2?\n{}\nEveryone in each department moves with it.",
        steps.len(),
        plan.join("\n")
    );
    let question = if departments
        .iter()
        .any(|(_, name)| name.eq_ignore_ascii_case(&leavers))
    {
        format!(
            "Resume a stopped year change only if {leavers} already contains the original top year.\n\n{question}"
        )
    } else {
        question
    };
    if !confirmed(&question) {
        return finish("Nothing changed.");
    }
    let mut made = 0;
    let mut stopped = None;
    for step in &steps {
        if made > 0 {
            sleep(PACE_MS).await;
        }
        if STOP.get() == Some(true) {
            stopped = Some("you stopped it.".to_string());
            break;
        }
        notice(&format!(
            "Year change: {} ({} of {})…",
            step.describe(),
            made + 1,
            steps.len()
        ));
        let result = match step {
            Step::Rename { id, to, .. } => {
                let body = json!({ "id": id, "name": to });
                call(&state, "PUT", &format!("/departments/{id}"), Some(&body)).await
            }
            Step::Create(name) => {
                call(
                    &state,
                    "POST",
                    "/departments",
                    Some(&json!({ "name": name })),
                )
                .await
            }
        };
        if let Err(problem) = result {
            stopped = Some(format!(
                "{} did not happen: {}",
                step.describe(),
                problem.message
            ));
            break;
        }
        made += 1;
    }
    load_departments(state.clone()).await;
    finish(&match stopped {
        None => format!(
            "Year change done: {made} changes. Show people again to see the new departments."
        ),
        Some(why) => format!(
            "Year change stopped after {made} of {} changes, because {why} Run it again to carry on; nothing is changed twice.",
            steps.len()
        ),
    });
}

pub fn setup(state: Rc<RefCell<State>>) {
    crate::manage_site_client::setup(&state);
    input("manage-leave-date").set_value(&today());
    input("manage-leavers-name").set_value(&format!(
        "Leavers {}",
        js_sys::Date::new_0().get_full_year()
    ));

    let tick_state = state.clone();
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
        ROWS.with_borrow_mut(|rows| {
            if let Some(row) = rows.get_mut(index) {
                row.ticked = tick.checked();
            }
        });
        PURGE.set(None);
        crate::client::render(&tick_state.borrow());
    });
    element("manage-rows").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let all_state = state.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        let checked = input("manage-all").checked();
        ROWS.with_borrow_mut(|rows| rows.iter_mut().for_each(|row| row.ticked = checked));
        PURGE.set(None);
        crate::client::render(&all_state.borrow());
    });
    element("manage-all").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let show_state = state.clone();
    on_click("manage-load", move || spawn_local(show(show_state.clone())));
    on_click("manage-stop", || STOP.set(Some(true)));
    let retention_state = state.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        PURGE.set(None);
        crate::client::render(&retention_state.borrow());
    });
    element("manage-retention").set_oninput(Some(callback.as_ref().unchecked_ref()));
    callback.forget();
    let preview_state = state.clone();
    on_click("manage-preview-purge", move || {
        preview_purge();
        crate::client::render(&preview_state.borrow());
    });
    let download_state = state.clone();
    on_click("manage-download-purge", move || {
        download_purge();
        crate::client::render(&download_state.borrow());
    });
    let purge_state = state.clone();
    on_click("manage-purge", move || {
        let count = PURGE.with_borrow(|review| {
            review
                .as_ref()
                .filter(|review| review.downloaded)
                .map(|r| r.candidates.len())
        });
        let Some(count) = count else { return };
        if confirmed(&format!(
            "Permanently purge {} and their historical data from Net2? Check that the review downloaded. This cannot be undone.",
            people(count)
        )) && let Some(review) = PURGE.take()
        {
            spawn_local(purge_leavers(purge_state.clone(), review));
        }
    });

    let levels_state = state.clone();
    on_click("manage-load-levels", move || {
        spawn_local(load_levels(levels_state.clone()))
    });
    let ticked = || ROWS.with_borrow(|rows| rows.iter().filter(|row| row.ticked).count());
    let undo_state = state.clone();
    on_click("manage-undo", move || {
        let day = input("manage-undo-date").value();
        let (date, action) = if day.is_empty() {
            (None, "Clear the expiry".into())
        } else {
            let action = format!("Set the expiry to 23:59 on {day}");
            (Some(day), action)
        };
        if confirmed(&format!(
            "{action} for {}? This may restore access.",
            people(ticked())
        )) {
            spawn_local(undo_leavers(undo_state.clone(), date));
        }
    });
    for (button, add, verb) in [
        ("manage-add-level", true, "Add"),
        ("manage-remove-level", false, "Remove"),
    ] {
        let state = state.clone();
        on_click(button, move || {
            let Some(level) = picked("manage-level") else {
                return notice("Load and choose an access level.");
            };
            let direction = if add { "to" } else { "from" };
            if confirmed(&format!(
                "{verb} {} {direction} {} in Net2?",
                level.1,
                people(ticked())
            )) {
                spawn_local(change_level(state.clone(), level, add));
            }
        });
    }
    let card_state = state.clone();
    on_click("manage-replace", move || {
        let person = selected_person();
        let number = card_state.borrow().card.as_ref().map(|card| card.number);
        let (Some(person), Some(number)) = (person, number) else {
            return notice("Tick one person and tap their new card.");
        };
        let kind = card_type();
        let Some((_, label)) = TOKEN_TYPES.iter().find(|(value, _)| *value == kind) else {
            return notice("Choose a card type on Batch assign cards first.");
        };
        if confirmed(&format!(
            "Save card {number} ({label}) for {} (Net2 ID {})? Their other active cards will be marked lost after the new card is saved.",
            person.name(),
            person.id
        )) {
            spawn_local(replace_card(card_state.clone(), number, kind));
        }
    });

    let move_state = state.clone();
    on_click("manage-move", move || {
        let Some(department) = picked("manage-move-to") else {
            return notice("Choose the department to move them to.");
        };
        if confirmed(&format!("Move {} to {}?", people(ticked()), department.1)) {
            spawn_local(move_ticked(move_state.clone(), department));
        }
    });

    let leave_state = state.clone();
    on_click("manage-leave", move || {
        let last_day = input("manage-leave-date").value();
        if last_day.is_empty() {
            return notice("Choose the last day they can get in.");
        }
        let department = picked("manage-leavers-to");
        let moving = department.as_ref().map_or(String::new(), |(_, name)| {
            format!(" They move to {name} first.")
        });
        let question = format!(
            "Mark {} as leavers?\nTheir access ends at 23:59 on {last_day}, and their cards stop working then.{moving} Anyone whose access already ends sooner keeps that date.",
            people(ticked())
        );
        if confirmed(&question) {
            spawn_local(mark_leavers(leave_state.clone(), last_day, department));
        }
    });

    on_click("manage-promote", move || {
        spawn_local(year_change(state.clone()))
    });
}
