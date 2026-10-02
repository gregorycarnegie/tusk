//! Topcoat renders this complete page at build time; GitHub Pages serves the result.
use topcoat::{
    context::Cx,
    view::{View, ViewExt, component, view},
};

const REPO_URL: &str = "https://github.com/gregorycarnegie/tusk";
const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

pub fn render() -> topcoat::Result<String> {
    let cx = Cx::default();
    Ok(futures_executor::block_on(page(&cx).single())?.render(&cx))
}

fn page(cx: &Cx) -> impl View {
    view! { cx =>
        <!DOCTYPE html>
        <html lang="en">
        <head>
            <meta charset="utf-8">
            <meta name="viewport" content="width=device-width, initial-scale=1">
            <title>"Tusk"</title>
            <meta name="description" content="Read Paxton Net2 tokens in the browser from a USB desktop reader, over WebHID, and send cards and portraits straight to Net2.">
            <link rel="icon" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Crect width='32' height='32' rx='9' fill='%2356AA1C'/%3E%3Cpath d='M9 6c-1 12 5 19 17 20-8-3-12-10-12-20a2.5 2.5 0 0 0-5 0Z' fill='%23fff'/%3E%3C/svg%3E">
            <link rel="preconnect" href="https://fonts.googleapis.com">
            <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin="anonymous">
            <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Martian+Mono:wght@400;600&family=Schibsted+Grotesk:wght@400;500;600;700;800&display=swap">

            <script>(topcoat::view::Unescaped::new_unchecked(r#"try { const t = localStorage.getItem("theme"); if (t) document.documentElement.dataset.theme = t; } catch (e) {}"#))</script>
            <style>(topcoat::view::Unescaped::new_unchecked(include_str!("style.css")))</style>
        </head>
        <body>
        <header class="top">
            <div class="top-inner">
                <span class="brand">
                    tusk_icon()
                    <span class="wordmark">"Tusk"</span>
                    if VERSION.contains('-') {
                        <span class="version pre" title="A pre-release: new features are still being checked on real Net2 systems">(VERSION)</span>
                    } else {
                        <span class="version">(VERSION)</span>
                    }
                </span>
                <nav class="tool-nav" aria-label="Tools">
                    <a id="reader-tab" href="#reader" aria-current="page">card_icon() <span>"Read a card"</span></a>
                    <a id="batch-tab" href="#batch">stack_icon() <span>"Batch assign"</span></a>
                    <a id="portraits-tab" href="#portraits">face_icon() <span>"Portraits"</span></a>
                </nav>
                <div class="top-tools">
                    <a class="icon-button" href=(REPO_URL) aria-label="Source code on GitHub" title="Source code on GitHub">github_icon()</a>
                    <button id="theme" class="icon-button" aria-label="Switch between light and dark" title="Switch between light and dark">theme_icon()</button>
                </div>
            </div>
            <div class="links" aria-label="Connections">
                <div id="reader-panel" class="link">
                    <span class="link-name">"Reader"</span>
                    <p id="status" class="status" data-tone="busy" role="status">
                        <span class="dot"></span><span id="message">"Loading reader support…"</span>
                    </p>
                    <button id="connect" class="primary small" disabled="">"Connect reader"</button>
                </div>
                <div class="link">
                    <span class="link-name">"Net2"</span>
                    <p id="net2-state" class="status" data-tone="idle" role="status">
                        <span class="dot"></span><span id="net2-message">"Not connected"</span>
                    </p>
                    <button id="net2-open" class="secondary small" popovertarget="net2-panel">"Sign in"</button>
                    <button id="net2-disconnect" class="secondary small" hidden="">"Disconnect"</button>
                </div>
            </div>
        </header>
        <main>
            <section id="reader-intro" class="intro">
                <h1>"Read a card"</h1>
                <p>"Present a card to the USB desktop reader to see its Net2 token number and who holds it."</p>
            </section>
            <section id="batch-intro" class="intro" hidden="">
                <h1>"Batch assign cards"</h1>
                <p>"Load people from a Net2 import CSV or straight from Net2, then tap each person’s card in turn."</p>
            </section>
            <section id="portraits-intro" class="intro" hidden="">
                <h1>"Upload portraits"</h1>
                <p>"Name each photo with a Net2 user ID, check who it matches, then upload them all to Net2."</p>
            </section>
            <section id="reading" class="reading empty" aria-label="Card reader">
                <div class="stage">
                    <div class="credential" aria-live="polite">
                        <div class="cred-row">
                            <span class="cred-label">"Net2 token"</span>
                            <span id="kind" class="chip" hidden=""></span>
                            contactless_icon()
                        </div>
                        chip_art()
                        <p id="number" class="number">"--------"</p>
                        <div class="cred-row">
                            <span id="prompt" class="cred-prompt">"Present a card to the reader"</span>
                            <code id="hex" hidden=""></code>
                        </div>
                        <span class="scan" aria-hidden="true"></span>
                    </div>
                    <p id="beta" class="beta-note">
                        "Hitag2 decoding is untested on real fobs. Check this number against Net2 before relying on it."
                    </p>
                </div>
                <aside class="side">
                    <section class="pane panel">
                        <h2 class="pane-title">"Card holder"</h2>
                        <p id="owner" class="owner" aria-live="polite" hidden=""></p>
                        <p class="hint owner-idle">"Each card you tap is looked up in Net2: who holds it, their department, and whether it is marked lost."</p>
                        <button id="owner-connect" class="secondary" popovertarget="net2-panel">"Sign in to Net2"</button>
                    </section>
                    <section class="pane panel">
                        <h2 class="pane-title">"Copy"</h2>
                        <div class="copy-buttons">
                            for (id, label) in [("copy-number", "Net2 number"), ("copy-hex", "Raw hex")] {
                                <button id=(id) class="secondary" disabled="">
                                    <span class="copy-normal">copy_icon()</span>
                                    <span class="copy-done">check_icon()</span>
                                    <span class="copy-normal">(label)</span>
                                    <span class="copy-done">"Copied"</span>
                                </button>
                            }
                        </div>
                    </section>
                </aside>
            </section>
            batch_tool()
            portrait_tool()
            <noscript>"Enable JavaScript to connect to the reader."</noscript>
        </main>
        net2_panel()
        <footer>
            <a href=(REPO_URL)>"Source on GitHub"</a>
            " · MIT licence · Not affiliated with or endorsed by Paxton Access Ltd."
        </footer>
        <script type="module">(topcoat::view::Unescaped::new_unchecked(r#"
          import('./tusk.js').then(({default: init}) => init()).catch(error => {
              document.getElementById('status').dataset.tone = 'problem';
              document.getElementById('message').textContent = 'Could not load reader support - reload the page';
              console.error(error);
          });
        "#))</script>
        </body>
        </html>
    }
}

#[component]
async fn batch_tool() -> topcoat::Result<impl View> {
    Ok(view! {
        <section id="batch-tool" class="workspace" hidden="" aria-label="Batch card assignment">
            <div class="rail panel">
                <div class="field">
                    <label for="batch-source" class="field-label">"Who needs cards?"</label>
                    <select id="batch-source">
                        <option value="csv" selected="">"People in a Net2 import CSV"</option>
                        <option value="net2">"People already in Net2"</option>
                    </select>
                </div>
                <div id="batch-csv" class="field">
                    <label class="drop">
                        upload_icon()
                        <span class="drop-title">"Choose a Net2 import CSV"</span>
                        <span class="drop-hint">"Needs First name, Surname and Card Number columns. Every other field is kept, and the file never leaves this browser."</span>
                        <input id="batch-file" type="file" accept=".csv,text/csv" aria-label="Net2 import CSV">
                    </label>
                </div>
                <div id="batch-net2" class="field" hidden="">
                    <button id="batch-signin" class="secondary wide" popovertarget="net2-panel">"Sign in to Net2 first"</button>
                    <label for="batch-department" class="field-label">"Department"</label>
                    <select id="batch-department"><option value="">"All users"</option></select>
                    <label for="batch-token-type" class="field-label">"Card type"</label>
                    <select id="batch-token-type">
                        for (value, label) in tusk::net2::TOKEN_TYPES {
                            if value == tusk::net2::DEFAULT_TOKEN_TYPE {
                                <option value=(value) selected="">(label)</option>
                            } else {
                                <option value=(value)>(label)</option>
                            }
                        }
                    </select>
                    <button id="batch-load" class="primary wide">"Load people from Net2"</button>
                    <p class="hint">"Each card is saved to its person in Net2 the moment it is tapped, as a Net2 decimal number."</p>
                </div>
                <div class="field options">
                    <span class="field-label">"Options"</span>
                    <label class="checkbox"><input id="batch-replace" type="checkbox"><span>"Give new cards to people who already have one"<small>"Applies to the next people you load. In a CSV the Card Number is replaced; in Net2 the new card is added, and the old one keeps working unless marked lost."</small></span></label>
                    <div id="batch-retire-option" hidden="">
                        <label class="checkbox"><input id="batch-retire" type="checkbox"><span>"Mark their old cards lost"<small>"Once the new card is saved, the person’s other cards are marked lost in Net2."</small></span></label>
                    </div>
                </div>
                <div id="batch-format-field" class="field">
                    <label for="batch-format" class="field-label">"Card number format"</label>
                    <select id="batch-format">
                        <option value="decimal" selected="">"Net2 decimal (recommended)"</option>
                        <option value="hex">"Raw card hex"</option>
                    </select>
                    <p class="hint">"Choose before the first card; numbers already in the file are kept as they are."</p>
                </div>
                <p id="batch-error" class="error" role="alert" hidden=""></p>
            </div>
            <div class="work">
                <div id="batch-session" hidden="">
                    <div class="task panel">
                        <div class="task-head">
                            <p id="batch-file-name" class="task-source"></p>
                            <p id="batch-progress" class="task-progress"></p>
                        </div>
                        <div class="meter" aria-hidden="true"><span id="batch-meter"></span></div>
                        <h2 id="batch-prompt" aria-live="polite"></h2>
                        <p id="batch-notice" role="status"></p>
                        <div class="actions">
                            <button id="batch-start" class="primary">"Start / resume"</button>
                            <button id="batch-pause" class="secondary">"Pause"</button>
                            <button id="batch-skip" class="secondary">"Skip person"</button>
                            <button id="batch-undo" class="secondary">"Undo last step"</button>
                        </div>
                        <p class="hint">"One card per person. Leave each card on the reader until its assignment appears; duplicates are refused. Hitag2 fobs are beta: check their numbers against Net2."</p>
                    </div>
                    <div class="review panel">
                        <div class="review-head">
                            <h2>"Assignments"</h2>
                            <button id="batch-download" class="primary small">download_icon() "Download CSV"</button>
                        </div>
                        <div class="table-scroll" tabindex="0" role="region" aria-label="Assignments">
                            <table>
                                <thead><tr><th scope="col">"Row"</th><th scope="col">"Name"</th><th scope="col">"Card number"</th><th scope="col">"Status"</th></tr></thead>
                                <tbody id="batch-rows"></tbody>
                            </table>
                        </div>
                        <p class="hint review-foot">"Download at any time; skipped and unassigned people keep their original values. Reopen the file to carry on."</p>
                    </div>
                </div>
                <div class="empty-state">
                    stack_icon()
                    <p class="empty-title">"No one queued yet"</p>
                    <p class="hint">"Choose a CSV, or sign in to Net2 and load a department. Each person then comes up in turn: tap their card and the next one appears."</p>
                </div>
            </div>
        </section>
    })
}

/// Sign-in lives in a popover, so it opens from any tab and keeps the page
/// underneath as it was.
#[component]
async fn net2_panel() -> topcoat::Result<impl View> {
    Ok(view! {
        <section id="net2-panel" class="sheet" popover="" role="dialog" aria-labelledby="net2-title">
            <div class="sheet-head">
                <h2 id="net2-title">"Sign in to Net2"</h2>
                <button class="icon-button" popovertarget="net2-panel" popovertargetaction="hide" aria-label="Close">close_icon()</button>
            </div>
            <p id="net2-problem" class="error" role="alert" hidden=""></p>
            <form id="net2-form" class="net2-form">
                <label>"Net2 server"<input id="net2-server" type="url" placeholder="https://net2-server:8443" required="" autocomplete="off" spellcheck="false"></label>
                <label>"Integration client ID"<input id="net2-client" required="" autocomplete="off" spellcheck="false"></label>
                <div class="pair">
                    <label>"Operator name"<input id="net2-user" required="" autocomplete="off"></label>
                    <label>"Password"<input id="net2-password" type="password" required="" autocomplete="off"></label>
                </div>
                <details>
                    <summary>"Client secret, if your integration has one"</summary>
                    <label>"Client secret"<input id="net2-secret" type="password" autocomplete="off"></label>
                </details>
                <p class="hint">"The client ID is the ClientID attribute in your Net2 API licence, not the licence Id. This computer must trust the Net2 server’s certificate. Nothing is sent anywhere but your Net2 server."</p>
                <button id="net2-connect" class="primary" type="submit">"Connect to Net2"</button>
            </form>
        </section>
    })
}

#[component]
async fn portrait_tool() -> topcoat::Result<impl View> {
    Ok(view! {
        <section id="portrait-tool" class="workspace" hidden="" aria-label="Portrait upload">
            <div class="rail panel">
                <div class="field">
                    <label class="drop">
                        upload_icon()
                        <span class="drop-title">"Choose portraits"</span>
                        <span class="drop-hint">"Name each file with the person’s Net2 user ID, like 12345.jpg, not a personnel or card number. Big photos are shrunk and other formats converted to JPG."</span>
                        <input id="portrait-files" type="file" accept="image/*" multiple="" aria-label="Portraits">
                    </label>
                </div>
                <div class="field options">
                    <label class="checkbox"><input id="portrait-replace" type="checkbox"><span>"Replace portraits people already have"</span></label>
                </div>
                <div class="actions stacked">
                    <button id="portrait-review" class="secondary">"Check matches"</button>
                    <button id="portrait-upload" class="primary">"Upload portraits"</button>
                    <button id="portrait-stop" class="secondary" hidden="">"Stop after this one"</button>
                </div>
                <p id="portrait-notice" class="notice" role="status">"Choose portraits to begin. Nothing uploads until you confirm."</p>
            </div>
            <div class="work">
                <div id="portrait-list" class="review panel" hidden="">
                    <div class="table-scroll tall" tabindex="0" role="region" aria-label="Portraits">
                        <table>
                            <thead><tr><th scope="col">"Portrait"</th><th scope="col">"User ID"</th><th scope="col">"Net2 person"</th><th scope="col">"Has portrait"</th><th scope="col">"Result"</th></tr></thead>
                            <tbody id="portrait-rows"></tbody>
                        </table>
                    </div>
                </div>
                <div class="empty-state">
                    face_icon()
                    <p class="empty-title">"No portraits chosen"</p>
                    <p class="hint">"Pick photos on the left, then Check matches: each one is shown beside the Net2 person it will go to, before anything is uploaded."</p>
                </div>
            </div>
        </section>
    })
}

#[component]
async fn chip_art() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg class="chip-art" viewBox="0 0 44 34" aria-hidden="true">
            <rect x=".5" y=".5" width="43" height="33" rx="6"></rect>
            <path d="M15 .5v33M29 .5v33M.5 12H15M29 12h14.5M.5 22H15M29 22h14.5M15 17h14"></path>
        </svg>
    })
}

#[component]
async fn contactless_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg class="contactless" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <path d="M8.5 9.5a4 4 0 0 1 0 5M12 7a7.5 7.5 0 0 1 0 10M15.5 4.5a11 11 0 0 1 0 15"></path>
        </svg>
    })
}

