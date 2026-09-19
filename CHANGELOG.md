# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/gregorycarnegie/tusk/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/gregorycarnegie/tusk/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/gregorycarnegie/tusk/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/gregorycarnegie/tusk/releases/tag/v0.1.0
