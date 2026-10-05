//! Turning a read reply into the token number Net2 would show.

use crate::protocol::{Reply, hex, is_read_reply};

/// A Mifare card, the only kind of token this app reads.
#[derive(Clone, PartialEq)]
pub struct Token {
    pub hex: String,
    /// What Net2 shows as the token number.
    pub number: u32,
}

/// The token, if this reply is a read that found a card. An empty payload
/// means the reader answered but there was nothing on it.
pub fn token(reply: &Reply) -> Option<Token> {
    if !is_read_reply(reply) {
        return None;
    }
    let p = &reply.payload;
    let end = p.iter().rposition(|b| *b != 0)? + 1;
    // Mifare UIDs come in 4, 7 or 10 bytes, so one ending in 00 keeps its
    // last byte rather than being cut at the zero padding
    let len = [4, 7, 10].into_iter().find(|n| *n >= end).unwrap_or(end);
    let uid = &p[..len];
    // Net2 reads the first four bytes big-endian and keeps 8 digits
    let number = u32::from_be_bytes(uid[..4].try_into().ok()?) % 100_000_000;
    Some(Token {
        hex: hex(uid),
        number,
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::protocol::{ACK, ADDR, OP_READ_MIFARE, REAL_TOKEN_READ, frame, parse};

    #[test]
    fn decodes_the_token_from_a_real_read() {
        let reply = parse(&REAL_TOKEN_READ).expect("a captured frame must parse");
        assert_eq!(reply.msg_type, ACK);
        let t = token(&reply).expect("a card was on the reader");
        assert_eq!(t.hex, "5B7D4039");
        // what Net2 itself showed for this card
        assert_eq!(t.number, 34935097);
    }

    #[test]
    fn keeps_a_uid_whole_when_it_ends_in_zero() {
        let seven_byte_uid = [0x04, 0x5B, 0x7D, 0x40, 0x39, 0x12, 0x00];
        let mut payload = [0u8; 32];
        payload[..7].copy_from_slice(&seven_byte_uid);
        let reply = parse(&frame(ADDR, ACK, &payload)).expect("must parse");
        assert_eq!(token(&reply).expect("a card").hex, "045B7D40391200");
    }

    #[test]
    fn keeps_a_seven_byte_uid_whole_when_it_ends_in_two_zeros() {
        let mut payload = [0u8; 32];
        payload[..5].copy_from_slice(&[0x04, 0x5B, 0x7D, 0x40, 0x39]);
        let reply = parse(&frame(ADDR, ACK, &payload)).expect("must parse");
        assert_eq!(token(&reply).expect("a card").hex, "045B7D40390000");
    }

    #[test]
    fn asks_with_the_opcode_net2_uses() {
        // the Mifare read as captured from Net2
        assert_eq!(hex(&frame(ADDR, OP_READ_MIFARE, &[])[..5]), "02058892DE");
    }

    #[test]
    fn an_empty_read_means_no_card_rather_than_an_empty_token() {
        let empty = parse(&frame(ADDR, ACK, &[0u8; 32])).expect("must parse");
        assert!(is_read_reply(&empty));
        assert!(token(&empty).is_none());
    }

    use proptest::prelude::*;

    proptest! {
        /// Whatever the reader sends, decoding it cannot take the page down,
        /// and any number found is one Net2 could show.
        #[test]
        fn no_reply_makes_decoding_panic(bytes in prop::collection::vec(any::<u8>(), 0..=41)) {
            if let Some(reply) = parse(&bytes)
                && let Some(t) = token(&reply)
            {
                prop_assert!(t.number < 100_000_000);
            }
        }

        /// The same, with every frame well formed, so the decoder sees far
        /// more payloads than the checksum would otherwise let through.
        #[test]
        fn any_read_payload_decodes_to_an_eight_digit_number_or_nothing(
            payload in prop::collection::vec(any::<u8>(), 16..=35),
        ) {
            let reply = parse(&frame(ADDR, ACK, &payload)).expect("must parse");
            if let Some(t) = token(&reply) {
                prop_assert!(t.number < 100_000_000);
            }
        }
    }
}
