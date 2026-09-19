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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::tests::REAL_TOKEN_READ;
    use crate::protocol::{ACK, ADDR, frame, parse};

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
    fn an_empty_read_means_no_card_rather_than_an_empty_token() {
        let empty = parse(&frame(ADDR, ACK, &[0u8; 32])).expect("must parse");
        assert!(is_read_reply(&empty));
        assert!(token(Read::Mifare, &empty).is_none());
        assert!(token(Read::Hitag2, &empty).is_none());
    }
}
