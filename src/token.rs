//! Turning a read reply into the token number Net2 would show.

use crate::protocol::{OP_READ_HITAG2, OP_READ_MIFARE, Reply, hex, is_read_reply};

/// Which kind of token a read asks the reader for. Net2 cycles through five;
/// these are the two implemented here.
#[derive(Clone, Copy, PartialEq)]
pub enum Read {
    Mifare,
    /// Beta: decoded exactly as Net2 does, but never tested on a real fob.
    Hitag2,
}

impl Read {
    pub fn opcode(self) -> u8 {
        match self {
            Read::Mifare => OP_READ_MIFARE,
            Read::Hitag2 => OP_READ_HITAG2,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct Token {
    pub read: Read,
    pub hex: String,
    /// What Net2 shows as the token number.
    pub number: u32,
}

/// The token, if this reply is a read that found a card. An empty payload
/// means the reader answered but there was nothing on it.
pub fn token(read: Read, reply: &Reply) -> Option<Token> {
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::protocol::{ACK, ADDR, INIT, REAL_TOKEN_READ, frame, parse};

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
    fn keeps_a_seven_byte_uid_whole_when_it_ends_in_two_zeros() {
        let mut payload = [0u8; 32];
        payload[..5].copy_from_slice(&[0x04, 0x5B, 0x7D, 0x40, 0x39]);
        let reply = parse(&frame(ADDR, ACK, &payload)).expect("must parse");
        assert_eq!(
            token(Read::Mifare, &reply).expect("a card").hex,
            "045B7D40390000"
        );
    }

    #[test]
    fn asks_with_the_opcodes_net2_uses() {
        // the Mifare read as captured from Net2, and the read in its handshake
        assert_eq!(
            hex(&frame(ADDR, Read::Mifare.opcode(), &[])[..5]),
            "02058892DE"
        );
        assert_eq!(Read::Hitag2.opcode(), INIT[1].1);
    }

    /// Pages 2 to 7 as TOKEN_R_DATA returns them, four bytes each.
    fn pages(p: [u32; 6]) -> Vec<u8> {
        p.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    /// Put each digit's code at a (page, bit offset) of the newer layout.
    fn place(p: &mut [u32; 6], at: &[(usize, usize)], digits: &[usize]) {
        for (&(page, off), &d) in at.iter().zip(digits) {
            p[page - 2] |= u32::from(DIGIT_CODES[d]) << (27 - off);
        }
    }

    /// Raw 5-bit codes in the classic layout: six from the top of page 4,
    /// the rest from the top of page 5. A 0 here is the magstripe padding.
    fn net2_bits(codes: &[u8]) -> u64 {
        let starts = (0..6).map(|n| 5 * n).chain((0..).map(|n| 32 + 5 * n));
        codes
            .iter()
            .zip(starts)
            .fold(0, |bits, (c, at)| bits | u64::from(*c) << (59 - at))
    }

    fn codes(digits: &[usize]) -> Vec<u8> {
        digits.iter().map(|d| DIGIT_CODES[*d]).collect()
    }

    #[test]
    fn decodes_a_hitag2_user_card_in_the_newer_layout() {
        // Synthetic, like the classic-layout test: proves the transcription.
        let mut p = [0u32; 6];
        p[5] |= 0b0001 << 2;
        place(&mut p, &CARD_TYPE_DIGITS, &[0, 0, 1]);
        place(&mut p, &USER_CARD_DIGITS, &[8, 7, 6, 5, 4, 3, 2, 9]);
        assert_eq!(hitag2_number(&pages(p)), Some(87654329));
    }

    #[test]
    fn decodes_the_45_bit_layout_when_the_card_type_is_not_a_user_card() {
        let bits = net2_bits(&codes(&[1, 2, 3, 4, 5, 6, 7, 8]));
        let mut p = [0, 0, (bits >> 32) as u32, bits as u32, 0, 0b0001 << 2];
        place(&mut p, &CARD_TYPE_DIGITS, &[0, 0, 2]);
        assert_eq!(hitag2_number(&pages(p)), Some(12345678));
    }

    #[test]
    fn a_number_can_end_in_zero_padding() {
        assert_eq!(net2_number(net2_bits(&codes(&[1, 2])), 64), Some(12));
    }

    #[test]
    fn rejects_anything_after_the_zero_padding() {
        let mut junk = codes(&[1, 2]);
        junk.extend([0, DIGIT_CODES[2]]);
        assert_eq!(net2_number(net2_bits(&junk), 64), None);
    }

    #[test]
    fn ignores_bits_past_the_45_bit_length() {
        let bits = net2_bits(&codes(&[1, 2])) | 1 << (63 - 50);
        assert_eq!(net2_number(bits, 45), Some(12));
        // the same bit inside a 64-bit number is junk after the padding
        assert_eq!(net2_number(bits, 64), None);
    }

    #[test]
    fn stops_reading_digits_at_the_45_bit_length() {
        // a ninth digit sits just past the end and must not shift the number
        let bits = net2_bits(&codes(&[1, 2, 3, 4, 5, 6, 7, 8, 9]));
        assert_eq!(net2_number(bits, 45), Some(12345678));
    }

    #[test]
    fn reads_leading_zero_padding_as_zeros() {
        let mut padded = vec![0, 0];
        padded.extend(codes(&[1, 2, 3, 4, 15]));
        assert_eq!(net2_number(net2_bits(&padded), 64), Some(1234));
    }

    #[test]
    fn keeps_the_last_eight_digits_of_a_longer_number() {
        let bits = net2_bits(&codes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 1, 2, 3]));
        assert_eq!(net2_number(bits, 64), Some(56789123));
    }

    #[test]
    fn an_empty_read_means_no_card_rather_than_an_empty_token() {
        let empty = parse(&frame(ADDR, ACK, &[0u8; 32])).expect("must parse");
        assert!(is_read_reply(&empty));
        assert!(token(Read::Mifare, &empty).is_none());
        assert!(token(Read::Hitag2, &empty).is_none());
    }

    use proptest::prelude::*;

    proptest! {
        /// Whatever the reader sends, decoding it cannot take the page down,
        /// and any number found is one Net2 could show.
        #[test]
        fn no_reply_makes_decoding_panic(bytes in prop::collection::vec(any::<u8>(), 0..=41)) {
            if let Some(reply) = parse(&bytes) {
                for read in [Read::Mifare, Read::Hitag2] {
                    if let Some(t) = token(read, &reply) {
                        prop_assert!(t.number < 100_000_000);
                    }
                }
            }
        }

        /// The same, with every frame well formed, so the decoders see far
        /// more payloads than the checksum would otherwise let through.
        #[test]
        fn any_read_payload_decodes_to_an_eight_digit_number_or_nothing(
            payload in prop::collection::vec(any::<u8>(), 16..=35),
        ) {
            let reply = parse(&frame(ADDR, ACK, &payload)).expect("must parse");
            for read in [Read::Mifare, Read::Hitag2] {
                if let Some(t) = token(read, &reply) {
                    prop_assert!(t.number < 100_000_000);
                    prop_assert!(read == Read::Mifare || t.number > 0);
                }
            }
        }
    }
}
