//! Signing in to Net2 and the requests that follow, straight from the page.
//! The access token stays in memory and goes only to the Net2 server.
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
};

use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{AbortSignal, Headers, HtmlInputElement, HtmlSelectElement, RequestInit, Response};

use crate::{
    batch::Batch,
    client::{element, on_click},
    net2::{
        Failure, UNCERTAIN_WRITE, access_token, describe_owners, failure, lost_tokens, owner_query,
        parse_origin, read_cards, read_departments, read_users, refresh_token, sign_in_failure,
        status_failure,
    },
    reader::{Session, State, Tone, Tool},
};

const UNREACHABLE: &str = "Could not reach Net2. Check the server address, that this computer trusts the Net2 certificate, and that Chrome may reach devices on your network.";

/// Which address failed matters most: a server name can resolve to a VPN
/// adapter that drops the connection, while localhost always works on the
/// Net2 server itself.
fn unreachable(origin: &str) -> String {
    let local = ["https://localhost", "https://127.0.0.1", "https://[::1]"]
        .iter()
        .any(|prefix| origin == *prefix || origin.starts_with(&format!("{prefix}:")));
    let hint = if local {
        "Check that the Net2 services are running and LocalAPI is enabled."
    } else {
        "Open that address in a browser tab to see why. On the Net2 server itself, use https://localhost:8443; a VPN can make the server's name unreachable."
    };
    format!("Could not reach {origin}. {hint}")
}

thread_local! {
    static APP: OnceCell<Rc<RefCell<State>>> = const { OnceCell::new() };
    /// A tapped card is on its way to Net2.
    static SAVING: Cell<bool> = const { Cell::new(false) };
    /// The card on the Read tab and what Net2 said about it, once it answers.
    static OWNER: RefCell<Option<(u32, Option<String>)>> = const { RefCell::new(None) };
    /// The Net2 panel was asked for on the Read tab.
    static LOOK_UP: Cell<bool> = const { Cell::new(false) };
}

fn value(id: &str) -> String {
    element(id).unchecked_into::<HtmlInputElement>().value()
}

fn clear(id: &str) {
    element(id)
        .unchecked_into::<HtmlInputElement>()
        .set_value("");
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

async fn fetch(
    origin: &str,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<(&str, String)>,
) -> Result<String, Failure> {
    // Signing in changes nothing, so a lost reply there is just unreachable.
    let write = token.is_some() && method != "GET";
    let headers = Headers::new().unwrap();
    headers.set("Accept", "application/json").unwrap();
    if let Some(token) = token {
        headers
            .set("Authorization", &format!("Bearer {token}"))
            .unwrap();
    }
    let init = RequestInit::new();
    init.set_method(method);
    if let Some((kind, body)) = body {
        headers.set("Content-Type", kind).unwrap();
        init.set_body(&JsValue::from_str(&body));
    }
    init.set_headers(&headers);
    init.set_signal(Some(&AbortSignal::timeout_with_u32(30_000)));
    let request = web_sys::window()
        .unwrap()
        .fetch_with_str_and_init(&format!("{origin}/api/v1{path}"), &init);
    let Ok(response) = JsFuture::from(request).await else {
        let message = if write { UNCERTAIN_WRITE } else { UNREACHABLE };
        return Err(failure(message, write, true));
    };
    let response: Response = response.unchecked_into();
    let text = match response.text() {
        Ok(text) => JsFuture::from(text).await.ok().and_then(|t| t.as_string()),
        Err(_) => None,
    }
    .unwrap_or_default();
    if !response.ok() {
        let retry = response.headers().get("Retry-After").ok().flatten();
        return Err(status_failure(
            response.status(),
            retry.as_deref(),
            write,
            &text,
        ));
    }
    Ok(text)
}

/// A sign-in body. A form post is a CORS simple request: no preflight. Net2's
/// nginx allows one sign-in request per 500 ms from anywhere but loopback, so
/// a preflight followed by a JSON post always gets a 429 the page cannot see.
fn form(fields: &[(&str, &str)]) -> Option<(&'static str, String)> {
    let body = fields
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{key}={}", js_sys::encode_uri_component(value)))
        .collect::<Vec<_>>()
        .join("&");
    Some(("application/x-www-form-urlencoded", body))
}

/// A session whose token is not `stale`: one another request already renewed,
/// or a new one from the refresh token. None if signed out meanwhile or Net2
/// will not renew, so a Disconnect is never undone by a late reply.
async fn renewed(state: &Rc<RefCell<State>>, stale: &str) -> Option<Session> {
    let current = state.borrow().net2.clone()?;
    if current.token != stale {
        return Some(current);
    }
    let refresh = current.refresh.as_deref()?;
    let (client_id, secret) = &current.client;
    let body = form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", client_id),
        ("client_secret", secret),
    ]);
    let reply = fetch(&current.origin, None, "POST", "/authorization/tokens", body)
        .await
        .ok()?;
    let session = Session {
        token: access_token(&reply)?,
        refresh: refresh_token(&reply).or_else(|| current.refresh.clone()),
        ..current
    };
    let mut state = state.borrow_mut();
    if state.net2.as_ref()?.token == stale {
        state.net2 = Some(session);
    }
    state.net2.clone()
}

