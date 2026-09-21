# Tusk

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.98%2B-orange.svg)](https://www.rust-lang.org)
[![Topcoat](https://img.shields.io/badge/topcoat-0.8.1-56AA1C.svg)](https://github.com/tokio-rs/topcoat)
[![Browser](https://img.shields.io/badge/browser-Chrome%20%7C%20Edge-4285f4.svg)](https://developer.mozilla.org/en-US/docs/Web/API/WebHID_API)
[![CI](https://github.com/gregorycarnegie/tusk/actions/workflows/ci.yml/badge.svg)](https://github.com/gregorycarnegie/tusk/actions/workflows/ci.yml)

Read tokens from a Paxton Net2 USB desktop reader in the browser, over WebHID.
No drivers, no Net2 software, no server — put a card on the reader and the
token number appears, as Net2 would show it.

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
python -m http.server 8080 --directory dist --bind 127.0.0.1
```

Open <http://127.0.0.1:8080>, click **Connect reader**, and pick the device.
The picker shows it under an unreadable name — that is the reader returning
uninitialised memory as its USB product string, and it differs every time it
enumerates. Filtering on Paxton's vendor ID at least leaves it as the only
entry in the list.

After the first connection the browser remembers permission for that device,
so reloading reconnects without a prompt. After unplugging it, click **Connect
reader** again: the reader has no USB serial number, so Chrome forgets the
permission when it is unplugged and the page cannot reconnect by itself.

## How it works

Topcoat renders the complete HTML page at build time. The browser loads a small
Rust WebAssembly module for WebHID, token decoding, and updating the controls.
Leptos and Trunk are no longer used. Topcoat's server, router, and runtime features
are disabled: there are no server endpoints or server-backed interactions.

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
