# Tusk

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.98%2B-orange.svg)](https://www.rust-lang.org)
[![Leptos](https://img.shields.io/badge/leptos-0.8-ef3939.svg)](https://leptos.dev)
[![Browser](https://img.shields.io/badge/browser-Chrome%20%7C%20Edge-4285f4.svg)](https://developer.mozilla.org/en-US/docs/Web/API/WebHID_API)
[![Tests](https://img.shields.io/badge/tests-10%20passing-4ade80.svg)](#testing)

Read tokens from a Paxton Net2 USB desktop reader in the browser, over WebHID.
No drivers, no Net2 software, no server — put a card on the reader and the
token number appears.

```
┌──────────────────────────────────────┐
│  Tusk                                │
│  [ Connect reader ]                  │
│  Ready - present a token             │
│ ┌──────────────────────────────────┐ │
│ │           5B7D4039               │ │
│ └──────────────────────────────────┘ │
└──────────────────────────────────────┘
```

The reader speaks an undocumented, lightly obfuscated HID protocol. Working it
out was most of this project; [PROTOCOL.md](PROTOCOL.md) is the write-up.

## Requirements

- **Chrome or Edge.** WebHID is not available in Firefox or Safari, and the
  page says so rather than failing silently.
- **A Paxton Net2 USB desktop reader** (`USB\VID_1071&PID_0001`).
- **Rust** with the `wasm32-unknown-unknown` target, and
  [Trunk](https://trunkrs.dev).

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
```

## Running

```sh
trunk serve --port 8080
```

Open <http://127.0.0.1:8080>, click **Connect reader**, and pick the device.
The picker shows it under an unreadable name — that is the reader returning
uninitialised memory as its USB product string, and it differs every time it
enumerates. Filtering on Paxton's vendor ID at least leaves it as the only
entry in the list.

After the first connection the browser remembers permission for that device,
so reloading reconnects without a prompt. Unplugging and replugging is handled
automatically.

## How it works

Messages are framed, checksummed, and XOR-obfuscated with a repeating key:

```
02 06 88 61 66 A8      prime the reader   (opcode 0x24, argument 0x0A)
02 05 88 92 DE         read the token     (opcode 0xD7)
02 25 88 55 …          reply with the token, obfuscated
```

The app runs a five-message handshake on connect, then polls four times a
second. Without the handshake the reader ignores reads, which is what makes a
freshly plugged-in device look broken.

Full details — framing, checksum, the key, the address rules, the command map
— are in [PROTOCOL.md](PROTOCOL.md).

## Testing

```sh
cargo t
```

`cargo t` is an alias for `cargo test --target x86_64-pc-windows-msvc`. The
plain command would build for wasm32, which has no test runner. See
[.cargo/config.toml](.cargo/config.toml).

The tests that matter are pinned to frames captured from the real device with
USBPcap. Input we invented ourselves would only prove that the encoder and
decoder share the same misunderstanding of the format — the exact failure mode
worth guarding against when the format was reverse engineered rather than
documented.

## Project layout

| Path | What it is |
|---|---|
| `src/main.rs` | The whole app: protocol, polling, UI |
| `PROTOCOL.md` | The reverse-engineered protocol |
| `index.html` | Page shell and styling |
| `.cargo/config.toml` | wasm target, unstable WebHID bindings, test alias |

## Licence

[MIT](LICENSE).

Not affiliated with or endorsed by Paxton Access Ltd. The protocol here was
worked out by observing a device I own; it is not official documentation and
may be wrong or incomplete.