/// A request as the signed-in operator. An expired token is renewed and the
/// request tried once more: Net2 refused the first, so nothing is sent twice.
/// A token that cannot be renewed signs the page out.
pub async fn call(
    state: &Rc<RefCell<State>>,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<String, Failure> {
    let session = state.borrow().net2.clone();
    let Some(Session { origin, token, .. }) = session else {
        return Err(failure("Connect to Net2 first.", false, true));
    };
    let body = body.map(|body| ("application/json", body.to_string()));
    let mut result = fetch(&origin, Some(&token), method, path, body.clone()).await;
    if result.as_ref().is_err_and(|problem| problem.expired)
        && let Some(session) = renewed(state, &token).await
    {
        result = fetch(&session.origin, Some(&session.token), method, path, body).await;
    }
    if result.as_ref().is_err_and(|problem| problem.expired) {
        crate::portrait_client::forget_matches();
        let mut state = state.borrow_mut();
        state.net2 = None;
        state.net2_status = (
            Tone::Problem,
            "Net2 signed this page out. Connect again.".into(),
        );
        crate::client::render(&state);
    }
    result
}

pub fn render(state: &State) {
    let net2_source = element("batch-source")
        .unchecked_into::<HtmlSelectElement>()
        .value()
        == "net2";
    let connected = state.net2.is_some();
    element("net2-panel").set_hidden(match state.tool {
        Tool::Reader => !connected && !LOOK_UP.get(),
        Tool::Batch => !net2_source,
        Tool::Portraits => false,
    });
    let (tone, message) = &state.net2_status;
    element("net2-state")
        .set_attribute("data-tone", tone.name())
        .unwrap();
    element("net2-message").set_text_content(Some(message));
    element("net2-form").set_hidden(state.net2.is_some());
    element("net2-disconnect").set_hidden(state.net2.is_none());
    let sending = state.batch.as_ref().is_some_and(|b| b.sending.is_some());
    element("batch-load")
        .unchecked_into::<web_sys::HtmlButtonElement>()
        .set_disabled(state.net2.is_none() || sending);
    // The reader handler staged a tap; send it once the borrow is released.
    if sending && !SAVING.get() {
        SAVING.set(true);
        if let Some(app) = APP.with(|app| app.get().cloned()) {
            spawn_local(save_card(app));
        }
    }
    element("owner-connect").set_hidden(connected || LOOK_UP.get());
    // Each tap on the Read tab is looked up once; lifting the card forgets it.
    let card = state
        .card
        .as_ref()
        .filter(|_| connected && state.tool == Tool::Reader)
        .map(|token| token.number);
    let owner = OWNER.with_borrow_mut(|owner| {
        if owner.as_ref().map(|(number, _)| *number) != card {
            *owner = card.map(|number| (number, None));
            if let (Some(number), Some(app)) = (card, APP.with(|app| app.get().cloned())) {
                spawn_local(look_up(app, number));
            }
        }
        owner.as_ref().map(|(_, said)| said.clone())
    });
    let line = element("owner");
    line.set_hidden(owner.is_none());
    line.set_text_content(
        owner
            .map(|said| said.unwrap_or_else(|| "Looking up in Net2…".into()))
            .as_deref(),
    );
}

async fn look_up(state: Rc<RefCell<State>>, card: u32) {
    let path = format!(
        "/customquery/querydb?query={}",
        js_sys::encode_uri_component(&owner_query(card))
    );
    let said = match call(&state, "GET", &path, None).await {
        Ok(reply) => describe_owners(&reply).unwrap_or_else(|message| message),
        Err(problem) => problem.message,
    };
    OWNER.with_borrow_mut(|owner| {
        if let Some((number, answer)) = owner
            && *number == card
        {
            *answer = Some(said);
        }
    });
    crate::client::render(&state.borrow());
}

async fn save_card(state: Rc<RefCell<State>>) {
    let job = state.borrow().batch.as_ref().and_then(|batch| {
        let (id, name, value) = batch.sending_to()?;
        Some((id, name, value, batch.replace))
    });
    let Some((id, name, value, replace)) = job else {
        SAVING.set(false);
        return;
    };
    let path = format!("/users/{id}/tokens");
    let token_type = element("batch-token-type")
        .unchecked_into::<HtmlSelectElement>()
        .value();
    let retire = replace
        && element("batch-retire")
            .unchecked_into::<HtmlInputElement>()
            .checked();
    let outcome = async {
        // Net2 is asked at the moment of writing, so the answer is current.
        let mut old = Vec::new();
        if !replace || retire {
            let reply = call(&state, "GET", &path, None).await?;
            let unreadable = |message| failure(message, false, false);
            if !replace
                && let Some(card) = read_cards(&reply).map_err(unreadable)?.into_iter().next()
            {
                return Ok(Err(card));
            }
            old = lost_tokens(&reply, &value).map_err(unreadable)?;
        }
        let token = json!({ "tokenType": token_type, "tokenValue": value, "isLost": false });
        call(&state, "POST", &path, Some(&token)).await?;
        // The new card is in, so a failure from here on still counts as saved:
        // the person has both cards rather than neither.
        let mut retired = String::new();
        for (token_id, card, body) in old {
            let marked = call(&state, "PUT", &format!("{path}/{token_id}"), Some(&body)).await;
            retired.push_str(&match marked {
                Ok(_) => format!(" Old card {card} is marked lost."),
                Err(problem) => format!(
                    " Old card {card} was not marked lost: {} Mark it lost in Net2.",
                    problem.message
                ),
            });
        }
        Ok::<_, Failure>(Ok(retired))
    }
    .await;
    SAVING.set(false);
    let mut state = state.borrow_mut();
    if let Some(batch) = &mut state.batch
        && batch.sending_to().is_some_and(|(staged, ..)| staged == id)
    {
        match outcome {
            Ok(Ok(retired)) => {
                batch.saved();
                batch.notice.push_str(&retired);
            }
            Ok(Err(card)) => batch.refused(
                format!(
                    "{name} already has card {card} in Net2, so they keep it. Lift the card and tap it again for the next person."
                ),
                true,
                false,
                false,
            ),
            Err(problem) if problem.uncertain => batch.refused(
                format!("{name}: {} Paused.", problem.message),
                false,
                true,
                true,
            ),
            Err(problem) => batch.refused(
                format!("Not saved for {name}. {}", problem.message),
                false,
                false,
                problem.halt,
            ),
        }
    }
    crate::client::render(&state);
}

async fn connect(state: Rc<RefCell<State>>) {
    let password = value("net2-password");
    let secret = value("net2-secret");
    clear("net2-password");
    clear("net2-secret");
    let origin = match parse_origin(&value("net2-server")) {
        Ok(origin) => origin,
        Err(message) => {
            state.borrow_mut().net2_status = (Tone::Problem, message);
            return crate::client::render(&state.borrow());
        }
    };
    let client_id = value("net2-client").trim().to_string();
    // offline_access asks for a refresh token, so an expired session renews
    // itself instead of stopping a batch for the password again.
    let body = form(&[
        ("grant_type", "password"),
        ("username", &value("net2-user")),
        ("password", &password),
        ("client_id", &client_id),
        ("client_secret", &secret),
        ("scope", "offline_access"),
    ]);
    {
        let mut state = state.borrow_mut();
        state.net2_status = (Tone::Busy, format!("Signing in to {origin}…"));
        crate::client::render(&state);
    }
    let token = fetch(&origin, None, "POST", "/authorization/tokens", body)
        .await
        .and_then(|reply| {
            let refresh = refresh_token(&reply);
            access_token(&reply).map(|token| (token, refresh)).ok_or_else(|| {
                failure(
                    "Net2 did not return an access token. Accounts that need a second sign-in step are not supported yet.",
                    false,
                    false,
                )
            })
        });
    let (token, refresh) = match token.map_err(sign_in_failure) {
        Ok(tokens) => tokens,
        Err(mut problem) => {
            // Some servers echo submitted values in errors.
            for secret in [&password, &secret].into_iter().filter(|s| s.len() > 2) {
                problem.message = problem.message.replace(secret.as_str(), "[hidden]");
            }
            if problem.message == UNREACHABLE {
                problem.message = unreachable(&origin);
            }
            state.borrow_mut().net2_status = (Tone::Problem, problem.message);
            return crate::client::render(&state.borrow());
        }
    };
    if let Some(storage) = storage() {
        let _ = storage.set_item("net2-server", &origin);
        let _ = storage.set_item("net2-client", &client_id);
    }
    {
        let mut state = state.borrow_mut();
        state.net2 = Some(Session {
            origin: origin.clone(),
            token,
            refresh,
            client: (client_id, secret),
        });
        state.net2_status = (Tone::Ready, format!("Connected to {origin}"));
        crate::client::render(&state);
    }
    // Departments only narrow the list; without them, All users still works.
    if let Ok(reply) = call(&state, "GET", "/departments", None).await {
        let list = element("batch-department");
        list.set_inner_html("<option value=\"\">All users</option>");
        let document = web_sys::window().unwrap().document().unwrap();
        for (id, name) in read_departments(&reply) {
            let option = document.create_element("option").unwrap();
            option.set_attribute("value", &id.to_string()).unwrap();
            option.set_text_content(Some(&name));
            list.append_child(&option).unwrap();
        }
    }
}

async fn load_people(state: Rc<RefCell<State>>) {
    let department = element("batch-department").unchecked_into::<HtmlSelectElement>();
    let id = department.value();
    let label = department
        .query_selector("option:checked")
        .ok()
        .flatten()
        .and_then(|option| option.text_content())
        .unwrap_or_default();
    let path = if id.is_empty() {
        "/users".to_string()
    } else {
        format!("/departments/{id}/users")
    };
    let replace = element("batch-replace")
        .unchecked_into::<HtmlInputElement>()
        .checked();
    element("batch-load")
        .unchecked_into::<web_sys::HtmlButtonElement>()
        .set_disabled(true);
    // Expiry dates are in the server's local time; the browser's date stands
    // in for it, as both are on the same site.
    let now = js_sys::Date::new_0();
    let today = format!(
        "{:04}-{:02}-{:02}",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date()
    );
    let result = match call(&state, "GET", &path, None).await {
        Ok(reply) => {
            read_users(&reply).and_then(|users| Batch::from_users(&users, replace, &today))
        }
        Err(problem) => Err(problem.message),
    };
    crate::batch_client::replace_batch(&state, result, format!("Net2 - {label}"));
}

pub fn setup(state: Rc<RefCell<State>>) {
    APP.with(|app| app.set(state.clone()).ok());
    if let Some(storage) = storage() {
        for id in ["net2-server", "net2-client"] {
            if let Ok(Some(saved)) = storage.get_item(id) {
                element(id)
                    .unchecked_into::<HtmlInputElement>()
                    .set_value(&saved);
            }
        }
        // An unknown saved value leaves no option selected; keep the default then.
        let types: HtmlSelectElement = element("batch-token-type").unchecked_into();
        if let Ok(Some(saved)) = storage.get_item("batch-token-type")
            && crate::net2::TOKEN_TYPES
                .iter()
                .any(|(value, _)| *value == saved)
        {
            types.set_value(&saved);
        }
    }
    let callback = Closure::<dyn FnMut()>::new(|| {
        let value = element("batch-token-type")
            .unchecked_into::<HtmlSelectElement>()
            .value();
        if let Some(storage) = storage() {
            let _ = storage.set_item("batch-token-type", &value);
        }
    });
    element("batch-token-type").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();
    let submit_state = state.clone();
    let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.prevent_default();
        spawn_local(connect(submit_state.clone()));
    });
    element("net2-form").set_onsubmit(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let disconnect_state = state.clone();
    on_click("net2-disconnect", move || {
        crate::portrait_client::forget_matches();
        let mut state = disconnect_state.borrow_mut();
        state.net2 = None;
        state.net2_status = (Tone::Idle, "Disconnected".into());
        if let Some(batch) = &mut state.batch
            && batch.net2_ids.is_some()
        {
            batch.running = false;
        }
        crate::client::render(&state);
    });
    on_click("owner-connect", || {
        LOOK_UP.set(true);
        if let Some(app) = APP.with(|app| app.get().cloned()) {
            crate::client::render(&app.borrow());
        }
        element("net2-panel").scroll_into_view();
        element("net2-server").focus().ok();
    });
    on_click("batch-load", move || {
        spawn_local(load_people(state.clone()))
    });
}
