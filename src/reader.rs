//! Talking to the reader over WebHID and keeping the page's state in step.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::prelude::*;
use web_sys::{HidDevice, HidInputReportEvent};

use crate::{
    batch::Batch,
    protocol::{ACK, ADDR, INIT, LEDS_ARG, OP_LEDS, REPORT_BYTES, frame, is_read_reply, parse},
    token::{Read, Token, token},
};

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

impl Tone {
    /// The status line's `data-tone`, which the stylesheet colours.
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Busy => "busy",
            Self::Ready => "ready",
            Self::Problem => "problem",
        }
    }
}

/// Which tab is showing.
#[derive(Clone, Copy, Default, PartialEq)]
pub enum Tool {
    #[default]
    Reader,
    Batch,
    Portraits,
}

/// Signed in to Net2. Kept in memory only, so a reload signs out.
#[derive(Clone)]
pub struct Session {
    pub origin: String,
    pub token: String,
}

/// Shared by the polling loop and the browser controls, without a UI framework.
pub struct State {
    pub card: Option<Token>,
    pub status: (Tone, String),
    pub link: Link,
    pub copied: String,
    pub batch: Option<Batch>,
    pub batch_filename: String,
    pub batch_error: String,
    pub tool: Tool,
    pub net2: Option<Session>,
    pub net2_status: (Tone, String),
    pub on_change: fn(&Self),
}

impl Default for State {
    fn default() -> Self {
        Self {
            card: None,
            status: (Tone::Idle, "Not connected".into()),
            link: Link::default(),
            copied: String::new(),
            batch: None,
            batch_filename: String::new(),
            batch_error: String::new(),
            tool: Tool::Reader,
            net2: None,
            net2_status: (Tone::Idle, "Not connected".into()),
            on_change: |_| {},
        }
    }
}

impl State {
    pub fn say(&mut self, tone: Tone, message: impl Into<String>) {
        self.status = (tone, message.into());
        if tone != Tone::Ready
            && let Some(batch) = &mut self.batch
        {
            batch.running = false;
        }
        (self.on_change)(self);
    }

    pub fn show(&mut self, card: Option<Token>) {
        if self.card != card {
            if let Some(batch) = &mut self.batch {
                batch.observe(card.as_ref());
            }
            self.copied.clear();
            self.card = card;
            (self.on_change)(self);
        }
    }
}

