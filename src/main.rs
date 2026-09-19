use leptos::prelude::*;
use leptos::reactive::owner::StoredValue;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use web_sys::{HidDevice, HidDeviceFilter, HidDeviceRequestOptions, HidInputReportEvent};

/// Paxton Net2 desktop reader: USB\VID_1071&PID_0001, HID vendor-defined.
const PAXTON_VID: u32 = 0x1071;

/// The reader declares one 41-byte input report and one 41-byte output report.
const REPORT_BYTES: usize = 41;

/// Any address with the high bit set works; it only picks the key phase.
const ADDR: u8 = 0x88;

/// Net2 sets the LEDs before every read (RWD_LEDS), and the reader stays idle
/// without it. Opcode names are Paxton's, from the BOARD_CMD enum in Net2.
const OP_LEDS: u8 = 0x24;
const LEDS_ARG: u8 = 0x0A;

/// RWD_READ_MIFARE: answers with the card's UID, then zero padding.
const OP_READ_MIFARE: u8 = 0xD7;

/// TOKEN_R_DATA: answers with Hitag2 pages 2 to 7, four bytes each.
const OP_READ_HITAG2: u8 = 0x14;

/// Fast enough to feel instant, slow enough to leave the reader alone.
const POLL_MS: i32 = 250;

/// A reset leaves the write promise pending forever, so give up on it.
const SEND_TIMEOUT_MS: i32 = 500;

/// Polls in a row without an answer before the display stops being trusted.
/// Writes can keep succeeding while no replies come back.
const MAX_MISSES: u32 = 4;

const READY: &str = "Ready - present a token";

const REPO_URL: &str = "https://github.com/gregorycarnegie/tusk";

/// How the status line is coloured.
#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Idle,
    Busy,
    Ready,
    Problem,
}

type Status = RwSignal<(Tone, String)>;

fn say(status: Status, tone: Tone, message: impl Into<String>) {
    status.set((tone, message.into()));
}

/// With the high bit set in the address, everything from the type byte onwards
/// is XORed with this repeating key, starting at key index (address mod 8).
/// Zero padding XORs to the key itself, which is why "Elephant" shows up as
/// readable text in raw frames. Addresses without that bit are plaintext.
const OBFUSCATION_KEY: [u8; 8] = *b"Elephant";

/// Decoded reply type: the reader understood and answered.
const ACK: u8 = 0x10;

/// "I did not understand", echoing our own bytes back. Sent unobfuscated even
/// when the address asks for obfuscation, so it is read straight off the wire.
const NAK: u8 = 0x13;

/// What Net2 sends once before it starts polling. Without it the reader
/// refuses the read, which is what a replug leaves us with.
const INIT: [(u8, u8, &[u8]); 5] = [
    (0x08, 0x25, &[]),            // RWD_OPEN_LINK
    (0x88, OP_READ_HITAG2, &[]),  // TOKEN_R_DATA
    (0xCD, 0x28, &[]),            // RWD_SERIAL_NUMBER
    (ADDR, OP_LEDS, &[LEDS_ARG]), // RWD_LEDS
    (ADDR, 0x00, &[]),
];

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn checksum(covered: &[u8]) -> u8 {
    !covered.iter().fold(0u8, |sum, b| sum.wrapping_add(*b))
}

/// Scramble (or unscramble - it is the same XOR) the opcode and payload for
/// an address, which is the only thing that decides the key phase.
fn mask(addr: u8, body: &mut [u8]) {
    if addr & 0x80 == 0 {
        return;
    }
    for (i, b) in body.iter_mut().enumerate() {
        *b ^= OBFUSCATION_KEY[(usize::from(addr) + i) % 8];
    }
}

/// Longest payload that still leaves room for the checksum and the EOT byte.
/// Reads and primes are far below this; it is here to document the ceiling.
const MAX_PAYLOAD: usize = REPORT_BYTES - 6;

