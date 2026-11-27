//! Read-only Net2 reports and the individually confirmed reception actions.
use std::{cell::RefCell, rc::Rc};

use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlButtonElement;

use crate::{
    client::{element, on_click},
    manage_client::{confirmed, picked, selected_person, site_busy},
    net2::{TOKEN_TYPES, User, field, last_access_query, read_named, text},
    net2_client::{call, card_type, fill, save_card},
    reader::State,
};

type App = Rc<RefCell<State>>;
const UNREADABLE: &str = "Net2 sent a reply Tusk does not understand.";

fn report(title: &str, status: &str) {
    element("manage-site-results").set_hidden(false);
    element("manage-site-heading").set_text_content(Some(title));
    element("manage-site-status").set_text_content(Some(status));
    element("manage-site-table").set_text_content(None);
}

fn table(headers: &[&str], rows: impl IntoIterator<Item = Vec<String>>) {
    let doc = web_sys::window().unwrap().document().unwrap();
    let table = doc.create_element("table").unwrap();
    let head = doc.create_element("thead").unwrap();
    let row = doc.create_element("tr").unwrap();
    for heading in headers {
        let th = doc.create_element("th").unwrap();
        th.set_attribute("scope", "col").unwrap();
        th.set_text_content(Some(heading));
        row.append_child(&th).unwrap();
    }
    head.append_child(&row).unwrap();
    table.append_child(&head).unwrap();
    let body = doc.create_element("tbody").unwrap();
    for cells in rows {
        let row = doc.create_element("tr").unwrap();
        for cell in cells {
            let td = doc.create_element("td").unwrap();
            td.set_text_content(Some(&cell));
            row.append_child(&td).unwrap();
        }
        body.append_child(&row).unwrap();
    }
    table.append_child(&body).unwrap();
    element("manage-site-table").append_child(&table).unwrap();
}

fn visible_date(date: &str) -> String {
    if date.starts_with("0001-01-01") {
        return String::new();
    }
    date.replacen('T', " ", 1)
}

fn int(record: &Value, key: &str) -> Option<i32> {
    field(record, key)?.as_i64()?.try_into().ok()
}

async fn records(state: &App, path: &str) -> Result<Vec<Value>, String> {
    let body = call(state, "GET", path, None)
        .await
        .map_err(|err| err.message)?;
    serde_json::from_str(&body).map_err(|_| UNREADABLE.into())
}

async fn load_choices(state: &App, picker: &str, path: &str, label: &str) {
    report(label, "Reading Net2…");
    fill(picker, &[]);
    let result = match picker {
        "manage-door" => call(state, "GET", path, None)
            .await
            .map(|body| read_named(&body))
            .map_err(|err| err.message),
        _ => records(state, path).await.map(|rows| {
            let mut seen = std::collections::HashSet::new();
            rows.iter()
                .filter_map(|row| {
                    let number = int(row, "tokenNumber").filter(|number| *number > 0)?;
                    let when = visible_date(&text(row, "eventDate"));
                    let place = text(row, "where");
                    (!when.is_empty() && !place.is_empty() && seen.insert(number))
                        .then(|| (number, format!("{number} · {when} · {place}")))
                })
                .collect()
        }),
    };
    match result {
        Ok(choices) => {
            fill(picker, &choices);
            report(
                label,
                &if choices.is_empty() {
                    "Net2 has no entries to show.".into()
                } else {
                    format!("{} available. Choose one in the list.", choices.len())
                },
            );
        }
        Err(text) => report(label, &text),
    }
}

async fn activity(state: &App, person: User) {
    let id = person.id;
    let title = format!("{} · Net2 ID {id}", person.name());
    report(&title, "Reading Net2…");
    let path = format!(
        "/customquery/querydb?query={}",
        js_sys::encode_uri_component(&last_access_query(id))
    );
    let latest = match records(state, &path).await {
        Ok(rows) => rows.first().map_or_else(
            || "No last access recorded.".into(),
            |row| {
                let when = visible_date(&text(row, "lastaccesstime"));
                let place = text(row, "lastarea");
                if when.is_empty() {
                    "No last access recorded.".into()
                } else if place.is_empty() {
                    format!("Last access: {when}.")
                } else {
                    format!("Last access: {when} · {place}.")
                }
            },
        ),
        Err(err) => format!("Last access unavailable. {err}"),
    };
    let filter = js_sys::encode_uri_component(&format!("UserID = {id}"));
    match records(
        state,
        &format!("/events?rowCount=25&where={filter}&orderBy=EventTime%20DESC"),
    )
    .await
    {
        Ok(events) => {
            let events: Vec<_> = events
                .iter()
                .filter(|row| int(row, "userID") == Some(id))
                .collect();
            let summary = if events.is_empty() {
                format!("{latest} No recent events.")
            } else {
                format!("{latest} Showing {} recent Net2 events.", events.len())
            };
            report(&title, &summary);
            if !events.is_empty() {
                table(
                    &["Time", "Where", "Event", "Details"],
                    events.into_iter().map(|row| {
                        vec![
                            visible_date(&text(row, "eventTime")),
                            text(row, "deviceName"),
                            text(row, "eventDescription"),
                            text(row, "eventDetails"),
                        ]
                    }),
                );
            }
        }
        Err(err) => report(
            &title,
            &format!("{latest} Recent events unavailable. {err}"),
        ),
    }
}

