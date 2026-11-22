//! Framing for the reader's HID reports: building commands and parsing replies.

// These implementations are checked by Verus; display formatting stays here.
use tusk_verified::MAX_PAYLOAD;
pub use tusk_verified::{ACK, REPORT_BYTES, Reply, is_read_reply, parse};

/// Any address with the high bit set works; it only picks the key phase.
pub const ADDR: u8 = 0x88;

/// Net2 sets the LEDs before every read (RWD_LEDS), and the reader stays idle
/// without it. Opcode names are Paxton's, from the BOARD_CMD enum in Net2.
pub const OP_LEDS: u8 = 0x24;
pub const LEDS_ARG: u8 = 0x0A;
/// Net2's token-found LED state, used after a successful assignment.
pub const LEDS_ASSIGNED: u8 = 0x06;

/// RWD_READ_MIFARE: answers with the card's UID, then zero padding.
pub const OP_READ_MIFARE: u8 = 0xD7;

/// A read found nothing; the one payload byte says which kind of read. Only
/// Mifare's matters here: the 0x28 that answers the handshake's TOKEN_R_DATA
/// is for 125 kHz tokens, which this app does not read.
pub const NO_TOKEN: u8 = 0x12;
pub const NO_MIFARE: u8 = 0x01;

/// What Net2 sends once before it starts polling. Without it the reader
/// refuses the read, which is what a replug leaves us with. It is sent
/// exactly as captured, 125 kHz read and all, since which part wakes the
/// reader is unknown.
pub const INIT: [(u8, u8, &[u8]); 5] = [
    (0x08, 0x25, &[]),            // RWD_OPEN_LINK
    (0x88, 0x14, &[]),            // TOKEN_R_DATA
    (0xCD, 0x28, &[]),            // RWD_SERIAL_NUMBER
    (ADDR, OP_LEDS, &[LEDS_ARG]), // RWD_LEDS
    (ADDR, 0x00, &[]),
];

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// Build a frame: STX, length, address, opcode, payload, checksum, EOT. The
/// length counts STX through checksum, which is how the reader frames replies.
/// Panics above MAX_PAYLOAD rather than silently truncating a command.
pub fn frame(addr: u8, opcode: u8, data: &[u8]) -> [u8; REPORT_BYTES] {
    // one byte more and the EOT below would overwrite the checksum
    assert!(
        data.len() <= MAX_PAYLOAD,
        "payload does not fit in one report"
    );
    tusk_verified::frame(addr, opcode, data)
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
    use rstest::rstest;
    use tusk_verified::{NAK, checksum, mask_byte};

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

    #[rstest]
    #[case::leds(OP_LEDS, &[LEDS_ARG], "0206886166A8")]
    #[case::read_mifare(OP_READ_MIFARE, &[], "02058892DE")]
    fn builds_the_two_frames_net2_sends_for_a_read(
        #[case] opcode: u8,
        #[case] payload: &[u8],
        #[case] expected: &str,
    ) {
        assert_eq!(
            hex(&frame(ADDR, opcode, payload)[..expected.len() / 2]),
            expected
        );
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
            let scrambled: Vec<u8> = body.iter().enumerate()
                .map(|(i, b)| mask_byte(addr, i, mask_byte(addr, i, *b)))
                .collect();
            prop_assert_eq!(scrambled, body);
        }
    }
}
