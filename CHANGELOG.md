# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- A wrong operator name or password at sign-in said "Not signed in to Net2.
  Connect again.", which reads like an expired session. It now says "Net2
  didn't accept the operator name or password."
- PNG portraits uploaded without an error but showed no picture in Net2. Net2
  stores a PNG and displays only JPGs, so every PNG is now converted to a JPG
  in the browser first. JPGs within Net2's limits still upload unchanged.

### Documentation

- The README explains how to trust the Net2 Local API certificate on each
  computer, and what `invalid_client` and a refused operator mean at
  sign-in: restart the Local API after installing a licence, and enter the
  operator exactly as Net2 names it.

## [0.4.1] - 2026-09-23

### Fixed

- A card lifted off the reader stayed on screen, and in batch mode a card on
  the reader at Start was never released. The reader answers an empty Mifare
  read late, after the next read has gone out, and Tusk credited each reply to
  the latest read. Replies are now matched to reads in order. Found with a live
  log of the reader: its "no card" reply is `0x12 01`, not the all-zero ack
  PROTOCOL.md described.

### Changed

- Portraits in any format the browser can open are accepted. A JPG or PNG
  within Net2's limits still uploads unchanged; anything else, or anything too
  big, is converted in the browser to a JPG of at most 1200 pixels on its
  longest side, upright and with transparency on white.

### Verification

- Tested against a real Net2 with a real reader: 20 Mifare cards saved straight
  to Net2 users with the chosen card type, and 20 portraits uploaded, including
  a WebP that was converted. This settles 0.4.0's known issue about untested
  saves and uploads. MFA sign-in and Hitag2 fobs remain untested.

## [0.4.0] - 2026-09-23

### Added

- **Upload portraits**: match JPG and PNG files named by Net2 user ID, check
  every match, then upload them to Net2. Existing portraits are kept unless
  replacement is ticked, and each upload re-reads the person first. Ported from
  the net_to_rest prototype, without its local server.
- **Cards straight to Net2**: load people from Net2, all users or one
  department, and save each tapped card to its person as it is read. People
  who already have a card keep it unless new cards were requested. CSV import
  and download still work as before. Pick the Net2 card type to record; it
  defaults to Proximity ISO card no magstripe and is remembered.
- Net2 sign-in with an operator account and integration ClientID, from the page
  itself. Net2's nginx allows cross-origin requests, so no server is needed.
  An `invalid_client` error now says to use the licence's ClientID, not its Id.
  Sign-in is sent as a form so the browser skips its CORS preflight: Net2's
  nginx allows one sign-in request per 500 ms from any non-loopback address,
  and a preflight followed by the real request always got a 429.

### Changed

- Pin dependency requirements to the versions in use.
- The build instructions serve the site on port 8000. Net2's own web server
  uses 8080 for its setup page.

### Known issues

- A real portrait upload and a real card save have been tested only against a
  fake Net2. Signing in to a real Net2 has been checked as far as its
  `invalid_client` refusal. Check the first few in Net2.
- Accounts that need MFA cannot sign in yet.

## [0.3.0] - 2026-09-21

### Added

- Guided CSV card assignment: prompt for each person's card, advance after a new
  tap, and download a Net2 import CSV. Decimal is the default; raw hex is optional.
- Preserve existing card numbers unless replacement is selected, refuse duplicate
  cards, and support pause, skip, undo, partial downloads, and an unsaved-work warning.

### Changed

- Replace the Leptos UI with a Topcoat page rendered at build time. Keep the
  Rust WebHID reader in WebAssembly and host the exported files on GitHub Pages.
- Replace Trunk with the Topcoat exporter and wasm-bindgen build commands;
  add a browser smoke test for the exported site under a repository subpath.

## [0.2.2] - 2026-09-19

### Added

- Tests for the polling loop, run in headless Chrome against a fake reader
  (`cargo w`), and mutation testing with cargo-mutants.
- Linux setup instructions. Chrome reaches the reader through `/dev/hidrawN`,
  which is root-only by default, so without a udev rule the device never
  opens and the reader looks broken. Windows needs no equivalent.

### Changed

- The host is now the default build target, and wasm32 is named where it is
  wanted. `cargo test` runs the host tests on any machine; `cargo w` runs the
  browser ones. The old `cargo t` alias hardcoded the Windows triple and
  could not build anywhere else.

### Fixed

- Error messages show the browser's own text once, instead of a debug print
  that repeated it. A reader that will not start now says to unplug it and
  plug it back in.

## [0.2.1] - 2026-09-19

### Changed

- Split `src/main.rs` into `protocol`, `token`, `reader` and `app` modules.
  No change in behaviour.

## [0.2.0] - 2026-09-19

