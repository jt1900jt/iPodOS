# Link protocol v1 (draft)

Framed binary protocol between the companion and the device over USB CDC-ACM. Version 1
covers the transport test (hello, echo, sink, source). Sync operations such as file transfer
and journal pull are added in later versions using the same framing.

Device side: `device/link/link_proto.{c,h}`, vendored into the firmware as
`apps/shell/link_proto.{c,h}`. Client: `companion/web/ipodlink.js`.

## USB

The firmware defaults USB to charge mode, so the iPod stays in the shell when plugged in.
Holding any button while connecting inverts this and gives disk mode. On the device, open
**USB LINK** from Home to run the link.

Two transports are exposed, and the device uses whichever the host opened:

| | Vendor bulk (WebUSB) | CDC-ACM (Web Serial) |
|---|---|---|
| Host API | WebUSB | Web Serial |
| Interface | vendor class `0xFF`, subclass `0x49`, protocol `0x50` | CDC class |
| Host to device | full speed | throttled |
| Use | sync and bulk transfer | logging, terminal debugging |

CDC-ACM is claimed by the host's serial tty layer, which fragments the browser's bulk writes
into ~100-200 byte USB transfers whatever chunk size the host asks for. Measured on a 7G:
12.2 MB/s device to host, but only 0.5-1.5 MB/s host to device. The vendor interface has no
tty layer in between, so transfers reach the endpoint whole.

The device advertises a BOS descriptor with two platform capabilities: WebUSB, so Chrome
offers the device without a host driver, and Microsoft OS 2.0, so Windows binds WinUSB to the
vendor interface automatically. Both are fetched with vendor request `0x21`; the MS OS 2.0
descriptor set is at `wIndex` 7. Linux and macOS need no driver.

## Framing

Every frame is a 16-byte header followed by `length` payload bytes. All fields little-endian.

| Off | Type | Field |
|---|---|---|
| 0 | u8[2] | magic `I` `P` |
| 2 | u8 | type |
| 3 | u8 | flags (bit 0: payload CRC present) |
| 4 | u32 | seq (host-chosen; replies echo the request's seq, except DATA) |
| 8 | u32 | length |
| 12 | u32 | CRC-32 (IEEE) of the payload when flag bit 0 is set, else 0 |

A receiver that sees a bad magic drops one byte and retries, so it resynchronizes after
garbage. A CRC mismatch on a request produces an ERROR reply instead of the normal reply.

## Frame types

| Type | Name | Direction | Payload |
|---|---|---|---|
| 0x01 | HELLO | host → device | none |
| 0x81 | HELLO_R | device → host | ASCII `ipodos-link/1 <device description>` |
| 0x02 | SINK | host → device | any bytes, discarded; any length up to 4 GiB |
| 0x82 | SINK_ACK | device → host | u32 bytes received, u32 CRC ok (1/0) |
| 0x03 | SOURCE | host → device | u32 total bytes, u32 chunk size (0 or >16384 means 16384), u32 flags (bit 0: CRC each chunk) |
| 0x83 | DATA | device → host | one chunk of a fixed test pattern; seq counts from 0 |
| 0x85 | SOURCE_DONE | device → host | u32 total bytes sent |
| 0x04 | ECHO | host → device | up to 65536 bytes |
| 0x84 | ECHO_R | device → host | the same bytes, with CRC if the request had one |
| 0xFF | ERROR | device → host | ASCII message |

### File operations (sync)

| Type | Name | Payload |
|---|---|---|
| 0x10 | STAT | path → STAT_R: `u8 kind` (0 missing, 1 file, 2 dir), `u32 size`, `u32 mtime` |
| 0x11 | LIST | path → zero or more LIST_R frames, then OK |
| 0x12 | MKDIR | path → OK; an existing directory is success |
| 0x13 | PUT_BEGIN | `u32 size`, `u32 crc`, `u8 flags`, path → OK |
| 0x14 | PUT_DATA | file bytes, up to 16384 per frame; no reply |
| 0x15 | PUT_END | none → OK with `u32 bytes written`, or ERROR |
| 0x16 | GET | path → DATA frames, then GET_DONE with `u32 bytes` |
| 0x17 | DELETE | path → OK |
| 0x18 | FREE | none → FREE_R: `u64 free`, `u64 total` |
| 0x19 | SYNC_DONE | none → OK; the shell reloads its library |

A LIST_R payload packs entries back to back: `u8 kind`, `u32 size`, `u32 mtime`,
`u16 name_len`, name. Entries are split across frames as needed, so a directory with
thousands of files needs no special handling.

Writes are staged: PUT_BEGIN opens a temporary file, PUT_DATA appends to it, and
PUT_END renames it over the target only when the byte count and CRC both match. An
interrupted sync therefore never leaves a half-written file under its real name, and
missing parent directories are created automatically.

Paths must be absolute and may not contain `..`; the device rejects anything else.

## Tests

`device/tools/link_host` runs the device-side code over stdin/stdout.
`companion/web/test-link.mjs` drives it with the same client the browser uses, covering
fragmentation, resync, CRC failures, size limits and odd chunk sizes. `make test` in
`device/` runs it.

## Browser test page

`companion/web/index.html` measures latency and throughput over Web Serial. Serve the folder
from localhost (Web Serial needs a secure context) and open it in desktop Chrome or Edge.

Latency is timed across a batch of round trips: browsers round `performance.now()` to 1 ms
or coarser, so a single sub-millisecond round trip measures as zero.

## Device receive path

Both transports share the same shape. The OUT endpoint uses two buffers. When a transfer completes, the next one is armed into the
other buffer before the finished buffer is copied into the receive ring, so the endpoint is
never idle while the USB thread works. The USB LINK screen reports the share of time the
endpoint had a transfer armed; anything well below 100% means the device is the bottleneck.
