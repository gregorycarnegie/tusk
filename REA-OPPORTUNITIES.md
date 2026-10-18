# Useful additions to Tusk

Investigation: 16 October 2026. Read-only review of Tusk source and static REA
inspection of the installed Net2 desktop reader service. No reader commands,
Net2 API requests, or application changes were made.

## Recommended order

| Priority | Addition | Why it helps | Smallest useful version |
| --- | --- | --- | --- |
| 1 | Protect undownloaded people-import results | New User IDs and reconciliation notes can be lost on reload or when replacing the spreadsheet. | Track undownloaded results; extend the existing leave-page warning and confirm before replacing the people sheet. |
| 2 | Download an import outcome report | A person can be created while department, access levels, or portrait updates fail. The current download omits those failure details. | A separate CSV with sheet row, person, User ID, and the existing result message, including uncertain outcomes. |
| 3 | Reader feedback after a successful assignment | During a queue of people, a sound or LED change would let the operator know when to move on. | Start with the documented token-found LED state; investigate the buzzer arguments before using them. Signal success after Net2 confirms saving, or after a local CSV assignment succeeds. |
| 4 | Show access levels and dates before import | The preview shows name, department, and photo, but the operation also changes access and validity. | Add resolved access levels and activation/expiry dates to the existing preview. |
| Conditional | Show a Wiegand number | Net2 contains a separate conversion path that might help sites comparing card numbers across systems. | Only pursue if a real deployment needs it; display separately from the existing Net2 number after validating the conversion. |

My recommendation is to do 1 and 2 together first. Reader feedback is the
smallest promising hardware addition. Wiegand is a research lead rather than
a feature to add speculatively.

## Source evidence and implementation scope

### Protect and export import results

- `src/people_client.rs:406` stores a newly created User ID in memory.
- `src/people_client.rs:242` replaces the loaded people sheet without an
  undownloaded-results check.
- `src/batch_client.rs:289` implements a leave-page warning, but checks only
  `state.batch.dirty`.
- `src/people_client.rs:429`, `:440`, and `:454` append department, access-level,
  and portrait failures to the visible result message after user creation.
- `src/people_client.rs:482` exports the original sheet with User IDs, omitting
  those result messages. At `:155`, download is disabled when there is no
  returned ID, which also excludes an uncertain creation outcome with no ID.

Reuse the current warning and CSV writer. Preserve the existing reimport
download. Do not automatically retry an uncertain user creation: Net2 may
already have created that person. A results report is the first step toward
manual reconciliation; automatic repair is a separate, larger feature.

Likely scope: `src/people_client.rs`, the existing unload handler in
`src/batch_client.rs`, a report control in `src/app.rs`, and scenarios in
`tests/static_site.py`. Verify closing/replacing a sheet after an import,
clearing the warning after download, a failed follow-up operation, and an
uncertain creation without an ID. No credentials or portrait data should be
included in the report.

### Reader feedback

`src/reader.rs:173` always sends the idle LED argument when polling.
`PROTOCOL.md` documents `RWD_LEDS` argument `0x06` as token found and lists
`RWD_BEEP` (`0x1E`), but does not specify the beep payload.

REA observed a static call from Net2's `DoPoll` to
`DesktopReaderSettings.get_ShouldSoundBuzzer`. This supports investigating
reader feedback, but does not establish a safe beep payload or the exact
hardware behavior. Preserve polling and reply classification when adding
commands. For direct assignment, a successful read alone must not signal a
successful Net2 save.

Likely scope: `src/protocol.rs`, `src/reader.rs`, the assignment completion
paths in `src/batch.rs` / `src/net2_client.rs`, fake-reader expectations, and
`PROTOCOL.md`. Validate on the real reader as well as against the fake.

### Access preview and conditional Wiegand display

`src/people_client.rs:114` previews name, department, and photo; access levels
are resolved at `:328`. `src/people.rs:126` reads validity dates. Preview these
before confirmation, without exposing PINs or sensitive custom fields.
Likely scope: the people preview/render code, its table in `src/app.rs`, and
the browser smoke test.

