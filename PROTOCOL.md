# Paxton Net2 desktop reader - USB protocol

Reverse engineered from the device plus one USBPcap capture of the Net2
software. Everything here is verified against real traffic, and the tests
check the key parts against captured bytes. Opcode names and the token number
formulas come from Net2 itself: `Paxton.Net2.DesktopReaderSrv.dll` is .NET,
and its BOARD_CMD enum and Derive*TokenNo methods decompile readably.

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

It has no USB serial number either (Windows gives it a generated instance id,
such as `9&17FE9C48&0&4`). Chrome only keeps WebHID permission across a replug
for devices that have one, so after unplugging, the page gets no `connect`
event and has to ask through the picker again.

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
obfuscation, so read opcode 0x13 straight off the wire before unmasking.

That makes the encoding ambiguous in principle: an opcode that masks to the
wire byte 0x13 would be read back as a plaintext NAK. It cannot happen in
practice, because reaching 0x13 needs a key byte equal to `opcode XOR 0x13`,
and no opcode in real traffic - neither the commands sent nor the reply types
0x10, 0x12 and 0x13 - produces one of the eight bytes of the key. A property
test found this, so it is written down rather than left as a surprise.

## Reply types

    0x10  ack, answer in the payload
    0x12  no such command (obfuscated dialect)
    0x13  not understood, echoes our bytes back (always plaintext)

## Startup handshake

The reader refuses reads until this runs. It is what Net2 sends once, before
any polling, and a replug resets the reader back to needing it.

    addr  op    payload   wire bytes
    0x08  0x25            02 05 08 25 CB      RWD_OPEN_LINK, plaintext address
    0x88  0x14            02 05 88 51 1F      TOKEN_R_DATA (a Hitag2 read)
    0xCD  0x28            02 05 CD 49 E2      RWD_SERIAL_NUMBER
    0x88  0x24  0x0A      02 06 88 61 66 A8   RWD_LEDS
    0x88  0x00            02 05 88 45 2B      unknown

## Reading a token

Two messages per read: set the LEDs, then read one kind of token. Net2 does
this every poll and the reader stays idle without the first one.

    OUT  02 06 88 61 66 A8   RWD_LEDS 0x0A
    IN   02 06 88 55 6C AE   ack, payload 0x00
    OUT  02 05 88 92 DE      RWD_READ_MIFARE
    IN   02 25 88 55 ...     ack, 32-byte payload

Tell the two acks apart by payload length: the LED command answers with one
byte, a read with 32. Net2 sets the LEDs to 0x06 instead after a token is
found. Each read command asks for one card technology, and Net2 cycles through
five of them, moving on when a read does not come back as an ack:

    0x14  TOKEN_R_DATA       Hitag2 pages (Paxton's own fobs)
    0xC7  RWD_READ_EM4100
    0xD7  RWD_READ_MIFARE
    0xA8  RWD_READ_HITAG_1
    0xD8  RWD_READ_HID

Tusk reads Mifare and, in beta, Hitag2.

### Mifare

With a card present the reply is an ack whose payload is the UID followed by
zero padding. With no card it is not an ack at all: it is type `0x12` with the
single byte `01`. (A Hitag2 read with no fob gets `0x12 28`.) This document
used to say an all-zero ack meant no card; net2.pcap and the live reader both
show `0x12 01`, in 47 of 47 empty reads.

Replies are in order but slow. An empty Mifare read is answered after the next
read has already been sent, so a reply must be credited to the oldest read still
waiting, not the latest one. Crediting the latest kept a lifted card on screen
for ever. `0x12 01` and `0x12 28` name their read, which also resyncs the queue.
Net2's token number is the first four bytes as a big-endian integer, keeping
the last eight decimal digits:

    0225 a5 71 35090555 657068616e74456c...   ->  UID 5B7D4039
    0x5B7D4039 = 1534935097  ->  mod 10^8  ->  34935097, as Net2 shows it

### Hitag2 (beta)

Implemented from Net2's decoder but not yet tested on a real fob, so the reply
layout is inferred from that code rather than observed. The payload holds
pages 2 to 7, four bytes each, big-endian, so page n starts at byte 4(n-2).

Digits are 5-bit codes, most significant bit first. The table is Net2's; it is
mostly an odd-parity bit plus the value, except 14:

    0 10000   4 00100   8 01000   12 11100
    1 00001   5 10101   9 11001   13 01101  end
    2 00010   6 10110  10 11010   14 11110
    3 10011   7 00111  11 01011   15 11111  end

Bits 26-29 of page 7 pick the layout:

- `0100`, Net2: digits from bit 0 of pages 4+5 as one 64-bit string, six from
  page 4, then from bit 32 (page 5) on, until an end code. A leading run of
  `00000` counts as zeros. Keep the last eight digits.
- `0001`, newer: three digits at (page, bit) (6,25) (7,5) (7,15) give a card
  type. Type 1, a user card, has its number at (5,10) (5,20) (6,0) (6,10)
  (4,5) (4,15) (4,25) (5,5). Any other type uses the Net2 layout over only the
  first 45 bits.

Net2 also rewrites some old-layout fobs when it sees them (RevertTokenToModeOne).
Tusk never writes to a token, so that is left out.

## Command map

Opcodes as decoded, at any address with the high bit set, named from Net2's
BOARD_CMD enum where it has one.

    0x14  TOKEN_R_DATA           Hitag2 pages
    0x15  TOKEN_R_SERIAL_NO
    0x16  TOKEN_W_CONFIGPSWTAG
    0x17  TOKEN_W_PAGES
    0x18  TOKEN_W_PASSWORDRWD
    0x1E  RWD_BEEP
    0x21  RWD_FIRMWARE_VERSION
    0x24  RWD_LEDS               arg 0x0A idle, 0x06 token found
    0x25  RWD_OPEN_LINK          works in both dialects
    0x26  RWD_RESET
    0x28  RWD_SERIAL_NUMBER
    0x64  -                      ack, "USB PES V1.14"
    0xA8  RWD_READ_HITAG_1
    0xC7  RWD_READ_EM4100
    0xC8  RWD_TURN_OFF_ON_FIELD
    0xD7  RWD_READ_MIFARE        ack, UID then zero padding; no card: 0x12 01
    0xD8  RWD_READ_HID
    most others                  0x12, no such command

The enum also has firmware, ASIC and settings commands (0x0A-0x0D, 0x1F-0x2D)
that Tusk has no reason to touch. The TOKEN_W_ commands write to tokens.

Sweeping opcodes 0x26-0x29 at a plaintext address made the reader flash red
and re-enumerate on the USB bus; 0x26 being RWD_RESET explains it.

## Still unknown

Whether the Hitag2 reply really has the layout Net2's code implies - it needs
a Paxton fob. Not investigated: what 0x00 does, the longer status structures
(0x61 returns 01 00 00 FF FF FF 08 ... 05 90 04), and whether the address
means anything beyond picking the key phase.
