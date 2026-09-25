//! Topcoat renders this complete page at build time; GitHub Pages serves the result.
use topcoat::{
    context::Cx,
    view::{View, ViewExt, component, view},
};

const REPO_URL: &str = "https://github.com/gregorycarnegie/tusk";

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
            <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Source+Sans+3:wght@400;600;700&display=swap">

            <script>(topcoat::view::Unescaped::new_unchecked(r#"try { const t = localStorage.getItem("theme"); if (t) document.documentElement.dataset.theme = t; } catch (e) {}"#))</script>
            <style>(topcoat::view::Unescaped::new_unchecked(include_str!("style.css")))</style>
        </head>
        <body>
        <header class="top">
            <span class="brand">tusk_icon() "Tusk" <span class="version">(concat!("v", env!("CARGO_PKG_VERSION")))</span></span>
            <nav>
                <a class="icon-button" href=(REPO_URL) aria-label="Source code on GitHub" title="Source code on GitHub">github_icon()</a>
                <button id="theme" class="icon-button" aria-label="Switch between light and dark" title="Switch between light and dark">theme_icon()</button>
            </nav>
        </header>
        <main>
            <nav class="tool-nav" aria-label="Tools">
                <a id="reader-tab" href="#reader" aria-current="page">"Read a card"</a>
                <a id="batch-tab" href="#batch">"Batch assign cards"</a>
                <a id="portraits-tab" href="#portraits">"Upload portraits"</a>
            </nav>
            <section id="reader-intro" class="intro">
                <h1>"Read Net2 tokens in your browser"</h1>
                <p>"Plug in a Paxton Net2 USB desktop reader, connect it, and present a card. No drivers or Net2 software needed."</p>
            </section>
            <section id="batch-intro" class="intro" hidden="">
                <h1>"A card for every name"</h1>
                <p>"Load people from a Net2 import CSV or straight from Net2, then tap each person’s card."</p>
            </section>
            <section id="portraits-intro" class="intro" hidden="">
                <h1>"A face for every card"</h1>
                <p>"Name each photo with a Net2 user ID, check who it matches, then upload them all to Net2."</p>
            </section>
            <section id="reader-panel" class="panel">
                <div class="panel-head">
                    <p id="status" class="status" data-tone="busy" role="status">
                        <span class="dot"></span><span id="message">"Loading reader support…"</span>
                    </p>
                    <button id="connect" class="primary" disabled="">"Connect reader"</button>
                </div>
                <div id="reading" class="reading empty" aria-live="polite">
                    <p class="label">"Net2 token number"</p>
                    <p id="number" class="number">"--------"</p>
                    <p class="meta">
                        <span id="prompt">"Present a card to the reader"</span>
                        <span id="kind" class="chip" hidden=""></span>
                        <code id="hex" hidden=""></code>
                    </p>
                    <p id="beta" class="beta-note">
                        "Hitag2 decoding is untested on real fobs. Check this number against Net2 before relying on it."
                    </p>
                    <div class="actions">
                        for (id, label) in [("copy-number", "Copy number"), ("copy-hex", "Copy hex")] {
                            <button id=(id) class="secondary" disabled="">
                                <span class="copy-normal">copy_icon()</span>
                                <span class="copy-done">check_icon()</span>
                                <span class="copy-normal">(label)</span>
                                <span class="copy-done">"Copied"</span>
                            </button>
                        }
                    </div>
                </div>
            </section>
            net2_panel()
            batch_tool()
            portrait_tool()
            <noscript>"Enable JavaScript to connect to the reader."</noscript>
        </main>
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
        <section id="batch-tool" hidden="" aria-label="Batch card assignment">
            <div class="batch-setup panel">
                <label for="batch-source">"1. Who needs cards?"</label>
                <select id="batch-source">
                    <option value="csv" selected="">"People in a Net2 import CSV"</option>
                    <option value="net2">"People already in Net2"</option>
                </select>
                <div id="batch-csv">
                    <input id="batch-file" type="file" accept=".csv,text/csv" aria-label="Net2 import CSV">
                    <p class="hint">"Needs First name, Surname and Card Number columns. All other fields are preserved. Your file stays in this browser."</p>
                </div>
                <div id="batch-net2" hidden="">
                    <label for="batch-department">"Department"</label>
                    <select id="batch-department"><option value="">"All users"</option></select>
                    <label for="batch-token-type">"Card type"</label>
                    <select id="batch-token-type">
                        for (value, label) in tusk::net2::TOKEN_TYPES {
                            if value == tusk::net2::DEFAULT_TOKEN_TYPE {
                                <option value=(value) selected="">(label)</option>
                            } else {
                                <option value=(value)>(label)</option>
                            }
                        }
                    </select>
                    <button id="batch-load" class="primary">"Load people from Net2"</button>
                    <p class="hint">"Each card is saved to its person in Net2 the moment it is tapped, as a Net2 decimal number. Download CSV still gives you a record of the session."</p>
                </div>
                <label class="checkbox"><input id="batch-replace" type="checkbox">"Give new cards to people who already have one"</label>
                <p class="hint">"Applies to the next people you load. Otherwise they keep their card. In a CSV the Card Number is replaced; in Net2 the new card is added and the old one keeps working until you remove it there."</p>
                <label for="batch-format">"Card number format"</label>
                <select id="batch-format">
                    <option value="decimal" selected="">"Net2 decimal (recommended)"</option>
                    <option value="hex">"Raw card hex"</option>
                </select>
                <p class="hint">"Format applies to newly scanned cards. Choose it before assigning the first card; existing values are kept as supplied."</p>
                <p id="batch-error" class="error" role="alert" hidden=""></p>
            </div>
            <div id="batch-session" hidden="">
                <div class="batch-task panel">
                    <p id="batch-file-name" class="hint"></p>
                    <p id="batch-progress" class="label"></p>
                    <h2 id="batch-prompt" aria-live="polite"></h2>
                    <p id="batch-notice" role="status"></p>
                    <div class="actions">
                        <button id="batch-start" class="primary">"Start / resume"</button>
                        <button id="batch-pause" class="secondary">"Pause"</button>
                        <button id="batch-skip" class="secondary">"Skip person"</button>
                        <button id="batch-undo" class="secondary">"Undo last step"</button>
                    </div>
                    <p class="hint">"One card per person. Leave each card on the reader until its assignment appears. Duplicate cards are refused."</p>
                    <p class="beta-note shown">"Hitag2 fobs are still beta: verify their numbers against Net2."</p>
                </div>
                <div class="batch-review panel">
                    <div class="panel-head">
                        <h2>"Review assignments"</h2>
                        <button id="batch-download" class="primary">"Download CSV"</button>
                    </div>
                    <p class="hint">"You can download progress at any time. Skipped and unassigned people keep their original values. Reopen the downloaded CSV to continue with empty card numbers."</p>
                    <div class="table-scroll" tabindex="0" role="region" aria-label="Assignments">
                        <table>
                            <thead><tr><th scope="col">"CSV row"</th><th scope="col">"Name"</th><th scope="col">"Card Number"</th><th scope="col">"Status"</th></tr></thead>
                            <tbody id="batch-rows"></tbody>
                        </table>
                    </div>
                </div>
            </div>
        </section>
    })
}