/// The browser's own message for a failed WebHID call. `{:?}` on the JsValue
/// wraps it in `JsValue(...)` and repeats it as the stack's first line.
pub fn reason(e: &JsValue) -> String {
    e.dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .unwrap_or_else(|| format!("{e:?}"))
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
async fn poll_once(dev: &HidDevice, read: Read, reading: &Cell<Read>) -> Result<(), JsValue> {
    send(dev, frame(ADDR, OP_LEDS, &[LEDS_ARG])).await?;
    reading.set(read);
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
pub async fn run(dev: HidDevice, state: Rc<RefCell<State>>) {
    // Claim the reader before the first await, so startup and the button
    // cannot each start a loop for it. An unplugged reader is closed, which is
    // what lets a new connection in while the old loop winds down.
    if state.borrow().link.opening || state.borrow().link.dev.as_ref().is_some_and(|d| d.opened()) {
        return;
    }
    let session = state.borrow().link.session + 1;
    state.borrow_mut().link = Link {
        dev: Some(dev.clone()),
        session,
        opening: true,
    };
    // not "Ready" until the reader has actually answered a read
    state
        .borrow_mut()
        .say(Tone::Busy, "Connecting to the reader");
    let owned = || state.borrow().link.session == session;
    let release = || {
        state.borrow_mut().link = Link {
            session,
            ..Link::default()
        };
    };

    // nothing else can claim the reader while `opening` is set, so this
    // connection still owns it if opening fails
    if !dev.opened()
        && let Err(e) = dev.open().await
    {
        release();
        state.borrow_mut().say(
            Tone::Problem,
            format!("Could not open the reader: {}", reason(&e)),
        );
        return;
    }
    state.borrow_mut().link.opening = false;

    // the handshake includes a Hitag2 read, so that is what replies answer first
    let reading = Rc::new(Cell::new(Read::Hitag2));
    let misses = Rc::new(Cell::new(0u32));
    let listener = dev.clone();
    let (callback_state, callback_reading, callback_misses) =
        (state.clone(), reading.clone(), misses.clone());
    let cb = Closure::<dyn FnMut(HidInputReportEvent)>::new(move |ev: HidInputReportEvent| {
        // keep `listener` alive: Chrome stops delivering reports once the
        // HIDDevice wrapper is garbage collected
        let _ = &listener;
        if callback_state.borrow().link.session != session {
            return;
        }
        let data = ev.data();
        let bytes: Vec<u8> = (0..data.byte_length()).map(|i| data.get_uint8(i)).collect();
        let Some(reply) = parse(&bytes) else { return };
        // the LED command answers with a short ack; anything else answers the read
        if reply.msg_type == ACK && !is_read_reply(&reply) {
            return;
        }
        // an answer to a read is the only proof the reader is working
        callback_misses.set(0);
        let mut state = callback_state.borrow_mut();
        if state.status.1 != READY {
            state.say(Tone::Ready, READY);
        }
        let read = callback_reading.get();
        match token(read, &reply) {
            Some(t) => state.show(Some(t)),
            // nothing of this kind on the reader, so only a token this kind
            // of read found can have been lifted off
            None if state.card.as_ref().is_some_and(|t| t.read == read) => state.show(None),
            None => {}
        }
    });
    dev.set_oninputreport(Some(cb.as_ref().unchecked_ref()));
    let detach = || {
        // An old session must not remove a new session's listener on the same device.
        if dev.oninputreport().map(JsValue::from).as_ref() == Some(cb.as_ref()) {
            dev.set_oninputreport(None);
        }
    };

    for (addr, opcode, args) in INIT {
        if let Err(e) = send(&dev, frame(addr, opcode, args)).await {
            if owned() {
                release();
                state.borrow_mut().say(
                    Tone::Problem,
                    // a reader left in a bad state only recovers from a cold start
                    format!(
                        "Reader would not start ({}) - unplug it, plug it back in, then click Connect reader",
                        reason(&e)
                    ),
                );
            }
            detach();
            return;
        }
        sleep(40).await;
    }

    let mut reads = [Read::Mifare, Read::Hitag2].into_iter().cycle();
    while owned()
        && poll_once(&dev, reads.next().unwrap(), &reading)
            .await
            .is_ok()
    {
        sleep(POLL_MS).await;
        // the reply lands during the sleep and resets this
        if !owned() {
            break;
        }
        misses.set(misses.get() + 1);
        if misses.get() == MAX_MISSES {
            state.borrow_mut().show(None);
            state
                .borrow_mut()
                .say(Tone::Problem, "The reader is not answering");
        }
    }
    if owned() {
        release();
        state.borrow_mut().show(None);
        // The reader has no USB serial number, so Chrome forgets the
        // permission when it is unplugged and never tells the page it came
        // back. Only the picker can grant it again.
        state.borrow_mut().say(
            Tone::Idle,
            "Reader unplugged - plug it back in, then click Connect reader",
        );
    }
    detach();
}

/// These run in a real browser against a fake reader: `cargo test` (wasm32)
/// hands them to wasm-bindgen-test-runner. The fake answers from our model of
/// the reader, with the one real card read captured from it, so they prove
/// the polling logic, not the protocol - that is what the host tests are for.
#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use wasm_bindgen_futures::spawn_local;
    use wasm_bindgen_test::*;

    use super::*;
    use crate::protocol::{OP_READ_HITAG2, OP_READ_MIFARE, REAL_TOKEN_READ};

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen(inline_js = r#"
        // web-sys reaches HIDDevice members by name, so a plain object with
        // the same ones stands in for the reader. `respond` maps each frame
        // sent to the frame the reader answers with, if any.
        export function fake_reader(respond, open_ms) {
            return {
                opened: false,
                oninputreport: null,
                sent: [],
                unplugged: false,
                hang: false,
                slow_acks: false,
                open() {
                    return new Promise(done => setTimeout(() => {
                        this.opened = true;
                        done();
                    }, open_ms));
                },
                sendReport(id, data) {
                    if (this.unplugged) {
                        // a reset reader can leave the write pending for good
                        return this.hang
                            ? new Promise(() => {})
                            : Promise.reject(new DOMException("unplugged", "NotFoundError"));
                    }
                    const frame = new Uint8Array(data);
                    this.sent.push(frame);
                    const reply = respond(frame);
                    if (reply) {
                        // the length byte: short acks can be held back
                        const ms = this.slow_acks && reply[1] < 16 ? 100 : 5;
                        setTimeout(() => this.oninputreport?.({ data: new DataView(reply.buffer) }), ms);
                    }
                    return Promise.resolve();
                },
            };
        }
    "#)]
    extern "C" {
        fn fake_reader(
            respond: &Closure<dyn FnMut(Vec<u8>) -> Option<Vec<u8>>>,
            open_ms: u32,
        ) -> HidDevice;
    }

    const CARD: u32 = 34935097;
    const UNPLUGGED: &str = "Reader unplugged";
    const NOT_ANSWERING: &str = "The reader is not answering";

    struct Fake {
        dev: HidDevice,
        /// The captured card is on the reader.
        card: Rc<Cell<bool>>,
        /// Writes succeed but nothing comes back.
        silent: Rc<Cell<bool>>,
    }

    impl Fake {
        /// A reader whose `open()` takes `open_ms` to settle.
        fn new(open_ms: u32) -> Self {
            let card = Rc::new(Cell::new(false));
            let silent = Rc::new(Cell::new(false));
            let (on, quiet) = (card.clone(), silent.clone());
            let respond =
                Closure::<dyn FnMut(Vec<u8>) -> Option<Vec<u8>>>::new(move |sent: Vec<u8>| {
                    let op = parse(&sent)?.msg_type;
                    if quiet.get() {
                        return None;
                    }
                    Some(match op {
                        OP_READ_MIFARE if on.get() => REAL_TOKEN_READ.to_vec(),
                        // No card: the UID's zero padding and nothing else. A
                        // Hitag2 read with no fob is assumed to answer the same.
                        OP_READ_MIFARE | OP_READ_HITAG2 => frame(ADDR, ACK, &[0; 32]).to_vec(),
                        _ => frame(ADDR, ACK, &[0]).to_vec(),
                    })
                });
            let dev = fake_reader(&respond, open_ms);
            respond.forget();
            Self { dev, card, silent }
        }

        fn sent(&self) -> Vec<Vec<u8>> {
            js_sys::Reflect::get(&self.dev, &"sent".into())
                .unwrap()
                .unchecked_into::<js_sys::Array>()
                .iter()
                .map(|f| js_sys::Uint8Array::new(&f).to_vec())
                .collect()
        }

        fn set(&self, key: &str, value: bool) {
            js_sys::Reflect::set(&self.dev, &key.into(), &value.into()).unwrap();
        }

        /// Pull the plug: the device closes, and writes fail or, with `hang`,
        /// never settle.
        fn unplug(&self, hang: bool) {
            self.set("opened", false);
            self.set("hang", hang);
            self.set("unplugged", true);
        }
    }

    /// Ends the test's polling loop rather than leaving it running under the
    /// tests after it.
    impl Drop for Fake {
        fn drop(&mut self) {
            self.unplug(false);
        }
    }

    /// The state the page hands to `run`.
    struct Page(Rc<RefCell<State>>);

    impl Page {
        fn new() -> Self {
            Self(Rc::new(RefCell::new(State::default())))
        }
        fn connect(&self, reader: &Fake) {
            spawn_local(run(reader.dev.clone(), self.0.clone()));
        }
        fn message(&self) -> String {
            self.0.borrow().status.1.clone()
        }
        fn number(&self) -> Option<u32> {
            self.0.borrow().card.as_ref().map(|t| t.number)
        }
    }

    /// Wait up to `ms` for `cond`, checking every 10 ms.
    async fn until(ms: i32, cond: impl Fn() -> bool) -> bool {
        for _ in 0..ms / 10 {
            if cond() {
                return true;
            }
            sleep(10).await;
        }
        cond()
    }

    #[wasm_bindgen_test]
    async fn sends_the_handshake_then_polls_for_mifare_first() {
        let (page, reader) = (Page::new(), Fake::new(0));
        page.connect(&reader);
        assert!(until(2000, || reader.sent().len() >= INIT.len() + 2).await);
        let expected: Vec<Vec<u8>> = INIT
            .iter()
            .map(|(addr, op, args)| frame(*addr, *op, args).to_vec())
            .chain([
                frame(ADDR, OP_LEDS, &[LEDS_ARG]).to_vec(),
                frame(ADDR, OP_READ_MIFARE, &[]).to_vec(),
            ])
            .collect();
        assert_eq!(reader.sent()[..expected.len()], expected[..]);
    }

    #[wasm_bindgen_test]
    async fn shows_the_card_once_the_reader_answers() {
        let (page, reader) = (Page::new(), Fake::new(0));
        reader.card.set(true);
        page.connect(&reader);
        assert!(until(2000, || page.number() == Some(CARD)).await);
        assert_eq!(page.message(), READY);
    }

    #[wasm_bindgen_test]
    async fn keeps_a_mifare_card_through_the_hitag2_polls() {
        let (page, reader) = (Page::new(), Fake::new(0));
        reader.card.set(true);
        page.connect(&reader);
        assert!(until(2000, || page.number() == Some(CARD)).await);
        // about six polls, half of them Hitag2 reads that find nothing
        for _ in 0..150 {
            assert_eq!(page.number(), Some(CARD));
            sleep(10).await;
        }
    }

    #[wasm_bindgen_test]
    async fn ignores_an_led_ack_that_arrives_after_the_read() {
        let (page, reader) = (Page::new(), Fake::new(0));
        reader.card.set(true);
        reader.set("slow_acks", true);
        page.connect(&reader);
        assert!(until(2000, || page.number() == Some(CARD)).await);
        // taken for an empty read, it would wipe the card until the next one
        for _ in 0..150 {
            assert_eq!(page.number(), Some(CARD));
            sleep(10).await;
        }
    }

    #[wasm_bindgen_test]
    async fn clears_the_card_when_it_is_lifted_off() {
        let (page, reader) = (Page::new(), Fake::new(0));
        reader.card.set(true);
        page.connect(&reader);
        assert!(until(2000, || page.number() == Some(CARD)).await);
        reader.card.set(false);
        assert!(until(1000, || page.number().is_none()).await);
        assert_eq!(page.message(), READY);
    }

    #[wasm_bindgen_test]
    async fn stops_trusting_the_card_when_the_reader_goes_quiet() {
        let (page, reader) = (Page::new(), Fake::new(0));
        reader.card.set(true);
        page.connect(&reader);
        assert!(until(2000, || page.number() == Some(CARD)).await);
        reader.silent.set(true);
        assert!(until(2000, || page.message() == NOT_ANSWERING).await);
        assert_eq!(page.number(), None);
        // and trusts it again once the reader is back
        reader.silent.set(false);
        assert!(until(1000, || page.number() == Some(CARD)).await);
        assert_eq!(page.message(), READY);
    }

    #[wasm_bindgen_test]
    async fn is_not_ready_until_the_reader_answers() {
        let (page, reader) = (Page::new(), Fake::new(0));
        reader.silent.set(true);
        page.connect(&reader);
        assert!(until(2000, || page.message() == NOT_ANSWERING).await);
        assert!(page.0.borrow().status.0 == Tone::Problem);
    }

    #[wasm_bindgen_test]
    async fn reports_an_unplug_whose_writes_fail() {
        let (page, reader) = (Page::new(), Fake::new(0));
        reader.card.set(true);
        page.connect(&reader);
        assert!(until(2000, || page.number() == Some(CARD)).await);
        reader.unplug(false);
        assert!(until(1000, || page.message().starts_with(UNPLUGGED)).await);
        assert_eq!(page.number(), None);
    }

    #[wasm_bindgen_test]
    async fn reports_an_unplug_whose_writes_never_settle() {
        let (page, reader) = (Page::new(), Fake::new(0));
        reader.card.set(true);
        page.connect(&reader);
        assert!(until(2000, || page.number() == Some(CARD)).await);
        reader.unplug(true);
        // only the send timeout gets the loop out of this one
        assert!(until(2000, || page.message().starts_with(UNPLUGGED)).await);
        assert_eq!(page.number(), None);
    }

    #[wasm_bindgen_test]
    async fn refuses_a_second_connection_while_the_first_is_opening() {
        let (page, slow, other) = (Page::new(), Fake::new(200), Fake::new(0));
        page.connect(&slow);
        page.connect(&other);
        assert!(until(2000, || page.message() == READY).await);
        // and once the first is open, the second is still refused
        page.connect(&other);
        sleep(300).await;
        assert!(other.sent().is_empty());
        assert!(!slow.sent().is_empty());
    }

    #[wasm_bindgen_test]
    async fn an_old_loop_winding_down_leaves_the_new_connection_alone() {
        let (page, old) = (Page::new(), Fake::new(0));
        old.card.set(true);
        page.connect(&old);
        assert!(until(2000, || page.number() == Some(CARD)).await);
        // the old loop can be stuck in a write for up to SEND_TIMEOUT_MS
        old.unplug(true);
        // one that fails at once in between, so its release must not hand
        // the old loop's session number out again
        let failed = Fake::new(0);
        failed.unplug(false);
        page.connect(&failed);
        assert!(
            until(200, || page
                .message()
                .starts_with("Reader would not start (unplugged)"))
            .await
        );
        let new = Fake::new(0);
        new.card.set(true);
        page.connect(&new);
        // it gives up while the new one is polling, and must not reset it
        for _ in 0..120 {
            assert!(!page.message().starts_with(UNPLUGGED), "{}", page.message());
            assert_eq!(page.number(), Some(CARD));
            sleep(10).await;
        }
        assert!(!new.sent().is_empty());
    }
}
