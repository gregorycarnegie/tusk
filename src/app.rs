//! The page: status, the token on the reader, and the buttons around it.

use leptos::{prelude::*, reactive::owner::StoredValue, task::spawn_local};
use web_sys::{HidDeviceFilter, HidDeviceRequestOptions};

use crate::{
    reader::{Link, PAXTON_VID, Tone, hid, reason, run, say},
    token::{Read, Token},
};

const REPO_URL: &str = "https://github.com/gregorycarnegie/tusk";

#[component]
pub fn App() -> impl IntoView {
    let card = RwSignal::new(None::<Token>);
    let status = RwSignal::new((Tone::Idle, "Not connected".to_string()));
    let link = StoredValue::new_local(Link::default());

    let has_hid = js_sys::Reflect::has(&web_sys::window().unwrap().navigator(), &"hid".into())
        .unwrap_or(false);
    if !has_hid {
        say(
            status,
            Tone::Problem,
            "WebHID is not available - open this page in Chrome or Edge",
        );
    }

    // reuse a reader the browser already has permission for, so a reload
    // does not mean clicking through the picker again
    spawn_local(async move {
        if !has_hid {
            return;
        }
        if let Ok(devices) = hid().get_devices().await
            && let Some(dev) = devices
                .iter()
                .find(|d| u32::from(d.vendor_id()) == PAXTON_VID)
        {
            run(dev, card, status, link).await;
        }
    });

    let connect = move |_| {
        spawn_local(async move {
            if !has_hid {
                return;
            }
            // already polling: the picker is only for granting permission, so
            // say so rather than appearing to do nothing
            if link.with_value(|l| l.dev.as_ref().is_some_and(|d| d.opened())) {
                say(status, Tone::Ready, "Already connected - present a token");
                return;
            }
            let filter = HidDeviceFilter::new();
            filter.set_vendor_id(PAXTON_VID);
            let opts = HidDeviceRequestOptions::new(&[filter]);
            match hid().request_device(&opts).await {
                Ok(devices) => match devices.get_checked(0) {
                    Some(dev) => run(dev, card, status, link).await,
                    None => say(status, Tone::Idle, "No reader selected"),
                },
                Err(e) => say(
                    status,
                    Tone::Problem,
                    format!("Could not reach the reader: {}", reason(&e)),
                ),
            }
        })
    };

    // what was last put on the clipboard, so the button that did it can say so
    // until a different token turns up
    let copied = RwSignal::new(String::new());
    let copy_button = move |label: &'static str, pick: fn(&Token) -> String| {
        let copy = move |_| {
            let Some(text) = card.with_untracked(|c| c.as_ref().map(pick)) else {
                return;
            };
            spawn_local(async move {
                let clipboard = web_sys::window().unwrap().navigator().clipboard();
                if clipboard.write_text(&text).await.is_ok() {
                    copied.set(text);
                }
            });
        };
        let done = move || card.with(|c| c.as_ref().map(pick)) == Some(copied.get());
        view! {
            <button class="secondary" disabled=move || card.with(|c| c.is_none()) on:click=copy>
                {move || if done() { check_icon().into_any() } else { copy_icon().into_any() }}
                {move || if done() { "Copied" } else { label }}
            </button>
        }
    };

    view! {
        <header class="top">
            <span class="brand">{tusk_icon()} "Tusk"</span>
            <nav>
                <a class="icon-button" href=REPO_URL aria-label="Source code on GitHub" title="Source code on GitHub">
                    {github_icon()}
                </a>
                <button class="icon-button" on:click=|_| toggle_theme() aria-label="Switch between light and dark" title="Switch between light and dark">
                    {theme_icon()}
                </button>
            </nav>
        </header>
        <main>
            <section class="intro">
                <h1>"Read Net2 tokens in your browser"</h1>
                <p>"Plug in a Paxton Net2 USB desktop reader, connect it, and present a card. No drivers or Net2 software needed."</p>
            </section>
            <section class="panel">
                <div class="panel-head">
                    <p class="status" data-tone=move || match status.with(|s| s.0) {
                        Tone::Idle => "idle",
                        Tone::Busy => "busy",
                        Tone::Ready => "ready",
                        Tone::Problem => "problem",
                    }>
                        <span class="dot"></span>
                        {move || status.with(|s| s.1.clone())}
                    </p>
                    <button class="primary" on:click=connect>"Connect reader"</button>
                </div>
                <div class="reading" class:empty=move || card.with(|c| c.is_none())>
                    <p class="label">"Net2 token number"</p>
                    <p class="number">
                        {move || card.with(|c| c.as_ref().map_or("--------".to_string(), |t| t.number.to_string()))}
                    </p>
                    <p class="meta">
                        {move || match card.get() {
                            Some(t) => view! {
                                <span class="chip" class:beta=t.read == Read::Hitag2>
                                    {if t.read == Read::Hitag2 { "Hitag2 · beta" } else { "Mifare" }}
                                </span>
                                <code>{t.hex}</code>
                            }.into_any(),
                            None => view! { <span>"Present a card to the reader"</span> }.into_any(),
                        }}
                    </p>
                    <p class="beta-note" class:shown=move || card.with(|c| c.as_ref().is_some_and(|t| t.read == Read::Hitag2))>
                        "Hitag2 decoding is untested on real fobs. Check this number against Net2 before relying on it."
                    </p>
                    <div class="actions">
                        {copy_button("Copy number", |t| t.number.to_string())}
                        {copy_button("Copy hex", |t| t.hex.clone())}
                    </div>
                </div>
            </section>
        </main>
        <footer>
            <a href=REPO_URL>"Source on GitHub"</a>
            " · MIT licence · Not affiliated with or endorsed by Paxton Access Ltd."
        </footer>
    }
}

/// Flip between light and dark, starting from whatever is showing now, and
/// remember the choice. index.html reapplies it before the first paint.
fn toggle_theme() {
    let window = web_sys::window().unwrap();
    let Some(root) = window.document().and_then(|d| d.document_element()) else {
        return;
    };
    let dark = match root.get_attribute("data-theme") {
        Some(theme) => theme == "dark",
        None => window
            .match_media("(prefers-color-scheme: dark)")
            .ok()
            .flatten()
            .is_some_and(|m| m.matches()),
    };
    let next = if dark { "light" } else { "dark" };
    let _ = root.set_attribute("data-theme", next);
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item("theme", next);
    }
}

fn tusk_icon() -> impl IntoView {
    view! {
        <svg class="logo" viewBox="0 0 32 32" aria-hidden="true">
            <rect width="32" height="32" rx="9" fill="var(--brand)" />
            <path d="M9 6c-1 12 5 19 17 20-8-3-12-10-12-20a2.5 2.5 0 0 0-5 0Z" fill="#fff" />
        </svg>
    }
}

fn github_icon() -> impl IntoView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true">
            <path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
        </svg>
    }
}

/// Both glyphs are drawn; the stylesheet shows the moon in light mode and the
/// sun in dark mode, so the button needs no state of its own.
fn theme_icon() -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path class="moon" d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8Z" />
            <g class="sun">
                <circle cx="12" cy="12" r="4" />
                <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
            </g>
        </svg>
    }
}

fn copy_icon() -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="12" height="12" rx="2" />
            <path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1" />
        </svg>
    }
}

fn check_icon() -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6 9 17l-5-5" />
        </svg>
    }
}