/// Build a frame: STX, length, address, opcode, payload, checksum, EOT. The
/// length counts STX through checksum, which is how the reader frames replies.
/// Panics above MAX_PAYLOAD rather than silently truncating a command.
fn frame(addr: u8, opcode: u8, data: &[u8]) -> [u8; REPORT_BYTES] {
    // one byte more and the EOT below would overwrite the checksum
    assert!(
        data.len() <= MAX_PAYLOAD,
        "payload does not fit in one report"
    );
    let len = 5 + data.len();
    let mut buf = [0u8; REPORT_BYTES];
    buf[0] = 0x02;
    buf[1] = len as u8;
    buf[2] = addr;
    buf[3] = opcode;
    buf[4..4 + data.len()].copy_from_slice(data);
    mask(addr, &mut buf[3..len - 1]);
    buf[len - 1] = checksum(&buf[..len - 1]);
    buf[40] = 0x04;
    buf
}

struct Reply {
    msg_type: u8,
    payload: Vec<u8>,
}

fn parse(bytes: &[u8]) -> Option<Reply> {
    let len = usize::from(*bytes.get(1)?);
    if bytes.first() != Some(&0x02) || len < 5 || len > bytes.len() {
        return None;
    }
    if bytes[len - 1] != checksum(&bytes[..len - 1]) {
        return None;
    }
    let mut clear = bytes[3..len - 1].to_vec();
    // a NAK is never masked, and no masked ack can look like one
    if clear.first() != Some(&NAK) {
        mask(bytes[2], &mut clear);
    }
    Some(Reply {
        msg_type: *clear.first()?,
        payload: clear[1..].to_vec(),
    })
}

/// A read answers with a long padded frame; the LED command answers with a
/// single zero byte. Telling them apart stops that ack wiping a fresh token.
fn is_read_reply(reply: &Reply) -> bool {
    reply.msg_type == ACK && reply.payload.len() >= 16
}

/// Which kind of token a read asks the reader for. Net2 cycles through five;
/// these are the two implemented here.
#[derive(Clone, Copy, PartialEq)]
enum Read {
    Mifare,
    /// Beta: decoded exactly as Net2 does, but never tested on a real fob.
    Hitag2,
}

impl Read {
    fn opcode(self) -> u8 {
        match self {
            Read::Mifare => OP_READ_MIFARE,
            Read::Hitag2 => OP_READ_HITAG2,
        }
    }
}

#[derive(Clone, PartialEq)]
struct Token {
    read: Read,
    hex: String,
    /// What Net2 shows as the token number.
    number: u32,
}

/// The token, if this reply is a read that found a card. An empty payload
/// means the reader answered but there was nothing on it.
fn token(read: Read, reply: &Reply) -> Option<Token> {
    if !is_read_reply(reply) {
        return None;
    }
    let p = &reply.payload;
    match read {
        Read::Mifare => {
            let end = p.iter().rposition(|b| *b != 0)? + 1;
            // Mifare UIDs come in 4, 7 or 10 bytes, so one ending in 00 keeps
            // its last byte rather than being cut at the zero padding
            let len = [4, 7, 10].into_iter().find(|n| *n >= end).unwrap_or(end);
            let uid = &p[..len];
            // Net2 reads the first four bytes big-endian and keeps 8 digits
            let number = u32::from_be_bytes(uid[..4].try_into().ok()?) % 100_000_000;
            Some(Token {
                read,
                hex: hex(uid),
                number,
            })
        }
        Read::Hitag2 => Some(Token {
            read,
            hex: hex(p.get(8..16)?),
            number: hitag2_number(p)?,
        }),
    }
}