Shows the token number Net2 uses, adds beta support for Paxton's Hitag2 fobs,
and is now hosted on GitHub Pages with CI.

### Added

- The Net2 token number, shown as Net2 shows it, with the raw hex beneath. For
  Mifare that is the first four UID bytes, big-endian, modulo 10^8. Checked
  against Net2 with a real card.
- **Beta:** Paxton Hitag2 fobs, read with `TOKEN_R_DATA` (`0x14`) in
  alternation with the Mifare read, and decoded following Net2's own decoder.
  Not yet tested on a real fob.
- Paxton's names for every opcode, from Net2's `BOARD_CMD` enum, in
  `PROTOCOL.md`.
- Hosted on GitHub Pages at <https://gregorycarnegie.github.io/tusk/>.
- GitHub Actions CI: formatting, clippy on the wasm target and the host with
  warnings as errors, and the tests, on every push and pull request. Pushes
  to `master` that pass are deployed to Pages.
- A redesign in Paxton-style colours (their green, charcoal and warm
  off-white), with light and dark themes that follow the system and a toggle
  that remembers the choice. The status line has a coloured dot for idle,
  connecting, ready and problem states, and the header links to the source.
- **Copy number** and **Copy hex** buttons, which say "Copied" once the
  clipboard has taken it.

### Changed

- What was called the prime (`0x24`) is `RWD_LEDS`, and the read (`0xD7`) is
  `RWD_READ_MIFARE`. Code and docs now use those names.

### Removed

- Automatic reconnection after a replug, which 0.1.0 listed as a feature but
  which never ran. The reader has no USB serial number, so Chrome drops the
  permission on unplug and never reports the reader coming back. The status
  now says to click **Connect reader** instead.

### Fixed

- A Mifare UID ending in `00` was treated as no card or shown short. UIDs are
  now cut at 4, 7 or 10 bytes, the sizes Mifare uses.
- "Ready" was shown once the handshake had been written, whether or not the
  reader answered. It now waits for the first reply to a read.
- A reader that stopped answering while still accepting writes kept showing
  the last token. The display clears after four polls without an answer.
- Startup and the connect button could both start a polling loop for
  the same reader, and an old loop ending could wipe a newer connection's
  state. The reader is now claimed before the first await, and only the newest
  connection resets shared state.
- The frame builder claimed to panic on an oversized payload but overwrote the
  checksum instead. It now panics.

## [0.1.0] - 2026-09-19

First working version: reads tokens from a Paxton Net2 USB desktop reader in
the browser, with no Net2 software running.

### Added

- WebHID connection to the reader, filtered to Paxton's vendor ID
  (`USB\VID_1071&PID_0001`), with a clear message on browsers that lack WebHID.
- The five-message startup handshake the reader needs before it will answer a
  read. Without it a freshly plugged-in device refuses every command.
- Token polling four times a second: prime with opcode `0x24`, collect with
  `0xD7`, decode, display.
- `PROTOCOL.md`, documenting the reverse-engineered protocol: framing,
  checksum, the `Elephant` XOR obfuscation and its phase rule, the plaintext
  and obfuscated dialects, the handshake, and the command map.
- Test suite pinned to frames captured from the real device, plus property
  tests covering the obfuscation across every address.
- `cargo t` alias, so tests run on the host rather than the wasm target.

### Fixed

Problems found and fixed while getting to the first working version, kept here
because each was a wrong assumption worth remembering:

- Reads were sent without the prime that must precede them. This appeared to
  work only because earlier probing had already primed the reader; a cold
  start failed.
- The prime's acknowledgement was treated as a token read, so it wiped the
  display immediately after each successful read.
- NAK replies are sent unobfuscated even when the address asks for
  obfuscation. Unmasking them turned `0x13` into a meaningless message type.
- A disconnected reader was left stored, so the connect button silently did
  nothing afterwards.
- Connecting while already connected did nothing at all, which was
  indistinguishable from a failure. It now says so.
- Nothing held a reference to the device after connecting, so Chrome could
  garbage-collect the handle and quietly stop delivering reports.
- A write to a reader that had re-enumerated never settled, wedging the poll
  loop. Writes now time out.

### Known issues

- The reader reports uninitialised memory as its USB product string, so it
  appears under a different unreadable name every time it enumerates. Nothing
  a web page can do; the device picker is browser UI showing what the device
  reports.
- Tokens are assumed to be 4 to 8 bytes. Longer ones would be ignored.

[Unreleased]: https://github.com/gregorycarnegie/tusk/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/gregorycarnegie/tusk/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/gregorycarnegie/tusk/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/gregorycarnegie/tusk/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/gregorycarnegie/tusk/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/gregorycarnegie/tusk/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/gregorycarnegie/tusk/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/gregorycarnegie/tusk/releases/tag/v0.1.0
