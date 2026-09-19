# Paxton Net2 desktop reader - USB protocol

Reverse engineered from the device plus one USBPcap capture of the Net2
software. Everything here is verified against real traffic, and the app
asserts the key parts against captured bytes at startup.

## Device

    USB\VID_1071&PID_0001        VID 0x1071 is Paxton Access Ltd
    HID, vendor-defined          usage page 0xFFA0, usage 0x01
    one 41-byte input report, one 41-byte output report, report id 0

Claimable from a browser with WebHID: the collection is vendor-defined, so
Chrome does not treat it as a protected device.

The reader answers the USB product string request with a buffer it never
fills, so the name is uninitialised memory - different garbage on every
enumeration, not a fixed encoding bug. It shows up as mojibake in the device
picker and there is nothing a page can do about it. Filtering on the vendor id
at least leaves it as the only entry in the list. Tested working on every port
tried, including a monitor hub.

## Frame format

Both directions, padded with zeros to fill the 41-byte report, 0x04 last.

    offset  0     1     2        3       4 .. len-2   len-1
            0x02  len   address  opcode  payload      checksum

    len       counts STX through checksum inclusive (5 + payload length)
    checksum  bitwise NOT of the sum of every byte before it, mod 256

## Obfuscation

If the address has its high bit set, the opcode and payload are XORed with the
repeating key `Elephant` (45 6C 65 70 68 61 6E 74), starting at key index
`address mod 8`. Addresses without the high bit are plaintext.

    body[i] ^= KEY[(address + i) mod 8]     i counted from the opcode

Only the low three bits of the address matter, so any address 0x80-0xFF works.
Zero payload bytes XOR to the key itself, which is why "Elephant" appears as
readable text in raw frames.

One exception: a NAK is returned unobfuscated even when the address asks for
obfuscation. No masked reply can be mistaken for one, so read opcode 0x13
straight off the wire before unmasking.

## Reply types

    0x10  ack, answer in the payload
    0x12  no such command (obfuscated dialect)
    0x13  not understood, echoes our bytes back (always plaintext)

## Startup handshake

The reader refuses reads until this runs. It is what Net2 sends once, before
any polling, and a replug resets the reader back to needing it.

    addr  op    payload   wire bytes
    0x08  0x25            02 05 08 25 CB      ping, plaintext address
    0x88  0x14            02 05 88 51 1F      open session
    0xCD  0x28            02 05 CD 49 E2
    0x88  0x24  0x0A      02 06 88 61 66 A8
    0x88  0x00            02 05 88 45 2B

## Reading a token

Two messages per read. The prime is required; the reader stays idle without it.

    OUT  02 06 88 61 66 A8   opcode 0x24, payload 0x0A   prime
    IN   02 06 88 55 6C AE   ack, payload 0x00
    OUT  02 05 88 92 DE      opcode 0xD7                 read
    IN   02 25 88 55 ...     ack, 32-byte payload

Unmask the read reply and the payload is the token followed by zero padding.
An all-zero payload means no card. Tell the two acks apart by payload length:
the prime answers with one byte, a read with 32.

    0225 a5 71 35090555 657068616e74456c...   ->  token 5B7D4039

## Command map

Opcodes as decoded, at any address with the high bit set.

    0xD7  read token        ack, token then zero padding
    0x24  prime a read      arg 0x0A, ack with a zero byte
    0x64  get version       ack, "USB PES V1.14"
    0x14  open session      part of the handshake
    0x25  status ping       works in both dialects
    0x28, 0x00              part of the handshake, purpose unknown
    most others             0x12, no such command

Something around opcode 0x26-0x29 sent to a plaintext address makes the reader
flash red and re-enumerate on the USB bus.

## Still unknown

Nothing needed for reading tokens. Not investigated: what 0x28 and 0x00 do,
the longer status structures (0x61 returns 01 00 00 FF FF FF 08 ... 05 90 04),
whether the address means anything beyond picking the key phase, and whether
write or configuration commands exist.