/// Net2's 5-bit digit codes, indexed by value. Mostly an odd-parity bit and
/// then the value MSB first, but 14 breaks that rule, so this is Net2's table
/// verbatim rather than a parity check. 13 and 15 end a number.
const DIGIT_CODES: [u8; 16] = [
    0b10000, 0b00001, 0b00010, 0b10011, 0b00100, 0b10101, 0b10110, 0b00111, 0b01000, 0b11001,
    0b11010, 0b01011, 0b11100, 0b01101, 0b11110, 0b11111,
];

fn digit(code: u8) -> Option<u8> {
    DIGIT_CODES.iter().position(|c| *c == code).map(|d| d as u8)
}

/// (page, bit offset from the MSB) of each 5-bit digit in the newer layout.
const CARD_TYPE_DIGITS: [(usize, usize); 3] = [(6, 25), (7, 5), (7, 15)];
const USER_CARD_DIGITS: [(usize, usize); 8] = [
    (5, 10),
    (5, 20),
    (6, 0),
    (6, 10),
    (4, 5),
    (4, 15),
    (4, 25),
    (5, 5),
];

/// The Net2 token number from a TOKEN_R_DATA reply, following
/// DeriveHitagTokenNo in Net2's desktop reader service. Page 7 says which
/// layout the fob uses. Net2 also rewrites some old fobs at this point; that
/// is deliberately left out, since this app never writes to a token.
fn hitag2_number(payload: &[u8]) -> Option<u32> {
    let pages = payload.get(..24)?;
    let page = |n: usize| u32::from_be_bytes(pages[4 * (n - 2)..][..4].try_into().unwrap());
    let digits_at = |at: &[(usize, usize)]| -> Option<String> {
        at.iter()
            .map(|&(p, off)| digit((page(p) >> (27 - off)) as u8 & 0x1F).map(|d| d.to_string()))
            .collect()
    };
    let pages_4_and_5 = u64::from(page(4)) << 32 | u64::from(page(5));
    let number = match page(7) >> 2 & 0xF {
        0b0100 => net2_number(pages_4_and_5, 64)?,
        0b0001 if digits_at(&CARD_TYPE_DIGITS)?.parse() == Ok(1) => {
            digits_at(&USER_CARD_DIGITS)?.parse().ok()?
        }
        0b0001 => net2_number(pages_4_and_5, 45)?,
        _ => return None,
    };
    Some(number).filter(|n| *n != 0)
}

/// Net2's classic layout: 5-bit digits from the top of pages 4 and 5, six in
/// page 4, then the rest from the start of page 5, stopping at 13 or 15.
/// Only the first `len` bits count. Follows DeriveNet2No, quirks included.
fn net2_number(bits: u64, len: usize) -> Option<u32> {
    let mut digits = String::new();
    let mut zeros = false;
    let mut i = 0;
    while i < len - 5 {
        let mut code = (bits >> (59 - i)) as u8 & 0x1F;
        // magstripe-style tokens pad leading digits with 00000, not the 0 code
        if code == 0 && (i == 0 || zeros) {
            code = DIGIT_CODES[0];
            zeros = true;
        } else {
            zeros = false;
        }
        let Some(d) = digit(code) else {
            // only zeros may follow the last digit
            if bits << i >> (64 - (len - i)) != 0 {
                return None;
            }
            break;
        };
        if d == 13 || d == 15 {
            break;
        }
        digits += &d.to_string();
        i = if digits.len() == 6 { 32 } else { i + 5 };
    }
    let padded = format!("{digits:0>8}");
    padded[padded.len() - 8..].parse().ok()
}

fn main() {
    leptos::mount::mount_to_body(App);
}

fn hid() -> web_sys::Hid {
    web_sys::window().unwrap().navigator().hid()
}

/// Reject after `ms`. A write to a reader that has been unplugged never
/// settles either way, which would wedge the polling loop.
fn expiry(ms: i32) -> js_sys::Promise {
    js_sys::Promise::new(&mut |_resolve, reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&reject, ms)
            .expect("setTimeout");
    })
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .expect("setTimeout");
    });
    let _ = promise.await;
}