REA observed `ExtractTokenData` calling both `DeriveMifareTokenNo` and
`DeriveMifareWiegandNo`, plus `DeriveEM4100WiegandNo`. Tusk currently derives
only its Net2 decimal number and raw hex in `src/token.rs:15`. The additional
conversion is an observed code path; its exact formula and usefulness at this
site remain unresolved. Do not change the existing number or assignment
format based on the method name alone. Some inspected arithmetic uses an
obfuscated constant lookup helper, so its constants require further recovery.

## REA evidence ledger

Target:
`C:\Program Files (x86)\Paxton Access\Access Control\Paxton.Net2.DesktopReaderSrv.dll`

- SHA-256: `53bd696d306a2646d5b29a78cb2f246bc413acdc28eb623b4c2c088f4980f315`
- Assembly version: `7.0.0.0`; target framework: `.NETFramework,Version=v4.8`.
- Module MVID: `d8050711-93e4-b007-1bf4-923aa49ad7f4`.
- Artifact identity/metadata parent Evidence:
  `ev_b048e650f56e87e0ce6d8f6decb2a5451e649e4475872fbbaf931e14fea0f02a`.
- Members parent Evidence:
  `ev_6996fa1a320e94a8621d306f7a7be5df65ba65dcd8fc0c1cc3f77731e6dbd213`.
  REA reports complete metadata coverage for this DLL: 32 types, 172 methods.
- Methods view Evidence:
  `ev_28f2380ce3b6b843f24998e6cab5f9fea658a70b744ca4386fe01779a8a49855`.
  Relevant build-local method tokens: `0x06000018` Mifare token conversion,
  `0x06000019` Mifare Wiegand conversion, `0x06000022` token extraction,
  `0x06000023` polling, `0x0600002a` buzzer setting getter.
- Calls view Evidence:
  `ev_232308bc57c5f1ee9287b60bbe2e89a19561483d9c3e682f283c2d5a6dee43e4`.
  Observed edges: `0x06000022 -> 0x06000018`,
  `0x06000022 -> 0x06000019`, `0x06000023 -> 0x0600002a`.

These are static observations, not execution or recovered original C#.
Recommendations are analyst judgments combining those observations with the
repository behavior. Coverage is limited to this installed DLL and the source
paths above; it does not establish reader hardware capabilities.

## Unresolved and deliberately skipped

- Saved USB capture: REA `inspect_pcap_capture` failed with
  `capability_unavailable`. It could not resolve an absolute TShark executable
  in its Windows process runner. Wireshark's `tshark.exe` and `capinfos.exe`
  exist under `C:\Program Files\Wireshark`, but REA needs absolute
  `REA_TSHARK_COMMAND` / `REA_CAPINFOS_COMMAND` settings. No MCP configuration
  was changed, and no new packet conclusions were drawn.
- No card/fob capability was verified on hardware. Hitag2 was deliberately
  removed in the current changelog after unsuccessful reads; seeing decoder
  methods in Net2 is insufficient reason to restore it.
- No opcode sweeps, reset, token-write, firmware, or antenna-calibration
  changes. No live Net2 writes or tests were run for this investigation.
- Existing local modifications to `Cargo.toml` and `Cargo.lock` were preserved.

## Implementation follow-up — 16 October 2026

The first four recommendations are implemented. Validation passed: 142 host
tests, 12 browser tests, 11 static-site smoke scenarios, formatting and clippy.

A subsequent test used the real connected Paxton USB reader and the built
site, with no fake HID device or Net2 connection. It read a Mifare card,
assigned it to a fictional local CSV row, continued reading it while held
with LED argument `0x06`, cleared the display when lifted, restored LED
argument `0x0A`, and exported the expected CSV. The operator confirmed that
the physical LED changed on assignment and returned to idle after lifting.
Local evidence: `target/real-reader-status.json`. Net2 save confirmation and
failure cases remain covered by stand-in API tests rather than this hardware
check. No commits were made; beep support and Wiegand remain deferred.
