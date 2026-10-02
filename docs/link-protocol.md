# Link protocol v1 (draft)

Framed binary protocol between the companion and the device over USB CDC-ACM. Version 1
covers the transport test (hello, echo, sink, source). Sync operations such as file transfer
and journal pull are added in later versions using the same framing.

Device side: `device/link/link_proto.{c,h}`, vendored into the firmware as
`apps/shell/link_proto.{c,h}`. Client: `companion/web/ipodlink.js`.

## USB

The firmware enables the CDC-ACM class and defaults USB to charge mode. When the iPod is
plugged in it stays in the shell, and the host sees a serial port with VID `0x05AC`. Holding
any button while plugging in inverts this and gives disk mode. On the device, open
**USB LINK** from Home to run the link.

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

## Tests

`device/tools/link_host` runs the device-side code over stdin/stdout.
`companion/web/test-link.mjs` drives it with the same client the browser uses, covering
fragmentation, resync, CRC failures, size limits and odd chunk sizes. `make test` in
`device/` runs it.

## Browser test page

`companion/web/index.html` measures latency and throughput over Web Serial. Serve the folder
from localhost (Web Serial needs a secure context) and open it in desktop Chrome or Edge.