#[component]
async fn net2_panel() -> topcoat::Result<impl View> {
    Ok(view! {
        <section id="net2-panel" class="panel" hidden="" aria-label="Net2 connection">
            <div class="panel-head">
                <p id="net2-state" class="status" data-tone="idle" role="status">
                    <span class="dot"></span><span id="net2-message">"Not connected"</span>
                </p>
                <button id="net2-disconnect" class="secondary" hidden="">"Disconnect"</button>
            </div>
            <form id="net2-form" class="net2-form">
                <label>"Net2 server"<input id="net2-server" type="url" placeholder="https://net2-server:8443" required="" autocomplete="off" spellcheck="false"></label>
                <label>"Integration client ID"<input id="net2-client" required="" autocomplete="off" spellcheck="false"></label>
                <label>"Operator name"<input id="net2-user" required="" autocomplete="off"></label>
                <label>"Password"<input id="net2-password" type="password" required="" autocomplete="off"></label>
                <details>
                    <summary>"Client secret, if your integration has one"</summary>
                    <label>"Client secret"<input id="net2-secret" type="password" autocomplete="off"></label>
                </details>
                <p class="hint">"The client ID is the ClientID attribute inside your Net2 API licence, not the licence Id. This computer must trust the Net2 server’s certificate, and Chrome may ask to let this page reach devices on your network. Nothing is sent anywhere but your Net2 server."</p>
                <button id="net2-connect" class="primary" type="submit">"Connect to Net2"</button>
            </form>
        </section>
    })
}

#[component]
async fn portrait_tool() -> topcoat::Result<impl View> {
    Ok(view! {
        <section id="portrait-tool" hidden="" aria-label="Portrait upload">
            <div class="batch-setup panel">
                <label for="portrait-files">"Choose portraits"</label>
                <input id="portrait-files" type="file" accept="image/*" multiple="">
                <p class="hint">"Name each file with the person’s Net2 user ID, like 12345.jpg: not a personnel or card number. Any photo this browser can open works; big photos are shrunk and other formats converted to JPG."</p>
                <label class="checkbox"><input id="portrait-replace" type="checkbox">"Replace portraits people already have"</label>
                <div class="actions">
                    <button id="portrait-review" class="secondary">"Check matches"</button>
                    <button id="portrait-upload" class="primary">"Upload portraits"</button>
                    <button id="portrait-stop" class="secondary" hidden="">"Stop after this one"</button>
                </div>
                <p id="portrait-notice" class="notice" role="status">"Choose portraits to begin. Nothing uploads until you confirm."</p>
            </div>
            <div id="portrait-list" class="batch-review panel" hidden="">
                <div class="table-scroll" tabindex="0" role="region" aria-label="Portraits">
                    <table>
                        <thead><tr><th scope="col">"Portrait"</th><th scope="col">"User ID"</th><th scope="col">"Net2 person"</th><th scope="col">"Has portrait"</th><th scope="col">"Result"</th></tr></thead>
                        <tbody id="portrait-rows"></tbody>
                    </table>
                </div>
            </div>
        </section>
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
