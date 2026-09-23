//! Browser file handling and controls for the CSV assignment tool.
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    BeforeUnloadEvent, Blob, BlobPropertyBag, HtmlAnchorElement, HtmlButtonElement,
    HtmlInputElement, HtmlSelectElement, Url,
};

use crate::{
    batch::{Batch, Format},
    client::{element, on_click},
    reader::{State, Tone, Tool},
};

fn input(id: &str) -> HtmlInputElement {
    element(id).unchecked_into()
}
fn disabled(id: &str, disabled: bool) {
    element(id)
        .unchecked_into::<HtmlButtonElement>()
        .set_disabled(disabled);
}

pub fn render(state: &State) {
    let tool = state.tool;
    element("reader-intro").set_hidden(tool != Tool::Reader);
    element("reading").set_hidden(tool != Tool::Reader);
    element("reader-panel").set_hidden(tool == Tool::Portraits);
    element("batch-intro").set_hidden(tool != Tool::Batch);
    element("batch-tool").set_hidden(tool != Tool::Batch);
    element("portraits-intro").set_hidden(tool != Tool::Portraits);
    for (id, tab) in [
        ("reader-tab", Tool::Reader),
        ("batch-tab", Tool::Batch),
        ("portraits-tab", Tool::Portraits),
    ] {
        element(id)
            .set_attribute("aria-current", if tab == tool { "page" } else { "false" })
            .unwrap();
    }
    let net2_source = element("batch-source")
        .unchecked_into::<HtmlSelectElement>()
        .value()
        == "net2";
    element("batch-csv").set_hidden(net2_source);
    element("batch-net2").set_hidden(!net2_source);
    let format: HtmlSelectElement = element("batch-format").unchecked_into();
    // Net2 stores the decimal number, so a direct queue has no choice to make.
    format.set_disabled(net2_source);
    if net2_source {
        format.set_value("decimal");
    }
    element("batch-error").set_hidden(state.batch_error.is_empty());
    element("batch-error").set_text_content(Some(&state.batch_error));
    element("batch-session").set_hidden(state.batch.is_none());
    let Some(batch) = &state.batch else { return };
    element("batch-file-name").set_text_content(Some(&state.batch_filename));
    let assigned = batch.assigned_count();
    let skipped = batch.rows.iter().filter(|r| r.skipped).count();
    let kept = (0..batch.rows.len())
        .filter(|&i| batch.row_status(i) == "Kept")
        .count();
    element("batch-progress").set_text_content(Some(&format!(
        "{} people · {assigned} assigned · {kept} kept · {skipped} skipped",
        batch.rows.len()
    )));
    let current = batch.current();
    element("batch-prompt").set_text_content(Some(&match current {
        Some(i) if batch.running => format!("Please tap {}’s card", batch.name(i)),
        Some(i) => format!("Next: {}", batch.name(i)),
        None => "Queue complete — review and download your CSV".into(),
    }));
    let notice = if state.status.0 != Tone::Ready && current.is_some() {
        "Connect the reader, then press Start / resume. Your progress is kept."
    } else {
        &batch.notice
    };
    element("batch-notice").set_text_content(Some(notice));
    let sending = batch.sending.is_some();
    disabled(
        "batch-start",
        current.is_none() || batch.running || state.status.0 != Tone::Ready,
    );
    disabled("batch-pause", !batch.running || sending);
    disabled("batch-skip", current.is_none() || sending);
    disabled("batch-undo", !batch.can_undo());
    input("batch-file").set_disabled(sending);
    format.set_disabled(assigned != 0 || batch.net2_ids.is_some());
    format.set_value(if batch.format == Format::Hex {
        "hex"
    } else {
        "decimal"
    });
    // Reuse rows so an unchanged reader status does not rebuild hundreds of nodes.
    let body = element("batch-rows");
    let document = web_sys::window().unwrap().document().unwrap();
    if body.child_element_count() as usize != batch.rows.len() {
        body.set_text_content(None);
        for _ in &batch.rows {
            let row = document.create_element("tr").unwrap();
            for _ in 0..4 {
                row.append_child(&document.create_element("td").unwrap())
                    .unwrap();
            }
            body.append_child(&row).unwrap();
        }
    }
    let mut row = body.first_element_child();
    for i in 0..batch.rows.len() {
        let node = row.unwrap();
        node.set_class_name(if current == Some(i) {
            "current-person"
        } else {
            ""
        });
        let mut cell = node.first_element_child();
        for value in [
            (i + 2).to_string(),
            batch.name(i),
            batch.value(i),
            batch.row_status(i).into(),
        ] {
            let node = cell.unwrap();
            if node.text_content().as_deref() != Some(&value) {
                node.set_text_content(Some(&value));
            }
            cell = node.next_element_sibling();
        }
        row = node.next_element_sibling();
    }
}