async fn send(dev: &HidDevice, mut buf: [u8; REPORT_BYTES]) -> Result<(), JsValue> {
    let pending = dev.send_report_with_u8_slice(0, &mut buf)?;
    let racers = js_sys::Array::of2(&pending.into(), &expiry(SEND_TIMEOUT_MS).into());
    js_sys::Promise::race(&racers).await.map(|_| ())
}

/// Ask the reader for one kind of token. The answer to the read arrives
/// separately, as an input report, so note which read it will be answering.
async fn poll_once(
    dev: &HidDevice,
    read: Read,
    reading: StoredValue<Read, LocalStorage>,
) -> Result<(), JsValue> {
    send(dev, frame(ADDR, OP_LEDS, &[LEDS_ARG])).await?;
    reading.set_value(read);
    send(dev, frame(ADDR, read.opcode(), &[])).await
}

/// Who has the reader. A connection is told apart by its session rather than
/// its device, since the same reader can come back as the same HIDDevice.
#[derive(Default)]
struct Link {
    dev: Option<HidDevice>,
    /// Bumped by every connection; only the newest may reset shared state.
    session: u32,
    /// Between claiming the reader and it being open, when `opened()` is
    /// still false but a second connection must not start.
    opening: bool,
}

/// Listen for replies, then keep asking until the reader goes away.
async fn run(
    dev: HidDevice,
    card: RwSignal<Option<Token>>,
    status: Status,
    link: StoredValue<Link, LocalStorage>,
) {
    // Claim the reader before the first await, so startup and the button
    // cannot each start a loop for it. An unplugged reader is closed, which is
    // what lets a new connection in while the old loop winds down.
    if link.with_value(|l| l.opening || l.dev.as_ref().is_some_and(|d| d.opened())) {
        return;
    }
    let session = link.with_value(|l| l.session) + 1;
    link.set_value(Link {
        dev: Some(dev.clone()),
        session,
        opening: true,
    });
    // not "Ready" until the reader has actually answered a read
    say(status, Tone::Busy, "Connecting to the reader");
    let owned = || link.with_value(|l| l.session == session);
    let release = || {
        link.update_value(|l| {
            *l = Link {
                session,
                ..Link::default()
            }
        })
    };

    // nothing else can claim the reader while `opening` is set, so this
    // connection still owns it if opening fails
    if !dev.opened()
        && let Err(e) = dev.open().await
    {
        release();
        say(
            status,
            Tone::Problem,
            format!("Could not open the reader: {e:?}"),
        );
        return;
    }
    link.update_value(|l| l.opening = false);

    // the handshake includes a Hitag2 read, so that is what replies answer first
    let reading = StoredValue::new_local(Read::Hitag2);
    let misses = StoredValue::new_local(0u32);
    let listener = dev.clone();
    let cb = Closure::<dyn FnMut(HidInputReportEvent)>::new(move |ev: HidInputReportEvent| {
        // keep `listener` alive: Chrome stops delivering reports once the
        // HIDDevice wrapper is garbage collected
        let _ = &listener;
        let data = ev.data();
        let bytes: Vec<u8> = (0..data.byte_length()).map(|i| data.get_uint8(i)).collect();
        let Some(reply) = parse(&bytes) else { return };
        // the LED command answers with a short ack; anything else answers the read
        if reply.msg_type == ACK && !is_read_reply(&reply) {
            return;
        }
        // an answer to a read is the only proof the reader is working
        misses.set_value(0);
        if status.with_untracked(|s| s.1 != READY) {
            say(status, Tone::Ready, READY);
        }
        let read = reading.get_value();
        match token(read, &reply) {
            Some(t) => card.set(Some(t)),
            // nothing of this kind on the reader, so only a token this kind
            // of read found can have been lifted off
            None if card.with_untracked(|c| c.as_ref().is_some_and(|t| t.read == read)) => {
                card.set(None)
            }
            None => {}
        }
    });
    dev.set_oninputreport(Some(cb.as_ref().unchecked_ref()));
    cb.forget();

    for (addr, opcode, args) in INIT {
        if let Err(e) = send(&dev, frame(addr, opcode, args)).await {
            if owned() {
                release();
                say(
                    status,
                    Tone::Problem,
                    format!("Reader would not start: {e:?}"),
                );
            }
            return;
        }
        sleep(40).await;
    }

    let mut reads = [Read::Mifare, Read::Hitag2].into_iter().cycle();
    while owned()
        && poll_once(&dev, reads.next().unwrap(), reading)
            .await
            .is_ok()
    {
        sleep(POLL_MS).await;
        // the reply lands during the sleep and resets this
        misses.update_value(|m| *m += 1);
        if misses.get_value() == MAX_MISSES {
            card.set(None);
            say(status, Tone::Problem, "The reader is not answering");
        }
    }
    if owned() {
        release();
        card.set(None);
        // The reader has no USB serial number, so Chrome forgets the
        // permission when it is unplugged and never tells the page it came
        // back. Only the picker can grant it again.
        say(
            status,
            Tone::Idle,
            "Reader unplugged - plug it back in, then click Connect reader",
        );
    }
}