#[component]
async fn card_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="2.5" y="5" width="19" height="14" rx="2.5"></rect><path d="M6.5 15h4"></path>
        </svg>
    })
}

#[component]
async fn stack_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="6" y="3" width="15" height="10" rx="2"></rect><path d="M3 9v8a2 2 0 0 0 2 2h12"></path>
        </svg>
    })
}

#[component]
async fn face_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="3"></rect><circle cx="12" cy="10" r="3"></circle><path d="M6.5 19a6 6 0 0 1 11 0"></path>
        </svg>
    })
}

#[component]
async fn upload_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg class="drop-icon" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 15V4M7.5 8.5 12 4l4.5 4.5M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"></path>
        </svg>
    })
}

#[component]
async fn download_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 4v11M7.5 10.5 12 15l4.5-4.5M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"></path>
        </svg>
    })
}

#[component]
async fn close_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <path d="M6 6l12 12M18 6 6 18"></path>
        </svg>
    })
}

#[component]
async fn tusk_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg class="logo" viewBox="0 0 32 32" aria-hidden="true">
            <rect width="32" height="32" rx="9" fill="var(--brand)"></rect>
            <path d="M9 6c-1 12 5 19 17 20-8-3-12-10-12-20a2.5 2.5 0 0 0-5 0Z" fill="#fff"></path>
        </svg>
    })
}

