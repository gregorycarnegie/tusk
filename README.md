# Tusk

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.98%2B-orange.svg)](https://www.rust-lang.org)
[![Topcoat](https://img.shields.io/badge/topcoat-0.8.1-56AA1C.svg)](https://github.com/tokio-rs/topcoat)
[![Browser](https://img.shields.io/badge/browser-Chrome%20%7C%20Edge-4285f4.svg)](https://developer.mozilla.org/en-US/docs/Web/API/WebHID_API)
[![CI](https://github.com/gregorycarnegie/tusk/actions/workflows/ci.yml/badge.svg)](https://github.com/gregorycarnegie/tusk/actions/workflows/ci.yml)

Read tokens from a Paxton Net2 USB desktop reader in the browser, over WebHID.
No drivers, no Net2 software, no server — put a card on the reader and the
token number appears, as Net2 would show it.

It can also talk to Net2 itself: upload ID card portraits, and save each
tapped card straight to the right person, with no CSV import in between.
See [Working with Net2 directly](#working-with-net2-directly).

**Try it: <https://gregorycarnegie.github.io/tusk/>** — in Chrome or Edge,
with the reader plugged in. Nothing to install.

Mifare cards are tested. Paxton's own Hitag2 fobs are **beta**: decoded the
way Net2 does, but not yet tried on a real fob, so check the number against
Net2 before relying on it.

```
┌──────────────────────────────────────────┐
│ ● Ready - present a token  (Connect)     │
├──────────────────────────────────────────┤
│            NET2 TOKEN NUMBER             │
│                34935097                  │
│            Mifare  5B7D4039              │
│      [Copy number]  [Copy hex]           │
└──────────────────────────────────────────┘
```

Light and dark themes follow the system setting, with a toggle in the header.

The reader speaks an undocumented, lightly obfuscated HID protocol. Working it
out was most of this project; [PROTOCOL.md](PROTOCOL.md) is the write-up.

## Requirements

- **Chrome or Edge.** WebHID is not available in Firefox or Safari, and the
  page says so rather than failing silently.
- **A Paxton Net2 USB desktop reader** (`USB\VID_1071&PID_0001`).
- **On Linux, a udev rule.** Chrome reaches the reader through
  `/dev/hidrawN`, which is root-only by default, so the device either never
  appears in the chooser or fails to open. Windows needs nothing here; Linux
  needs this once:

  ```sh
  sudo tee /etc/udev/rules.d/70-paxton-net2.rules <<'EOF'
  SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1071", ATTRS{idProduct}=="0001", TAG+="uaccess"
  EOF
  sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw
  ```

  `uaccess` grants an ACL to whoever is logged in at the active seat, the same
  mechanism udev uses for webcams.
- **Rust 1.98+** with the `wasm32-unknown-unknown` target, and
  `wasm-bindgen-cli` matching the `wasm-bindgen` version in `Cargo.lock`.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
```

## Running

```sh
cargo run --locked -- dist
cargo build --locked --release --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir dist target/wasm32-unknown-unknown/release/tusk.wasm
python -m http.server 8000 --directory dist --bind 127.0.0.1
```

Open <http://127.0.0.1:8000>, click **Connect reader**, and pick the device. (Not 8080 on a Net2 server:
Net2's own web server uses that port for its setup page.)
The picker shows it under an unreadable name — that is the reader returning
uninitialised memory as its USB product string, and it differs every time it
enumerates. Filtering on Paxton's vendor ID at least leaves it as the only
entry in the list.

After the first connection the browser remembers permission for that device,
so reloading reconnects without a prompt. After unplugging it, click **Connect
reader** again: the reader has no USB serial number, so Chrome forgets the
permission when it is unplugged and the page cannot reconnect by itself.

## Assign a batch of cards

Open **Batch assign cards** (or `/#batch` locally and `/tusk/#batch` on Pages).

1. Connect the reader and choose a Net2 import **CSV UTF-8** file with
   `First name`, `Surname`, and `Card Number` columns.
2. Existing card numbers are kept by default. To reissue cards, select
   **Replace existing card numbers** before loading the file.
3. Leave the format on **Net2 decimal**, or select **Raw card hex** before
   assigning the first card. Decimal uses the same Net2 conversion as the reader;
   hex writes the full raw hex shown by the reader. Other CSV values are unchanged.
4. Press **Start / resume**, then tap the named person's card. Wait for the
   assignment to appear before moving to the next card. A held card is accepted
   once; cards already used elsewhere in the batch are refused.
5. Use **Skip person** for an absent student, **Undo last step** to correct a
   mistake, and **Download CSV** to save the result for Net2's import wizard.

Download works before the queue is complete, so you can save progress. The file
keeps its columns, order, leading zeros, Unicode, quoted/multiline fields, and
trailing empty fields; only newly assigned `Card Number` values change. CSV
quoting is normalized and rows use CRLF. Skipped/unassigned people retain their
original values. Reopen a partial download with replacement off to fill remaining
empty card numbers. The tool accepts up to 10 MB and 10,000 people per file.

Processing stays in the browser. The batch pauses on reader problems or when
switching back to **Read a card**. Unsaved work triggers a leave-page warning;
there is no persistent storage, so download regularly before closing the tab.
Hitag2 decoding remains beta and should be checked against Net2 on real fobs.

## Working with Net2 directly

**Upload portraits** and the **People already in Net2** option under **Batch
assign cards** sign in to the Net2 Local API from the page itself. Net2's
own web server allows this; nothing passes through any other server, and
the access token is held in memory only, so reloading signs you out.

You need:

- Net2 with **LocalAPI enabled** in the Net2 Configuration Utility, and the
  Net2 Service and Net2 Nginx Service running.
- An **API licence** installed on the Net2 server, and its **ClientID**. Open
  the `.lic` file in `C:\Program Files (x86)\Paxton Access\Access Control\ApiLicences`
  and copy the value of `<Attribute name="ClientID">`. The licence's own
  `<Id>` looks similar and gives `invalid_client`.
- A Net2 **operator** allowed to view users and edit tokens and portraits.
  Accounts that need a second sign-in step (MFA) are not supported yet.
- A browser that **trusts the Net2 server's certificate**, with the server
  entered by a name the certificate covers (`https://net2-server:8443`). On the
  Net2 server itself, `https://localhost:8443` works. Chrome may also ask to
  let the page reach devices on your local network; allow it.

**Portraits.** Name each JPG or PNG with the person's Net2 user ID
(`12345.jpg`, no leading zeros, at most 3.5 MB and 40 megapixels). Choose the
files, press **Check matches**, and compare every name with its photo. People
who already have a portrait are skipped unless you tick **Replace portraits
people already have**. **Upload portraits** asks for confirmation; each upload
re-reads the person first and stops if they changed since the check. If a
result says Net2 did not confirm the change, look in Net2 before trying again.

**Cards straight to Net2.** Choose **People already in Net2**, connect, pick a
department (or all users) and press **Load people from Net2**. Each tap is
saved to that person as a Net2 decimal number. Anyone who already has a card
keeps it and is passed over, unless **Give new cards to people who already
have one** was ticked when you loaded them. In that case the new card is added
and the old one keeps working until you remove it in Net2. Saved cards cannot
be undone from Tusk; remove them in Net2. **Download CSV** still gives you a
record with each person's Net2 user ID.

If signing in says **Could not reach Net2**, open the server address in a
browser tab. A VPN is a common cause: a server name can resolve to the VPN
adapter's address, where the connection is dropped. On the Net2 server itself,
use `https://localhost:8443`.

Choose the **Card type** Net2 should record, as in Net2's Change token type
dialog. It defaults to Proximity ISO card no magstripe, and the browser
remembers your choice. Net2's "Dual credential" type is not offered: the API
has no name for it. A real Net2 upload of either kind has not yet been tried,
so check the first few in Net2.

## How it works

Topcoat renders the complete HTML page at build time. The browser loads a small
Rust WebAssembly module for WebHID, token decoding, and updating the controls.
Leptos and Trunk are no longer used. Topcoat's server, router, and runtime features
are disabled: there are no server endpoints or server-backed interactions. The
Net2 tools call the Net2 Local API with `fetch`; Net2's nginx answers CORS for
any origin, so this works from GitHub Pages as well as locally.

Topcoat 0.8.1 does not yet provide a built-in static export command, so
`cargo run -- dist` renders its `view!` template to `dist/index.html`.
Re-run the three build commands above after changing the sources, then reload.
Serve or upload the resulting `dist` directory as-is. Relative asset URLs work
both at the domain root and under `/tusk/` on GitHub Pages; no base URL is needed.
The reader still needs HTTPS (or localhost) for WebHID and clipboard access.
This is a Topcoat static-template experiment, not a migration to its reactive runtime.

### Reader protocol

Messages are framed, checksummed, and XOR-obfuscated with a repeating key:

```
02 06 88 61 66 A8      set the LEDs       (RWD_LEDS 0x24, argument 0x0A)
02 05 88 92 DE         read a Mifare card (RWD_READ_MIFARE 0xD7)
02 25 88 55 …          reply with the token, obfuscated
```

The app runs a five-message handshake on connect, then polls four times a
second, alternating Mifare and Hitag2 reads. Without the handshake the reader
ignores reads, which is what makes a freshly plugged-in device look broken.

Full details — framing, checksum, the key, the address rules, the command map
— are in [PROTOCOL.md](PROTOCOL.md). The command names and token number
formulas come from Net2's own code.

## Testing and deployment

```sh
cargo test   # protocol and token decoding, on the host
cargo w      # the polling loop, in headless Chrome against a fake reader
```

`cargo w` is an alias for `cargo test --lib --target wasm32-unknown-unknown`; see
[CONTRIBUTING.md](CONTRIBUTING.md) for what it needs. The host is the default
target, so `cargo test` works on any machine without naming a triple.

The tests that matter are pinned to frames captured from the real device with
USBPcap. Input we invented ourselves would only prove that the encoder and
decoder share the same misunderstanding of the format — the exact failure mode
worth guarding against when the format was reverse engineered rather than
documented.

[CI](.github/workflows/ci.yml) runs on every push and pull request: `cargo fmt
--check`, clippy with warnings as errors on both the wasm target and the host,
and both sets of tests. The build renders the page with Topcoat, packages the
WebAssembly module with wasm-bindgen, and smoke-tests the exported site under a
repository subpath. Each push to `master` that passes is published to GitHub Pages.

## Project layout

| Path | What it is |
|---|---|
| `src/protocol.rs` | Building command frames and parsing replies |
| `src/token.rs` | Turning a read reply into the Net2 token number |
| `src/reader.rs` | WebHID connection and the polling loop |
| `src/app.rs` | Topcoat page template and icons |
| `src/main.rs` | Static HTML exporter |
| `src/client.rs` | Browser controls and DOM updates |
| `src/batch.rs` | CSV preservation, assignment queue, and duplicate checks |
| `src/batch_client.rs` | Batch upload, guided prompts, review, and download |
| `src/net2.rs` | Net2 API errors and records, and portrait file rules |
| `src/net2_client.rs` | Net2 sign-in, requests, and saving tapped cards |
| `src/portrait_client.rs` | Portrait matching, checking, and upload |
| `src/lib.rs` | Shared protocol library and WebAssembly entry point |
| `PROTOCOL.md` | The reverse-engineered protocol |
| `src/style.css` | Page styling |
| `tests/static_site.py` | Exported-site browser smoke test |
| `.cargo/config.toml` | Unstable WebHID bindings, browser-test alias |

## Licence

[MIT](LICENSE).

Not affiliated with or endorsed by Paxton Access Ltd. The protocol here was
worked out by observing a device I own; it is not official documentation and
may be wrong or incomplete.
