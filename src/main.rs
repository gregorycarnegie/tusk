use leptos::prelude::*;
use leptos::reactive::owner::StoredValue;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use web_sys::{
    HidConnectionEvent, HidDevice, HidDeviceFilter, HidDeviceRequestOptions,
    HidInputReportEvent,
};

/// Paxton Net2 desktop reader: USB\VID_1071&PID_0001, HID vendor-defined.
const PAXTON_VID: u32 = 0x1071;

/// The reader declares one 41-byte input report and one 41-byte output report.
const REPORT_BYTES: usize = 41;

/// Any address with the high bit set works; it only picks the key phase.
const ADDR: u8 = 0x88;

/// Reading a token takes two messages: prime the reader, then collect.
/// Net2 does exactly this, and the reader stays idle without the first one.
const OP_PRIME: u8 = 0x24;
const PRIME_ARG: u8 = 0x0A;
const OP_READ: u8 = 0xD7;

/// Fast enough to feel instant, slow enough to leave the reader alone.
const POLL_MS: i32 = 250;

/// A reset leaves the write promise pending forever, so give up on it.
const SEND_TIMEOUT_MS: i32 = 500;

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
    (0x08, 0x25, &[]),
    (0x88, 0x14, &[]),
    (0xCD, 0x28, &[]),
    (ADDR, OP_PRIME, &[PRIME_ARG]),
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

/// Build a frame: STX, length, address, opcode, payload, checksum, EOT. The
/// length counts STX through checksum, which is how the reader frames replies.
fn frame(addr: u8, opcode: u8, data: &[u8]) -> [u8; REPORT_BYTES] {
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

/// A read answers with a long padded frame; the prime answers with a single
/// zero byte. Telling them apart stops the prime ack wiping a fresh token.
fn is_read_reply(reply: &Reply) -> bool {
    reply.msg_type == ACK && reply.payload.len() >= 16
}

/// The token, if this reply is a read that found a card. An empty payload
/// means the reader answered but there was nothing on it.
fn token(reply: &Reply) -> Option<String> {
    let end = reply.payload.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    match reply.msg_type == ACK && (4..=8).contains(&end) {
        true => Some(hex(&reply.payload[..end])),
        false => None,
    }
}

/// A real token read captured from the Net2 software, at address 0xA5.
#[cfg(debug_assertions)]
const CAPTURED_READ: [u8; 37] = [
    0x02, 0x25, 0xA5, 0x71, 0x35, 0x09, 0x05, 0x55, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74, 0x45,
    0x6C, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74, 0x45, 0x6C, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74,
    0x45, 0x6C, 0x65, 0x70, 0x68, 0x61, 0xF9,
];

fn main() {
    // the obfuscated read must decode back to the token it really carried
    debug_assert_eq!(
        parse(&CAPTURED_READ).as_ref().and_then(token).as_deref(),
        Some("5B7D4039")
    );
    // the prime we build must match, byte for byte, what Net2 sends
    debug_assert_eq!(
        hex(&frame(ADDR, OP_PRIME, &[PRIME_ARG])[..6]),
        "0206886166A8"
    );
    // and the read must match the frame that returned a token on this machine
    debug_assert_eq!(hex(&frame(ADDR, OP_READ, &[])[..5]), "02058892DE");
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

/// Ask the reader for whatever token is on it. The answer to the read arrives
/// separately, as an input report.
async fn poll_once(dev: &HidDevice) -> Result<(), JsValue> {
    send(dev, frame(ADDR, OP_PRIME, &[PRIME_ARG])).await?;
    send(dev, frame(ADDR, OP_READ, &[])).await
}

/// Listen for replies, then keep asking until the reader goes away.
async fn run(
    dev: HidDevice,
    card: RwSignal<Option<String>>,
    status: RwSignal<String>,
    held: StoredValue<Option<HidDevice>, LocalStorage>,
) {
    if !dev.opened() {
        if let Err(e) = dev.open().await {
            status.set(format!("Could not open the reader: {e:?}"));
            return;
        }
    }

    let listener = dev.clone();
    let cb = Closure::<dyn FnMut(HidInputReportEvent)>::new(move |ev: HidInputReportEvent| {
        // keep `listener` alive: Chrome stops delivering reports once the
        // HIDDevice wrapper is garbage collected
        let _ = &listener;
        let data = ev.data();
        let bytes: Vec<u8> = (0..data.byte_length()).map(|i| data.get_uint8(i)).collect();
        // the prime and the handshake answer too, so only act on reads
        if let Some(reply) = parse(&bytes).filter(is_read_reply) {
            card.set(token(&reply));
        }
    });
    dev.set_oninputreport(Some(cb.as_ref().unchecked_ref()));
    cb.forget();
    held.set_value(Some(dev.clone()));

    for (addr, opcode, args) in INIT {
        if let Err(e) = send(&dev, frame(addr, opcode, args)).await {
            status.set(format!("Reader would not start: {e:?}"));
            held.set_value(None);
            return;
        }
        sleep(40).await;
    }

    status.set("Ready - present a token".into());
    while poll_once(&dev).await.is_ok() {
        sleep(POLL_MS).await;
    }
    held.set_value(None);
    card.set(None);
    status.set("Reader disconnected - plug it back in".into());
}

#[component]
fn App() -> impl IntoView {
    let card = RwSignal::new(None::<String>);
    let status = RwSignal::new("Not connected".to_string());
    let held = StoredValue::new_local(None::<HidDevice>);

    let has_hid = js_sys::Reflect::has(&web_sys::window().unwrap().navigator(), &"hid".into())
        .unwrap_or(false);
    if !has_hid {
        status.set("WebHID is not available - open this page in Chrome or Edge".into());
    }

    // reuse a reader the browser already has permission for, so a reload
    // does not mean clicking through the picker again
    spawn_local(async move {
        if !has_hid {
            return;
        }
        if let Ok(devices) = hid().get_devices().await {
            if let Some(dev) = devices.iter().find(|d| u32::from(d.vendor_id()) == PAXTON_VID) {
                run(dev, card, status, held).await;
            }
        }
    });

    // the reader coming back on the bus should just work, without a click
    if has_hid {
        let cb = Closure::<dyn FnMut(HidConnectionEvent)>::new(move |ev: HidConnectionEvent| {
            let dev = ev.device();
            if u32::from(dev.vendor_id()) == PAXTON_VID
                && !held.get_value().is_some_and(|d| d.opened())
            {
                spawn_local(async move { run(dev, card, status, held).await });
            }
        });
        hid().set_onconnect(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }

    let connect = move |_| {
        spawn_local(async move {
            if !has_hid {
                return;
            }
            // already polling: the picker is only for granting permission, so
            // say so rather than appearing to do nothing
            if held.get_value().is_some_and(|d| d.opened()) {
                status.set("Already connected - present a token".into());
                return;
            }
            let filter = HidDeviceFilter::new();
            filter.set_vendor_id(PAXTON_VID);
            let opts = HidDeviceRequestOptions::new(&[filter]);
            match hid().request_device(&opts).await {
                Ok(devices) => match devices.get_checked(0) {
                    Some(dev) => run(dev, card, status, held).await,
                    None => status.set("No reader selected".into()),
                },
                Err(e) => status.set(format!("Could not reach the reader: {e:?}")),
            }
        })
    };

    view! {
        <main>
            <h1>"Paxton Net2 reader"</h1>
            <button on:click=connect>"Connect reader"</button>
            <p class="status">{status}</p>
            <div class="token" class:waiting=move || card.get().is_none()>
                {move || card.get().unwrap_or("--------".to_string())}
            </div>
        </main>
    }
}
