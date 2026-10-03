// iPod OS link protocol client (docs/link-protocol.md). Runs in the browser over Web Serial
// and in Node for tests. Transport: { write(Uint8Array): Promise, read(): Promise<Uint8Array|null> }.

export const T = {
  HELLO: 0x01, SINK: 0x02, SOURCE: 0x03, ECHO: 0x04,
  HELLO_R: 0x81, SINK_ACK: 0x82, DATA: 0x83, ECHO_R: 0x84, SOURCE_DONE: 0x85, ERROR: 0xff,
};
export const F_CRC = 0x01;
const HDR = 16;

const TABLE = (() => {
  const t = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[i] = c >>> 0;
  }
  return t;
})();

// CRC-32 (IEEE); chains: crc32(b, crc32(a)) === crc32(a||b)
export function crc32(data, crc = 0) {
  let c = ~crc >>> 0;
  for (let i = 0; i < data.length; i++) c = TABLE[(c ^ data[i]) & 0xff] ^ (c >>> 8);
  return ~c >>> 0;
}

export function header(type, flags, seq, len, crc) {
  const h = new Uint8Array(HDR);
  const v = new DataView(h.buffer);
  h[0] = 0x49; h[1] = 0x50; h[2] = type; h[3] = flags;
  v.setUint32(4, seq >>> 0, true);
  v.setUint32(8, len >>> 0, true);
  v.setUint32(12, crc >>> 0, true);
  return h;
}

export class LinkError extends Error {}

export class Link {
  constructor(transport) {
    this.t = transport;
    this.chunks = [];
    this.have = 0;
    this.seq = 1;
    this.closed = false;
  }

  async fill(n) {
    while (this.have < n) {
      const c = await this.t.read();
      if (!c) { this.closed = true; throw new LinkError('link closed'); }
      if (c.length) { this.chunks.push(c); this.have += c.length; }
    }
  }

  take(n) {
    const out = new Uint8Array(n);
    let off = 0;
    while (off < n) {
      const c = this.chunks[0];
      const k = Math.min(c.length, n - off);
      out.set(c.subarray(0, k), off);
      off += k;
      if (k === c.length) this.chunks.shift(); else this.chunks[0] = c.subarray(k);
    }
    this.have -= n;
    return out;
  }

  async frame() {
    for (;;) {
      await this.fill(HDR);
      const h = this.take(HDR);
      if (h[0] !== 0x49 || h[1] !== 0x50) {
        // resync: push back all but the first byte
        this.chunks.unshift(h.subarray(1));
        this.have += HDR - 1;
        continue;
      }
      const v = new DataView(h.buffer);
      const f = { type: h[2], flags: h[3], seq: v.getUint32(4, true), len: v.getUint32(8, true), crc: v.getUint32(12, true) };
      await this.fill(f.len);
      f.payload = this.take(f.len);
      f.crcOk = !(f.flags & F_CRC) || crc32(f.payload) === f.crc;
      return f;
    }
  }

  async expect(type) {
    const f = await this.frame();
    if (f.type === T.ERROR) throw new LinkError('device error: ' + new TextDecoder().decode(f.payload));
    if (f.type !== type) throw new LinkError(`expected frame 0x${type.toString(16)}, got 0x${f.type.toString(16)}`);
    return f;
  }

  async send(type, payload = new Uint8Array(0), crc = false) {
    const seq = this.seq++;
    await this.t.write(header(type, crc ? F_CRC : 0, seq, payload.length, crc ? crc32(payload) : 0));
    if (payload.length) await this.t.write(payload);
    return seq;
  }

  async hello() {
    await this.send(T.HELLO);
    const f = await this.expect(T.HELLO_R);
    return new TextDecoder().decode(f.payload);
  }

  async echo(payload, crc = true) {
    const t0 = performance.now();
    await this.send(T.ECHO, payload, crc);
    const f = await this.expect(T.ECHO_R);
    const ms = performance.now() - t0;
    if (!f.crcOk) throw new LinkError('echo reply crc mismatch');
    if (f.payload.length !== payload.length || f.payload.some((b, i) => b !== payload[i]))
      throw new LinkError('echo payload mismatch');
    return ms;
  }