async fn door_command(state: &App, id: i32, name: &str, command: &str) {
    report(name, "Sending door command…");
    match call(
        state,
        "POST",
        &format!("/commands/door/{command}"),
        Some(&json!({"doorId": id})),
    )
    .await
    {
        Ok(body) => {
            let response = serde_json::from_str::<Value>(&body).unwrap_or_default();
            let status = field(&response, "output")
                .map_or_else(String::new, |output| text(output, "status"));
            report(
                name,
                &match status.as_str() {
                    "CompletedOk" => "Net2 completed the command.".into(),
                    "CompletedFalied" | "CompletedFailed" => {
                        "Net2 reported that the command failed.".into()
                    }
                    "Scheduled" | "Running" | "FireAndForget" => {
                        format!("Net2 accepted the command ({status}). Check the door’s status.")
                    }
                    _ => {
                        "Net2 did not confirm the command outcome. Check the door before retrying."
                            .into()
                    }
                },
            );
        }
        Err(err) => report(name, &err.message),
    }
}

pub fn forget() {
    for id in ["manage-unknown", "manage-door"] {
        fill(id, &[]);
    }
    element("manage-site-results").set_hidden(true);
    element("manage-site-table").set_text_content(None);
}

pub fn render(ready: bool, one_person: bool) {
    for id in [
        "manage-load-unknown",
        "manage-load-doors",
        "manage-open-door",
        "manage-hold-door",
        "manage-normal-door",
    ] {
        element(id)
            .unchecked_into::<HtmlButtonElement>()
            .set_disabled(!ready);
    }
    for id in ["manage-activity", "manage-assign-unknown"] {
        element(id)
            .unchecked_into::<HtmlButtonElement>()
            .set_disabled(!ready || !one_person);
    }
}

fn run(state: &App, work: impl AsyncFnOnce(&App) + 'static) {
    site_busy(state, true);
    let state = state.clone();
    spawn_local(async move {
        work(&state).await;
        if state.borrow().net2.is_none() {
            forget();
        }
        site_busy(&state, false);
    });
}

pub fn setup(state: &App) {
    for (button, picker, path, label) in [
        (
            "manage-load-unknown",
            "manage-unknown",
            "/events/latestunknowntokens?max=20",
            "Unknown cards",
        ),
        ("manage-load-doors", "manage-door", "/doors", "Doors"),
    ] {
        let state = state.clone();
        on_click(button, move || {
            run(&state, async move |state| {
                load_choices(state, picker, path, label).await
            })
        });
    }
    let app = state.clone();
    on_click("manage-activity", move || {
        if let Some(person) = selected_person() {
            run(&app, async move |state| activity(state, person).await);
        }
    });
    let app = state.clone();
    on_click("manage-assign-unknown", move || {
        let (Some(person), Some((number, card))) = (selected_person(), picked("manage-unknown"))
        else {
            return report(
                "Unknown cards",
                "Tick one person and choose the card they tapped.",
            );
        };
        let value = number.to_string();
        let kind = card_type();
        let Some((_, label)) = TOKEN_TYPES.iter().find(|(value, _)| *value == kind) else {
            return report(
                "Unknown cards",
                "Choose a card type on Batch assign cards first.",
            );
        };
        if !confirmed(&format!(
            "Assign {card} ({label}) to {} (Net2 ID {})?",
            person.name(),
            person.id
        )) {
            return;
        }
        run(&app, async move |state| {
            report("Unknown cards", "Saving card in Net2…");
            match save_card(state, person.id, &value, &kind, true, false).await {
                Ok(Ok(_)) => {
                    fill("manage-unknown", &[]);
                    report(
                        "Unknown cards",
                        &format!("Saved card {value} for {}.", person.name()),
                    );
                }
                Ok(Err(card)) => report(
                    "Unknown cards",
                    &format!("This person already has card {card}."),
                ),
                Err(err) => report("Unknown cards", &err.message),
            }
        });
    });
    for (button, command, verb) in [
        ("manage-open-door", "open", "Open once"),
        (
            "manage-hold-door",
            "holdopen",
            "Hold open until returned to normal operation",
        ),
        (
            "manage-normal-door",
            "reinstate",
            "Return to normal operation",
        ),
    ] {
        let app = state.clone();
        on_click(button, move || {
            let Some((id, name)) = picked("manage-door") else {
                return report("Doors", "Load and choose a door.");
            };
            if confirmed(&format!("{verb} for {name} (Net2 door ID {id})?")) {
                run(&app, async move |state| {
                    door_command(state, id, &name, command).await
                });
            }
        });
    }
}