#[component]
fn App() -> impl IntoView {
    let card = RwSignal::new(None::<Token>);
    let status = RwSignal::new((Tone::Idle, "Not connected".to_string()));
    let link = StoredValue::new_local(Link::default());

    let has_hid = js_sys::Reflect::has(&web_sys::window().unwrap().navigator(), &"hid".into())
        .unwrap_or(false);
    if !has_hid {
        say(
            status,
            Tone::Problem,
            "WebHID is not available - open this page in Chrome or Edge",
        );
    }

    // reuse a reader the browser already has permission for, so a reload
    // does not mean clicking through the picker again
    spawn_local(async move {
        if !has_hid {
            return;
        }
        if let Ok(devices) = hid().get_devices().await
            && let Some(dev) = devices
                .iter()
                .find(|d| u32::from(d.vendor_id()) == PAXTON_VID)
        {
            run(dev, card, status, link).await;
        }
    });

    let connect = move |_| {
        spawn_local(async move {
            if !has_hid {
                return;
            }
            // already polling: the picker is only for granting permission, so
            // say so rather than appearing to do nothing
            if link.with_value(|l| l.dev.as_ref().is_some_and(|d| d.opened())) {
                say(status, Tone::Ready, "Already connected - present a token");
                return;
            }
            let filter = HidDeviceFilter::new();
            filter.set_vendor_id(PAXTON_VID);
            let opts = HidDeviceRequestOptions::new(&[filter]);
            match hid().request_device(&opts).await {
                Ok(devices) => match devices.get_checked(0) {
                    Some(dev) => run(dev, card, status, link).await,
                    None => say(status, Tone::Idle, "No reader selected"),
                },
                Err(e) => say(
                    status,
                    Tone::Problem,
                    format!("Could not reach the reader: {e:?}"),
                ),
            }
        })
    };

    // what was last put on the clipboard, so the button that did it can say so
    // until a different token turns up
    let copied = RwSignal::new(String::new());
    let copy_button = move |label: &'static str, pick: fn(&Token) -> String| {
        let copy = move |_| {
            let Some(text) = card.with_untracked(|c| c.as_ref().map(pick)) else {
                return;
            };
            spawn_local(async move {
                let clipboard = web_sys::window().unwrap().navigator().clipboard();
                if clipboard.write_text(&text).await.is_ok() {
                    copied.set(text);
                }
            });
        };
        let done = move || card.with(|c| c.as_ref().map(pick)) == Some(copied.get());
        view! {
            <button class="secondary" disabled=move || card.with(|c| c.is_none()) on:click=copy>
                {move || if done() { check_icon().into_any() } else { copy_icon().into_any() }}
                {move || if done() { "Copied" } else { label }}
            </button>
        }
    };

    view! {
        <header class="top">
            <span class="brand">{tusk_icon()} "Tusk"</span>
            <nav>
                <a class="icon-button" href=REPO_URL aria-label="Source code on GitHub" title="Source code on GitHub">
                    {github_icon()}
                </a>
                <button class="icon-button" on:click=|_| toggle_theme() aria-label="Switch between light and dark" title="Switch between light and dark">
                    {theme_icon()}
                </button>
            </nav>
        </header>
        <main>
            <section class="intro">
                <h1>"Read Net2 tokens in your browser"</h1>
                <p>"Plug in a Paxton Net2 USB desktop reader, connect it, and present a card. No drivers or Net2 software needed."</p>
            </section>
            <section class="panel">
                <div class="panel-head">
                    <p class="status" data-tone=move || match status.with(|s| s.0) {
                        Tone::Idle => "idle",
                        Tone::Busy => "busy",
                        Tone::Ready => "ready",
                        Tone::Problem => "problem",
                    }>
                        <span class="dot"></span>
                        {move || status.with(|s| s.1.clone())}
                    </p>
                    <button class="primary" on:click=connect>"Connect reader"</button>
                </div>
                <div class="reading" class:empty=move || card.with(|c| c.is_none())>
                    <p class="label">"Net2 token number"</p>
                    <p class="number">
                        {move || card.with(|c| c.as_ref().map_or("--------".to_string(), |t| t.number.to_string()))}
                    </p>
                    <p class="meta">
                        {move || match card.get() {
                            Some(t) => view! {
                                <span class="chip" class:beta=t.read == Read::Hitag2>
                                    {if t.read == Read::Hitag2 { "Hitag2 · beta" } else { "Mifare" }}
                                </span>
                                <code>{t.hex}</code>
                            }.into_any(),
                            None => view! { <span>"Present a card to the reader"</span> }.into_any(),
                        }}
                    </p>
                    <p class="beta-note" class:shown=move || card.with(|c| c.as_ref().is_some_and(|t| t.read == Read::Hitag2))>
                        "Hitag2 decoding is untested on real fobs. Check this number against Net2 before relying on it."
                    </p>
                    <div class="actions">
                        {copy_button("Copy number", |t| t.number.to_string())}
                        {copy_button("Copy hex", |t| t.hex.clone())}
                    </div>
                </div>
            </section>
        </main>
        <footer>
            <a href=REPO_URL>"Source on GitHub"</a>
            " · MIT licence · Not affiliated with or endorsed by Paxton Access Ltd."
        </footer>
    }
}