pub fn setup(state: Rc<RefCell<State>>) {
    let navigate = {
        let state = state.clone();
        move || {
            let mut state = state.borrow_mut();
            let hash = web_sys::window()
                .unwrap()
                .location()
                .hash()
                .unwrap_or_default();
            state.tool = match hash.as_str() {
                "#batch" => Tool::Batch,
                "#portraits" => Tool::Portraits,
                _ => Tool::Reader,
            };
            if state.tool != Tool::Batch
                && let Some(batch) = &mut state.batch
            {
                batch.running = false;
            }
            crate::client::render(&state);
        }
    };
    navigate();
    let callback = Closure::<dyn FnMut()>::new(navigate);
    web_sys::window()
        .unwrap()
        .set_onhashchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let file_state = state.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        let Some(file) = input("batch-file").files().and_then(|files| files.get(0)) else {
            return;
        };
        let state = file_state.clone();
        let replace = input("batch-replace").checked();
        let format = selected_format();
        input("batch-file").set_disabled(true);
        input("batch-replace").set_disabled(true);
        element("batch-format")
            .unchecked_into::<HtmlSelectElement>()
            .set_disabled(true);
        // Stop collecting cards as soon as a different file is selected.
        if let Some(batch) = &mut state.borrow_mut().batch {
            batch.running = false;
        }
        spawn_local(async move {
            let result = if file.size() > 10.0 * 1024.0 * 1024.0 {
                Err("Please use a CSV smaller than 10 MB.".into())
            } else {
                match file.array_buffer().await {
                    Ok(buffer) => {
                        Batch::parse(&js_sys::Uint8Array::new(&buffer).to_vec(), replace, format)
                    }
                    Err(_) => Err("Could not read that file. Choose it again.".into()),
                }
            };
            input("batch-file").set_disabled(false);
            input("batch-file").set_value("");
            input("batch-replace").set_disabled(false);
            element("batch-format")
                .unchecked_into::<HtmlSelectElement>()
                .set_disabled(false);
            replace_batch(&state, result, file.name());
        });
    });
    input("batch-file").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let source_state = state.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        crate::client::render(&source_state.borrow());
    });
    element("batch-source").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    let format_state = state.clone();
    let callback = Closure::<dyn FnMut()>::new(move || {
        let mut state = format_state.borrow_mut();
        if let Some(batch) = &mut state.batch
            && batch.assigned_count() == 0
        {
            batch.format = selected_format();
        }
        crate::client::render(&state);
    });
    element("batch-format").set_onchange(Some(callback.as_ref().unchecked_ref()));
    callback.forget();

    for action in [
        "batch-start",
        "batch-pause",
        "batch-skip",
        "batch-undo",
        "batch-download",
    ] {
        let state = state.clone();
        on_click(action, move || {
            let mut state = state.borrow_mut();
            let card = state.card.clone();
            let ready = state.status.0 == Tone::Ready;
            let filename = state.batch_filename.clone();
            if let Some(batch) = &mut state.batch {
                match action {
                    "batch-start" if ready => batch.resume(card.as_ref()),
                    "batch-pause" => {
                        batch.running = false;
                        batch.notice = "Paused. Press Start / resume when ready.".into();
                    }
                    "batch-skip" => batch.skip(),
                    "batch-undo" => batch.undo(),
                    "batch-download" => match download(batch, &filename) {
                        Ok(()) => {
                            batch.dirty = false;
                            batch.notice =
                                "CSV downloaded. Keep this copy of your progress.".into();
                        }
                        Err(error) => state.batch_error = error,
                    },
                    _ => {}
                }
            }
            crate::client::render(&state);
        });
    }

    let callback = Closure::<dyn FnMut(BeforeUnloadEvent)>::new(move |event: BeforeUnloadEvent| {
        if state.borrow().batch.as_ref().is_some_and(|b| b.dirty) {
            event.prevent_default();
            event.set_return_value("");
        }
    });
    web_sys::window()
        .unwrap()
        .set_onbeforeunload(Some(callback.as_ref().unchecked_ref()));
    callback.forget();
}

/// Swap in newly loaded people, unless that would lose unsaved assignments.
pub fn replace_batch(state: &RefCell<State>, result: Result<Batch, String>, name: String) {
    let discard = !state.borrow().batch.as_ref().is_some_and(|b| b.dirty)
        || result.is_err()
        || web_sys::window()
            .unwrap()
            .confirm_with_message(
                "Replace this batch? Assignments since your last download will be lost.",
            )
            .unwrap_or(false);
    let mut state = state.borrow_mut();
    if discard {
        match result {
            Ok(batch) => {
                state.batch = Some(batch);
                state.batch_filename = name;
                state.batch_error.clear();
            }
            Err(error) => state.batch_error = error,
        }
    }
    crate::client::render(&state);
}

fn selected_format() -> Format {
    if element("batch-format")
        .unchecked_into::<HtmlSelectElement>()
        .value()
        == "hex"
    {
        Format::Hex
    } else {
        Format::Decimal
    }
}

fn download(batch: &Batch, filename: &str) -> Result<(), String> {
    let bytes = batch.export()?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
    let options = BlobPropertyBag::new();
    options.set_type("text/csv;charset=utf-8");
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|_| "Could not prepare the download.")?;
    let url =
        Url::create_object_url_with_blob(&blob).map_err(|_| "Could not create a download link.")?;
    let document = web_sys::window().unwrap().document().unwrap();
    let anchor: HtmlAnchorElement = document.create_element("a").unwrap().unchecked_into();
    anchor.set_href(&url);
    anchor.set_download(&format!(
        "{}-cards.csv",
        filename.rsplit_once('.').map_or(filename, |(base, _)| base)
    ));
    anchor.click();
    // Give the browser time to start the download before releasing its data.
    let release = Closure::once_into_js(move || {
        let _ = Url::revoke_object_url(&url);
    });
    web_sys::window()
        .unwrap()
        .set_timeout_with_callback_and_timeout_and_arguments_0(release.unchecked_ref(), 1000)
        .map_err(|_| "Could not finish the download.")?;
    Ok(())
}
