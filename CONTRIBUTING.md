# Contributing

Thanks for taking a look. This is a small project with an unusual constraint:
most of it cannot be tested without the hardware in front of you.

## Getting set up

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
trunk serve --port 8080
```

Then open <http://127.0.0.1:8080> in Chrome or Edge. Firefox and Safari have
no WebHID, so the page will tell you it cannot work there.

## Running the tests

```sh
cargo t
```

That is an alias for `cargo test --target x86_64-pc-windows-msvc`, because
`.cargo/config.toml` sets the default target to wasm32, which has no test
runner. On a non-Windows host, change the triple in the alias.

The protocol code is pure and fully testable without hardware. The WebHID
plumbing is not tested, by choice — mocking it would mostly assert that the
mock behaves like the mock.

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