/// Flip between light and dark, starting from whatever is showing now, and
/// remember the choice. index.html reapplies it before the first paint.
fn toggle_theme() {
    let window = web_sys::window().unwrap();
    let Some(root) = window.document().and_then(|d| d.document_element()) else {
        return;
    };
    let dark = match root.get_attribute("data-theme") {
        Some(theme) => theme == "dark",
        None => window
            .match_media("(prefers-color-scheme: dark)")
            .ok()
            .flatten()
            .is_some_and(|m| m.matches()),
    };
    let next = if dark { "light" } else { "dark" };
    let _ = root.set_attribute("data-theme", next);
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item("theme", next);
    }
}

fn tusk_icon() -> impl IntoView {
    view! {
        <svg class="logo" viewBox="0 0 32 32" aria-hidden="true">
            <rect width="32" height="32" rx="9" fill="var(--brand)" />
            <path d="M9 6c-1 12 5 19 17 20-8-3-12-10-12-20a2.5 2.5 0 0 0-5 0Z" fill="#fff" />
        </svg>
    }
}

fn github_icon() -> impl IntoView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true">
            <path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
        </svg>
    }
}

/// Both glyphs are drawn; the stylesheet shows the moon in light mode and the
/// sun in dark mode, so the button needs no state of its own.
fn theme_icon() -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path class="moon" d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8Z" />
            <g class="sun">
                <circle cx="12" cy="12" r="4" />
                <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
            </g>
        </svg>
    }
}

