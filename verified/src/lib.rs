//! Executable byte-level core shared by Tusk and the Verus proofs.
use vstd::prelude::*;

verus! {

/// The reader declares one 41-byte input report and one 41-byte output report.
pub const REPORT_BYTES: usize = 41;
/// Room for STX, length, address, opcode, checksum and EOT is reserved.
pub const MAX_PAYLOAD: usize = REPORT_BYTES - 6;
/// The reader understood and answered.
pub const ACK: u8 = 0x10;
/// An unmasked "not understood" reply, even at an obfuscated address.
pub const NAK: u8 = 0x13;

pub struct Reply {
    pub msg_type: u8,
    pub payload: Vec<u8>,
}

pub open spec fn key_at(index: int) -> u8 {
    match (index % 8) as u8 {
        0 => 0x45, 1 => 0x6C, 2 => 0x65, 3 => 0x70,
        4 => 0x68, 5 => 0x61, 6 => 0x6E, _ => 0x74,
    }
}

pub open spec fn masked(addr: u8, index: int, byte: u8) -> u8 {
    if addr & 0x80u8 == 0 { byte }
    else { byte ^ key_at(addr as int + index) }
}

pub fn mask_byte(addr: u8, index: usize, byte: u8) -> (out: u8)
    ensures out == masked(addr, index as int, byte),
{
    if addr & 0x80 == 0 { return byte; }
    let key = [0x45u8, 0x6C, 0x65, 0x70, 0x68, 0x61, 0x6E, 0x74];
    // Reduce before adding so even an arbitrary usize index cannot overflow.
    byte ^ key[((addr as usize % 8) + index % 8) % 8]
}

pub proof fn masking_is_its_own_inverse(addr: u8, index: int, byte: u8)
    ensures masked(addr, index, masked(addr, index, byte)) == byte,
{
    let k = key_at(addr as int + index);
    assert((byte ^ k) ^ k == byte) by (bit_vector);
}

pub open spec fn sum(bytes: Seq<u8>) -> u8
    decreases bytes.len(),
{
    if bytes.len() == 0 { 0 }
    else { ((sum(bytes.drop_last()) as int + bytes.last() as int) % 256) as u8 }
}

pub fn checksum(bytes: &[u8]) -> (out: u8)
    ensures out == !sum(bytes@),
{
    let mut total = 0u8;
    let mut i = 0usize;
    while i < bytes.len()
        invariant i <= bytes.len(), total == sum(bytes@.take(i as int)),
        decreases bytes.len() - i,
    {
        proof {
            assert(bytes@.take(i as int + 1).drop_last() == bytes@.take(i as int));
            assert(bytes@.take(i as int + 1).last() == bytes@[i as int]);
        }
        total = total.wrapping_add(bytes[i]);
        i += 1;
    }
    proof { assert(bytes@.take(i as int) == bytes@); }
    total = !total;
    total
}

pub open spec fn frame_body(addr: u8, opcode: u8, data: Seq<u8>) -> Seq<u8> {
    seq![0x02u8, (5 + data.len()) as u8, addr]
        + Seq::new(data.len() + 1, |i: int|
            masked(addr, i, if i == 0 { opcode } else { data[i - 1] }))
}

pub open spec fn built_frame(addr: u8, opcode: u8, data: Seq<u8>) -> Seq<u8> {
    let body = frame_body(addr, opcode, data);
    body + seq![!sum(body)] + Seq::new((35 - data.len()) as nat, |i: int| 0u8) + seq![0x04u8]
}

/// The public protocol wrapper retains the oversized-payload panic.
pub fn frame(addr: u8, opcode: u8, data: &[u8]) -> (out: [u8; REPORT_BYTES])
    requires data.len() <= MAX_PAYLOAD,
    ensures out@ == built_frame(addr, opcode, data@),
{
    let len = 5 + data.len();
    let mut buf = [0u8; REPORT_BYTES];
    buf[0] = 0x02;
    buf[1] = len as u8;
    buf[2] = addr;
    buf[3] = mask_byte(addr, 0, opcode);
    let mut i = 0usize;
    while i < data.len()
        invariant
            data.len() <= 35, len == 5 + data.len(), i <= data.len(),
            buf[0] == 0x02, buf[1] == len as u8, buf[2] == addr,
            buf[3] == masked(addr, 0, opcode),
            forall|j: int| 0 <= j < i ==> buf@[4 + j] == masked(addr, j + 1, data@[j]),
            forall|j: int| 4 + i <= j < 41 ==> buf@[j] == 0,
        decreases data.len() - i,
    {
        buf[4 + i] = mask_byte(addr, i + 1, data[i]);
        i += 1;
    }
    proof {
        assert(buf@.take(len as int - 1) =~= frame_body(addr, opcode, data@));
    }
    buf[len - 1] = checksum(&buf[..len - 1]);
    buf[40] = 0x04;
    proof { assert(buf@ =~= built_frame(addr, opcode, data@)); }
    buf
}

pub open spec fn valid_frame(bytes: Seq<u8>) -> bool {
    bytes.len() >= 5 && bytes[0] == 0x02
        && 5 <= bytes[1] as int <= bytes.len()
        && bytes[bytes[1] as int - 1] == !sum(bytes.take(bytes[1] as int - 1))
}

/// Every allowed payload at every address round-trips, except when the wire
/// opcode is indistinguishable from the protocol's plaintext NAK exception.
pub proof fn frame_roundtrip(addr: u8, opcode: u8, data: Seq<u8>)
    requires
        data.len() <= MAX_PAYLOAD,
        addr & 0x80u8 == 0 || masked(addr, 0, opcode) != NAK,
    ensures
        valid_frame(built_frame(addr, opcode, data)),
        decoded(built_frame(addr, opcode, data), 0) == opcode,
        Seq::new(data.len(), |i: int| decoded(built_frame(addr, opcode, data), i + 1)) == data,
{
    let body = frame_body(addr, opcode, data);
    let bytes = built_frame(addr, opcode, data);
    assert(bytes.take(4 + data.len() as int) =~= body);
    masking_is_its_own_inverse(addr, 0, opcode);
    assert forall|i: int| 0 <= i < data.len() implies
        decoded(bytes, i + 1) == data[i] by {
        masking_is_its_own_inverse(addr, i + 1, data[i]);
    }
    assert(Seq::new(data.len(), |i: int| decoded(bytes, i + 1)) =~= data);
}

/// Commands used by Tusk and reply types in the captured protocol all avoid
/// that ambiguity, for every possible address/key phase.
pub proof fn used_opcodes_are_unambiguous(addr: u8, opcode: u8)
    requires
        opcode == ACK || opcode == NAK || opcode == 0x12 || opcode == 0x00
            || opcode == 0x14 || opcode == 0x24 || opcode == 0x25
            || opcode == 0x28 || opcode == 0x64 || opcode == 0xD7,
    ensures addr & 0x80u8 == 0 || masked(addr, 0, opcode) != NAK,
{
    let k = key_at(addr as int);
    assert(k == 0x45 || k == 0x6C || k == 0x65 || k == 0x70
        || k == 0x68 || k == 0x61 || k == 0x6E || k == 0x74);
    assert((
        (opcode == 0x10u8 || opcode == 0x13u8 || opcode == 0x12u8 || opcode == 0x00u8
            || opcode == 0x14u8 || opcode == 0x24u8 || opcode == 0x25u8
            || opcode == 0x28u8 || opcode == 0x64u8 || opcode == 0xD7u8)
        && (k == 0x45u8 || k == 0x6Cu8 || k == 0x65u8 || k == 0x70u8
            || k == 0x68u8 || k == 0x61u8 || k == 0x6Eu8 || k == 0x74u8)
    ) ==> opcode ^ k != 0x13u8) by (bit_vector);
}

pub open spec fn decoded(bytes: Seq<u8>, index: int) -> u8 {
    if bytes[3] == NAK { bytes[index + 3] }
    else { masked(bytes[2], index, bytes[index + 3]) }
}

/// Accepts arbitrary slices, including truncated reports and malformed lengths.
pub fn parse(bytes: &[u8]) -> (out: Option<Reply>)
    ensures
        out.is_some() == valid_frame(bytes@),
        out.is_some() ==> {
            let reply = out.unwrap();
            reply.msg_type == decoded(bytes@, 0)
                && reply.payload@ == Seq::new((bytes@[1] as int - 5) as nat,
                    |i: int| decoded(bytes@, i + 1))
        },
{
    if bytes.len() < 5 { return None; }
    let len = bytes[1] as usize;
    if bytes[0] != 0x02 || len < 5 || len > bytes.len() { return None; }
    if bytes[len - 1] != checksum(&bytes[..len - 1]) { return None; }
    let raw = bytes[3] == NAK;
    let msg_type = if raw { bytes[3] } else { mask_byte(bytes[2], 0, bytes[3]) };
    let mut payload = Vec::new();
    let mut i = 0usize;
    while i < len - 5
        invariant
            5 <= len <= bytes.len(), len == bytes@[1] as usize,
            valid_frame(bytes@), raw == (bytes[3] == NAK),
            i <= len - 5, payload.len() == i,
            forall|j: int| 0 <= j < i ==> payload@[j] == decoded(bytes@, j + 1),
        decreases len - 5 - i,
    {
        let byte = if raw { bytes[4 + i] } else { mask_byte(bytes[2], i + 1, bytes[4 + i]) };
        payload.push(byte);
        i += 1;
    }
    proof {
        assert(payload@ =~= Seq::new((bytes@[1] as int - 5) as nat, |j: int| decoded(bytes@, j + 1)));
    }
    Some(Reply { msg_type, payload })
}

pub fn is_read_reply(reply: &Reply) -> (out: bool)
    ensures out == (reply.msg_type == ACK && reply.payload.len() >= 16),
{
    reply.msg_type == ACK && reply.payload.len() >= 16
}

pub open spec fn uid_end(payload: Seq<u8>, end: int) -> bool {
    0 < end <= payload.len() && payload[end - 1] != 0
        && forall|i: int| end <= i < payload.len() ==> payload[i] == 0
}

pub open spec fn uid_len(end: int) -> int {
    if end <= 4 { 4 } else if end <= 7 { 7 } else if end <= 10 { 10 } else { end }
}

pub open spec fn net2_number(payload: Seq<u8>) -> int {
    (payload[0] as int * 16777216 + payload[1] as int * 65536
        + payload[2] as int * 256 + payload[3] as int) % 100000000
}

/// UID bytes and Net2's eight-digit number, before display formatting.
pub fn token_data(reply: &Reply) -> (out: Option<(Vec<u8>, u32)>)
    ensures
        out.is_some() == (reply.msg_type == ACK && reply.payload.len() >= 16
            && exists|i: int| 0 <= i < reply.payload.len() && reply.payload@[i] != 0),
        out.is_some() ==> {
            let (uid, number) = out.unwrap();
            number < 100000000 && number as int == net2_number(reply.payload@)
                && exists|end: int| #[trigger] uid_end(reply.payload@, end)
                    && uid@ == reply.payload@.take(uid_len(end))
        },
{
    if !is_read_reply(reply) { return None; }
    let p = &reply.payload;
    let mut end = p.len();
    while end > 0
        invariant
            p.len() >= 16, end <= p.len(),
            forall|i: int| end <= i < p.len() ==> p@[i] == 0,
        ensures end == 0 || p@[end as int - 1] != 0,
        decreases end,
    {
        if p[end - 1] != 0 { break; }
        end -= 1;
    }
    if end == 0 { return None; }
    let len = if end <= 4 { 4 } else if end <= 7 { 7 } else if end <= 10 { 10 } else { end };
    let value = p[0] as u32 * 16777216 + p[1] as u32 * 65536
        + p[2] as u32 * 256 + p[3] as u32;
    let uid = vstd::slice::slice_to_vec(&p.as_slice()[..len]);
    proof {
        assert(uid_end(p@, end as int));
        assert(exists|e: int| #[trigger] uid_end(p@, e) && uid@ == p@.take(uid_len(e)));
        assert(exists|i: int| 0 <= i < p.len() && p@[i] != 0) by {
            assert(p@[end as int - 1] != 0);
        }
    }
    Some((uid, value % 100000000))
}

} // verus!