#[component]
async fn github_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 16 16" aria-hidden="true">
            <path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"></path>
        </svg>
    })
}

/// Both glyphs are drawn; the stylesheet shows the moon in light mode and the
/// sun in dark mode, so the button needs no state of its own.
#[component]
async fn theme_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path class="moon" d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8Z"></path>
            <g class="sun">
                <circle cx="12" cy="12" r="4"></circle>
                <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"></path>
            </g>
        </svg>
    })
}

#[component]
async fn copy_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="12" height="12" rx="2"></rect>
            <path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1"></path>
        </svg>
    })
}

#[component]
async fn check_icon() -> topcoat::Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6 9 17l-5-5"></path>
        </svg>
    })
}

#[cfg(test)]
mod tests {
    /// `client::element` panics on an id the page does not have, so check
    /// every id the browser code looks up by name. Ids that reach `element`
    /// through a loop are not seen here; tests/static_site.py covers those.
    #[test]
    fn every_element_the_browser_code_looks_up_is_in_the_page() {
        let html = super::render().unwrap();
        let sources = [
            include_str!("client.rs"),
            include_str!("batch_client.rs"),
            include_str!("net2_client.rs"),
            include_str!("portrait_client.rs"),
        ];
        let mut checked = 0;
        for source in sources {
            for call in [
                "element(\"",
                "on_click(\"",
                "input(\"",
                "button(\"",
                "disabled(\"",
            ] {
                for (at, _) in source.match_indices(call) {
                    // create_element("tr") is not a lookup
                    if source[..at].ends_with(|c: char| c == '_' || c.is_alphanumeric()) {
                        continue;
                    }
                    let rest = &source[at + call.len()..];
                    let id = &rest[..rest.find('"').unwrap()];
                    assert!(
                        html.contains(&format!("id=\"{id}\"")),
                        "no #{id} in the page"
                    );
                    checked += 1;
                }
            }
        }
        // A pattern that stopped matching would pass silently otherwise.
        assert!(checked > 50, "only {checked} lookups found");
    }
}