fn copy_icon() -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="9" width="12" height="12" rx="2" />
            <path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1" />
        </svg>
    }
}

fn check_icon() -> impl IntoView {
    view! {
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <path d="M20 6 9 17l-5-5" />
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Frames captured from the real device with USBPcap while the Net2
    // software drove it. Synthesised input would only prove our encoder and
    // decoder share the same misunderstanding, so these are the tests that
    // can actually tell us the model of the protocol is wrong.
    const REAL_TOKEN_READ: [u8; 37] = [
        0x02, 0x25, 0xA5, 0x71, 0x35, 0x09, 0x05, 0x55, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74, 0x45,
        0x6C, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74, 0x45, 0x6C, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74,
        0x45, 0x6C, 0x65, 0x70, 0x68, 0x61, 0xF9,
    ];
    const REAL_VERSION: [u8; 18] = [
        0x02, 0x12, 0x88, 0x55, 0x39, 0x36, 0x32, 0x48, 0x31, 0x2B, 0x27, 0x65, 0x3A, 0x54, 0x5E,
        0x59, 0x55, 0xA3,
    ];
    const REAL_PLAINTEXT_ACK: [u8; 6] = [0x02, 0x06, 0x00, 0x10, 0x00, 0xE7];
    const REAL_NAK: [u8; 25] = [
        0x02, 0x19, 0x88, 0x13, 0x02, 0x05, 0x88, 0x92, 0xDE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4A,
    ];

    #[test]
    fn decodes_the_token_from_a_real_read() {
        let reply = parse(&REAL_TOKEN_READ).expect("a captured frame must parse");
        assert_eq!(reply.msg_type, ACK);
        let t = token(Read::Mifare, &reply).expect("a card was on the reader");
        assert_eq!(t.hex, "5B7D4039");
        // what Net2 itself showed for this card
        assert_eq!(t.number, 34935097);
    }

    #[test]
    fn decodes_a_hitag2_fob_in_the_net2_layout() {
        // Synthetic: no Hitag2 fob has been read yet. Built from Net2's digit
        // table for 12345678, so it only proves the transcription of Net2's
        // decoder, not that a real fob looks like this.
        let codes: Vec<u64> = [1, 2, 3, 4, 5, 6, 7, 8, 15]
            .map(|d| u64::from(DIGIT_CODES[d]))
            .to_vec();
        let page4 = codes[..6].iter().fold(0, |acc, c| acc << 5 | c) << 2;
        let page5 = codes[6..].iter().fold(0, |acc, c| acc << 5 | c) << 17;
        let mut payload = [0u8; 32];
        payload[8..12].copy_from_slice(&(page4 as u32).to_be_bytes());
        payload[12..16].copy_from_slice(&(page5 as u32).to_be_bytes());
        payload[20..24].copy_from_slice(&(0b0100u32 << 2).to_be_bytes());
        assert_eq!(hitag2_number(&payload), Some(12345678));
    }

    #[test]
    fn decodes_a_real_version_string() {
        let reply = parse(&REAL_VERSION).expect("a captured frame must parse");
        assert_eq!(String::from_utf8_lossy(&reply.payload), "USB PES V1.14");
    }

    #[test]
    fn reads_a_plaintext_reply_without_unmasking_it() {
        // address 0 has the high bit clear, so this frame is not obfuscated
        let reply = parse(&REAL_PLAINTEXT_ACK).expect("must parse");
        assert_eq!(reply.msg_type, ACK);
    }

    #[test]
    fn reads_a_nak_even_though_the_address_asks_for_obfuscation() {
        // the reader sends NAKs in the clear regardless of address, which we
        // got wrong at first and it showed up as a nonsense message type
        let reply = parse(&REAL_NAK).expect("must parse");
        assert_eq!(reply.msg_type, NAK);
    }

    #[test]
    fn builds_the_two_frames_net2_sends_for_a_read() {
        assert_eq!(hex(&frame(ADDR, OP_LEDS, &[LEDS_ARG])[..6]), "0206886166A8");
        assert_eq!(hex(&frame(ADDR, OP_READ_MIFARE, &[])[..5]), "02058892DE");
    }

    #[test]
    fn keeps_a_uid_whole_when_it_ends_in_zero() {
        let seven_byte_uid = [0x04, 0x5B, 0x7D, 0x40, 0x39, 0x12, 0x00];
        let mut payload = [0u8; 32];
        payload[..7].copy_from_slice(&seven_byte_uid);
        let reply = parse(&frame(ADDR, ACK, &payload)).expect("must parse");
        assert_eq!(
            token(Read::Mifare, &reply).expect("a card").hex,
            "045B7D40391200"
        );
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn refuses_a_payload_that_would_overwrite_the_checksum() {
        frame(ADDR, ACK, &[0; MAX_PAYLOAD + 1]);
    }

    #[test]
    fn rejects_a_frame_whose_checksum_does_not_match() {
        let mut corrupted = REAL_TOKEN_READ;
        corrupted[6] ^= 0xFF;
        assert!(parse(&corrupted).is_none());
    }

    #[test]
    fn tells_a_read_reply_apart_from_the_prime_ack() {
        // both are acks; only the long one carries a token, and mistaking the
        // short one for a read used to wipe the display straight after a read
        let read = parse(&REAL_TOKEN_READ).expect("must parse");
        let prime_ack = parse(&frame(ADDR, ACK, &[0x00])).expect("must parse");
        assert!(is_read_reply(&read));
        assert!(!is_read_reply(&prime_ack));
    }

    #[test]
    fn an_empty_read_means_no_card_rather_than_an_empty_token() {
        let empty = parse(&frame(ADDR, ACK, &[0u8; 32])).expect("must parse");
        assert!(is_read_reply(&empty));
        assert!(token(Read::Mifare, &empty).is_none());
        assert!(token(Read::Hitag2, &empty).is_none());
    }

    /// Opcodes this app actually builds, plus the reply types the reader
    /// sends back. Sweeping all 256 instead would fail, and fairly: a masked
    /// opcode whose wire byte lands on 0x13 is indistinguishable from a
    /// plaintext NAK. That is an ambiguity in the protocol rather than a bug
    /// here, and it cannot bite, because no opcode in real traffic can
    /// produce that byte under any of the eight key positions.
    fn real_opcodes() -> impl Strategy<Value = u8> {
        prop::sample::select(vec![
            ACK,
            NAK,
            0x00,
            0x14,
            0x24,
            0x25,
            0x28,
            0x64,
            OP_READ_MIFARE,
        ])
    }

    proptest! {
        /// The obfuscation phase comes from the address, so a bug there would
        /// only show up at some addresses. Sweeping them is exactly what a
        /// property test is for.
        #[test]
        fn a_built_frame_parses_back_to_what_went_in(
            addr: u8,
            opcode in real_opcodes(),
            payload in prop::collection::vec(any::<u8>(), 0..=MAX_PAYLOAD),
        ) {
            let reply = parse(&frame(addr, opcode, &payload)).expect("must parse");
            prop_assert_eq!(reply.msg_type, opcode);
            prop_assert_eq!(reply.payload, payload);
        }

        /// Masking is its own inverse, which is what lets one routine both
        /// build and read frames.
        #[test]
        fn masking_twice_returns_the_original(addr: u8, body in prop::collection::vec(any::<u8>(), 0..40)) {
            let mut scrambled = body.clone();
            mask(addr, &mut scrambled);
            mask(addr, &mut scrambled);
            prop_assert_eq!(scrambled, body);
        }
    }
}