  // Host -> device. Returns { seconds, bytes, ok }.
  async sink(total, { crc = true, chunk = 65536, corrupt = false } = {}) {
    const block = new Uint8Array(chunk);
    for (let i = 0; i < chunk; i++) block[i] = (i * 13 + 5) & 0xff;
    let c = 0;
    if (crc) for (let sent = 0; sent < total; sent += chunk) c = crc32(block.subarray(0, Math.min(chunk, total - sent)), c);
    if (corrupt) c ^= 1;
    const t0 = performance.now();
    await this.t.write(header(T.SINK, crc ? F_CRC : 0, this.seq++, total, c));
    for (let sent = 0; sent < total; sent += chunk) await this.t.write(block.subarray(0, Math.min(chunk, total - sent)));
    const f = await this.expect(T.SINK_ACK);
    const v = new DataView(f.payload.buffer, f.payload.byteOffset);
    return { seconds: (performance.now() - t0) / 1000, bytes: v.getUint32(0, true), ok: v.getUint32(4, true) === 1 };
  }

  // Device -> host. Returns { seconds, bytes, frames, crcErrors }.
  async source(total, { crc = true, chunk = 16384 } = {}) {
    const req = new Uint8Array(12);
    const v = new DataView(req.buffer);
    v.setUint32(0, total, true); v.setUint32(4, chunk, true); v.setUint32(8, crc ? F_CRC : 0, true);
    const t0 = performance.now();
    await this.send(T.SOURCE, req);
    let bytes = 0, frames = 0, crcErrors = 0;
    for (;;) {
      const f = await this.frame();
      if (f.type === T.DATA) {
        bytes += f.len; frames++;
        if (!f.crcOk) crcErrors++;
      } else if (f.type === T.SOURCE_DONE) {
        break;
      } else if (f.type === T.ERROR) {
        throw new LinkError('device error: ' + new TextDecoder().decode(f.payload));
      }
    }
    return { seconds: (performance.now() - t0) / 1000, bytes, frames, crcErrors };
  }
}

// --- transports ---

// Web Serial (CDC-ACM). Works everywhere the port enumerates, but the host tty layer
// fragments writes into small USB transfers, so host-to-device throughput is poor.
// Baud rate is ignored by CDC-ACM but required by the API.
export async function openSerial(port) {
  await port.open({ baudRate: 115200, bufferSize: 1 << 20 });
  const reader = port.readable.getReader();
  const writer = port.writable.getWriter();
  return {
    kind: 'serial',
    async write(data) { await writer.write(data); },
    async read() { const { value, done } = await reader.read(); return done ? null : value; },
    async close() { reader.releaseLock(); writer.releaseLock(); await port.close(); },
  };
}

export const USB_FILTERS = [{ vendorId: 0x05ac, classCode: 0xff, subclassCode: 0x49, protocolCode: 0x50 }];

// WebUSB on the device's vendor-specific interface: bulk transfers go straight to the
// endpoint with no tty layer in between.
export async function openUsb(device, { readSize = 1 << 18 } = {}) {
  await device.open();
  if (!device.configuration) await device.selectConfiguration(1);

  let iface = null, epIn = 0, epOut = 0;
  for (const i of device.configuration.interfaces) {
    const a = i.alternate;
    if (a.interfaceClass === 0xff && a.interfaceSubclass === 0x49 && a.interfaceProtocol === 0x50) {
      iface = i;
      for (const ep of a.endpoints) {
        if (ep.type !== 'bulk') continue;
        if (ep.direction === 'in') epIn = ep.endpointNumber; else epOut = ep.endpointNumber;
      }
    }
  }
  if (!iface || !epIn || !epOut) throw new Error('no iPod OS bulk interface on this device');
  await device.claimInterface(iface.interfaceNumber);

  let closed = false;
  return {
    kind: 'webusb',
    async write(data) {
      const r = await device.transferOut(epOut, data);
      if (r.status !== 'ok') throw new LinkError('transferOut ' + r.status);
      if (r.bytesWritten !== data.length) throw new LinkError('short transferOut');
    },
    async read() {
      if (closed) return null;
      const r = await device.transferIn(epIn, readSize);
      if (r.status === 'stall') { await device.clearHalt('in', epIn); return new Uint8Array(0); }
      if (r.status !== 'ok') return null;
      return new Uint8Array(r.data.buffer, r.data.byteOffset, r.data.byteLength);
    },
    async close() {
      closed = true;
      try { await device.releaseInterface(iface.interfaceNumber); } catch {}
      await device.close();
    },
  };
}
