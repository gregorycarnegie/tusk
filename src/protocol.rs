//! Framing for the reader's HID reports: building commands and parsing replies.

/// The reader declares one 41-byte input report and one 41-byte output report.
pub const REPORT_BYTES: usize = 41;

/// Any address with the high bit set works; it only picks the key phase.
pub const ADDR: u8 = 0x88;

/// Net2 sets the LEDs before every read (RWD_LEDS), and the reader stays idle
/// without it. Opcode names are Paxton's, from the BOARD_CMD enum in Net2.
pub const OP_LEDS: u8 = 0x24;
pub const LEDS_ARG: u8 = 0x0A;

/// RWD_READ_MIFARE: answers with the card's UID, then zero padding.
pub const OP_READ_MIFARE: u8 = 0xD7;

/// TOKEN_R_DATA: answers with Hitag2 pages 2 to 7, four bytes each.
pub const OP_READ_HITAG2: u8 = 0x14;

/// With the high bit set in the address, everything from the type byte onwards
/// is XORed with this repeating key, starting at key index (address mod 8).
/// Zero padding XORs to the key itself, which is why "Elephant" shows up as
/// readable text in raw frames. Addresses without that bit are plaintext.
const OBFUSCATION_KEY: [u8; 8] = *b"Elephant";

/// Decoded reply type: the reader understood and answered.
pub const ACK: u8 = 0x10;

/// "I did not understand", echoing our own bytes back. Sent unobfuscated even
/// when the address asks for obfuscation, so it is read straight off the wire.
const NAK: u8 = 0x13;

/// What Net2 sends once before it starts polling. Without it the reader
/// refuses the read, which is what a replug leaves us with.
pub const INIT: [(u8, u8, &[u8]); 5] = [
    (0x08, 0x25, &[]),            // RWD_OPEN_LINK
    (0x88, OP_READ_HITAG2, &[]),  // TOKEN_R_DATA
    (0xCD, 0x28, &[]),            // RWD_SERIAL_NUMBER
    (ADDR, OP_LEDS, &[LEDS_ARG]), // RWD_LEDS
    (ADDR, 0x00, &[]),
];

pub fn hex(bytes: &[u8]) -> String {
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
pub fn frame(addr: u8, opcode: u8, data: &[u8]) -> [u8; REPORT_BYTES] {
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

pub struct Reply {
    pub msg_type: u8,
    pub payload: Vec<u8>,
}

pub fn parse(bytes: &[u8]) -> Option<Reply> {
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
pub fn is_read_reply(reply: &Reply) -> bool {
    reply.msg_type == ACK && reply.payload.len() >= 16
}

// Frames captured from the real device with USBPcap while the Net2 software
// drove it. Synthesised input would only prove our encoder and decoder share
// the same misunderstanding, so these are the tests that can actually tell us
// the model of the protocol is wrong.

/// A Mifare read with card 5B7D4039 on the reader. Shared by the host and
/// browser tests, so it lives outside either test module.
#[cfg(test)]
pub const REAL_TOKEN_READ: [u8; 37] = [
    0x02, 0x25, 0xA5, 0x71, 0x35, 0x09, 0x05, 0x55, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74, 0x45, 0x6C,
    0x65, 0x70, 0x68, 0x61, 0x6E, 0x74, 0x45, 0x6C, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74, 0x45, 0x6C,
    0x65, 0x70, 0x68, 0x61, 0xF9,
];

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
    fn pads_with_zeros_and_ends_with_eot() {
        // the padding after the checksum is sent unmasked, as Net2 sends it
        let built = frame(ADDR, OP_LEDS, &[LEDS_ARG]);
        assert!(built[6..40].iter().all(|b| *b == 0), "{}", hex(&built));
        assert_eq!(built[40], 0x04);
    }

    #[test]
    fn rejects_a_frame_without_the_start_byte() {
        // a valid checksum, so only the missing STX can reject it
        let mut bytes = [0x03, 0x06, 0x00, ACK, 0x00, 0x00];
        bytes[5] = checksum(&bytes[..5]);
        assert!(parse(&bytes).is_none());
    }

    #[test]
    fn rejects_a_frame_shorter_than_its_length_byte() {
        assert!(parse(&REAL_PLAINTEXT_ACK[..5]).is_none());
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
