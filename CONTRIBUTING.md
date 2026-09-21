# Contributing

Thanks for taking a look. This is a small project with an unusual constraint:
most of it cannot be tested without the hardware in front of you.

## Getting set up

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked
cargo run --locked -- dist
cargo build --locked --release --lib --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir dist target/wasm32-unknown-unknown/release/tusk.wasm
python -m http.server 8080 --directory dist --bind 127.0.0.1
```

Then open <http://127.0.0.1:8080> in Chrome or Edge. Firefox and Safari have
no WebHID, so the page will tell you it cannot work there.

On Linux, install the udev rule in the [README](README.md#requirements) before
believing the reader is broken. Without it `/dev/hidrawN` is root-only and
Chrome cannot open the device, which looks exactly like a reader that will not
answer. Windows has no equivalent gate, so a build that works there can still
fail here for reasons that have nothing to do with the code.

Topcoat generates the page on the host; only the reader and DOM controls compile
to WebAssembly. Re-run the three build commands after editing. `dist` is generated
and ignored by Git. There is no application server to deploy.

## Running the tests

```sh
cargo test   # protocol and token decoding, on the host
cargo w      # the polling loop, in headless Chrome
python tests/static_site.py  # after building dist; needs Chrome and chromedriver
```

The host is the default target, so `cargo test` needs no triple and works
wherever you are. `cargo w` is an alias for
`cargo test --lib --target wasm32-unknown-unknown`, which runs the tests in
`src/reader.rs` with `wasm-bindgen-test-runner`. It needs Chrome, a matching
`chromedriver`, and `wasm-bindgen-cli` at exactly the `wasm-bindgen` version
in `Cargo.lock`:

```sh
cargo install wasm-bindgen-cli --version <the version in Cargo.lock>
```

Get `chromedriver` from [Chrome for
Testing](https://googlechromelabs.github.io/chrome-for-testing/) at your
Chrome's version. If `geckodriver` is also on your `PATH` the runner may pick
it, and Firefox has no WebHID, so name the one you want:

```sh
CHROMEDRIVER=$(command -v chromedriver) cargo w
```

Those tests drive the real polling loop against a fake reader — a plain JS
object standing in for the `HIDDevice`. The fake answers from our model of the
reader, so it cannot tell us that model is wrong; that is what the captured
frames in the host tests are for. What it does prove is the loop's own logic:
when the card is shown and cleared, when the reader counts as gone, and that
two connections never fight over it.

## Testing against the hardware

Two traps cost real time during development. Both are easy to fall into again.

**Always test from a cold start.** Unplug the reader and plug it back in
before believing a change works. The reader keeps state: once it has been
primed, it will answer reads that a freshly enumerated device refuses. A fix
that looks correct on a warm reader can be completely wrong.

**Watch the reader, not just the screen.** It flashes green when it accepts a
command and red when it resets. A red flash and a Windows disconnect sound
mean the device re-enumerated and any handle the page holds is dead.

## Working on the protocol

Read [PROTOCOL.md](PROTOCOL.md) first. It is not a summary written after the
fact — it is the working reference, and the tests assert against the byte
sequences it documents.

If you change how frames are built or parsed, keep the tests anchored to real
captured frames rather than to values you computed from your own
understanding. A decoder tested only against its own encoder will happily
agree with itself while both are wrong. That is not hypothetical here: during
development a deliberate one-byte shift in the obfuscation phase left the
round-trip property test passing, and only the captured frames caught it.

To capture more traffic, install [USBPcap](https://desowin.org/usbpcap/) and
Wireshark, then run as administrator:

```sh
USBPcapCMD.exe -d \\.\USBPcapN -A --inject-descriptors -o capture.pcap
```

Find the right interface number by checking which one lists the reader.

## Style

- Explain *why* in comments, not *what*. The what is in the code.
- Prefer deleting to adding. The reverse-engineering scaffolding that found
  the protocol was removed once it had done its job; the findings live in
  `PROTOCOL.md` and git history instead.
- Tests are named after the behaviour they protect
  (`rejects_a_frame_whose_checksum_does_not_match`, not `test_parse_2`).
- Before committing a test, check it fails when the code is wrong. Break the
  behaviour deliberately, watch it go red, put it back. A test that cannot
  fail costs maintenance and returns nothing.

## Commits

Describe what changed and why it mattered. If a change came from a wrong
assumption, say what the assumption was — that is the part worth reading in
six months.

Record user-visible changes in [CHANGELOG.md](CHANGELOG.md) under
`Unreleased`.
