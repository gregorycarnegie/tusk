//! Talking to the reader over WebHID and keeping the page's state in step.

use leptos::prelude::*;
use leptos::reactive::owner::StoredValue;
use wasm_bindgen::prelude::*;
use web_sys::{HidDevice, HidInputReportEvent};

use crate::protocol::{
    ACK, ADDR, INIT, LEDS_ARG, OP_LEDS, REPORT_BYTES, frame, is_read_reply, parse,
};
use crate::token::{Read, Token, token};

/// Paxton Net2 desktop reader: USB\VID_1071&PID_0001, HID vendor-defined.
pub const PAXTON_VID: u32 = 0x1071;

/// Fast enough to feel instant, slow enough to leave the reader alone.
const POLL_MS: i32 = 250;

/// A reset leaves the write promise pending forever, so give up on it.
const SEND_TIMEOUT_MS: i32 = 500;

/// Polls in a row without an answer before the display stops being trusted.
/// Writes can keep succeeding while no replies come back.
const MAX_MISSES: u32 = 4;

const READY: &str = "Ready - present a token";

/// How the status line is coloured.
#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Idle,
    Busy,
    Ready,
    Problem,
}

pub type Status = RwSignal<(Tone, String)>;

pub fn say(status: Status, tone: Tone, message: impl Into<String>) {
    status.set((tone, message.into()));
}

pub fn hid() -> web_sys::Hid {
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
pub struct Link {
    pub dev: Option<HidDevice>,
    /// Bumped by every connection; only the newest may reset shared state.
    session: u32,
    /// Between claiming the reader and it being open, when `opened()` is
    /// still false but a second connection must not start.
    opening: bool,
}

/// Listen for replies, then keep asking until the reader goes away.
pub async fn run(
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
