# Formal verification

This crate contains the executable HID byte codec and token decoding used by
Tusk through `src/protocol.rs` and `src/token.rs`. These are the actual
implementations linked into the application. The specifications and proofs
are erased by ordinary Cargo builds.

## Run it

Use **Verus 0.2026.11.02.8bf68ee**, **Rust 1.99.1**, and the exact
`vstd = "=0.0.0-2026-11-01-0251"` dependency in this crate's manifest.
Download the archive for your platform from the
[official pinned release](https://github.com/verus-lang/verus/releases/tag/release/0.2026.11.02.8bf68ee),
unzip it, and put its directory on `PATH`. Install the matching Rust toolchain:

```sh
rustup toolchain install 1.99.1
cargo verus verify -p tusk-verified --locked --fwd-verus-args-to roots -- --no-cheating
```

Run this from the repository root. `--no-cheating` rejects `assume`, `admit`,
`external_body` and `assume_specification` in this crate. Arguments are forwarded
to the root crate because `vstd` contains trusted standard-library specifications.
The command also checks `vstd`'s proofs. No application crate is opted into
verification merely to skip its function bodies.

For a fresh check of this crate using the release's bundled, preverified `vstd`
(without Cargo's incremental verification cache):

```sh
verus --crate-type lib --edition 2024 verified/src/lib.rs --no-cheating
```

On Windows, the locally downloaded tool is at
`target/verus-tools/verus-x86-win/verus.exe`. In PowerShell:

```powershell
& ./target/verus-tools/verus-x86-win/verus.exe --crate-type lib --edition 2024 verified/src/lib.rs --no-cheating
```

The CI `verify` job downloads the pinned Linux release, checks its SHA-256,
and runs Cargo verification before the site can deploy. Normal development
still uses `cargo test --locked` and the existing browser tests.

## Proved properties

All executable functions inside `verus!` are checked for their contracts,
index bounds, arithmetic overflow and loop termination:

| Implementation or theorem | Guarantee |
| --- | --- |
| `mask_byte` | Exact `Elephant` XOR phase for every address and every `usize` index; plaintext addresses are unchanged. |
| `masking_is_its_own_inverse` | Applying the same mask twice restores every possible byte. |
| `checksum` | Bitwise complement of the sum modulo 256, for any byte slice. |
| `frame` | For payloads of 0–35 bytes, exactly the specified 41-byte frame: header, masked body, checksum, zero padding and EOT. |
| `parse` | For any byte slice, returns a reply exactly when STX, declared length and checksum are valid; returns the exact decoded opcode and payload, respecting plaintext NAKs. |
| `frame_roundtrip` | Every allowed payload and address decode back to the input when the wire opcode avoids the NAK ambiguity. |
| `used_opcodes_are_unambiguous` | Every currently documented Tusk command/reply opcode meets the round-trip restriction for all 256 addresses. |
| `is_read_reply` | Exactly ACK with at least 16 payload bytes. |
| `token_data` | Returns a token exactly for a read reply with a nonzero payload byte. Keeps UID endings padded to 4, 7 or 10 bytes (longer nonzero data is preserved), and returns the first four bytes big-endian modulo 100,000,000. Every returned number is below 100,000,000. |

The parser intentionally retains its existing acceptance rules: it does not
require a 41-byte slice or validate EOT/padding on replies. Captured short
replies are supported. The frame wrapper in `src/protocol.rs` checks the
35-byte precondition at runtime and retains its oversized-payload panic.

## Coverage and trust boundary

This is partial application verification. The rest of the repository has the
following status:

| Code | Status |
| --- | --- |
| `src/protocol.rs`, `src/token.rs` | Call the proved implementations; wrappers, opcode constants and hex formatting are tested Rust outside the proof boundary. |
| `src/batch.rs` | Queue assignment, duplicate refusal, undo and write-confirmation behavior have tests, not formal proofs. |
| `src/people.rs`, `src/manage.rs`, `src/net2.rs` | Date rules, promotion planning, record updates, API parsing and image rules have tests, not formal proofs. |
| `src/sheet.rs`, `src/tsql.rs` | Spreadsheet parsing/export and SQL generation, including their dependencies, are outside the proofs. |
| `src/reader.rs`, `src/*_client.rs`, `src/client.rs` | Browser I/O, polling, async state, authentication and Net2 writes are outside the proofs. |
| `src/app.rs`, `src/main.rs`, CSS | Page generation and presentation are outside the proofs. |

The mathematical protocol model comes from [PROTOCOL.md](../PROTOCOL.md).
Proofs establish agreement with that model, not the correctness of the model
against hardware. Existing captured-frame tests remain essential. Rust,
Verus, Z3, `vstd`'s standard-library specifications and the compilation/ghost
erasure process are trusted. Allocation failure and actual device/server
behavior are not proved.

## Validation on 22 November 2026

- Verus: **20 verified, 0 errors** for this crate, including `--no-cheating`.
- Cargo verification also checked `vstd`: **2159 verified, 0 errors**.
- Existing host tests: **159 library tests and 1 site test passed**.
- The WebAssembly library compiled with `cargo check --locked --lib --target wasm32-unknown-unknown`.
- A temporary mutant changing token conversion to modulo 100,000,001 failed
  its postcondition, demonstrating that the number contract detects the error.

The browser suite and physical hardware were not exercised during this change.
