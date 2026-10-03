// Runs the browser client against the device-side protocol code compiled for the host
// (device/tools/link_host). Usage: node test-link.mjs path/to/link_host
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import { Link, T, header, LinkError } from './ipodlink.js';

function harness(bin, seed, root) {
  const args = root ? [String(seed), root] : [String(seed)];
  const p = spawn(bin, args, { stdio: ['pipe', 'pipe', 'inherit'] });
  const queue = [];
  let wake = null, ended = false;
  p.stdout.on('data', (d) => { queue.push(new Uint8Array(d)); if (wake) { wake(); wake = null; } });
  p.stdout.on('end', () => { ended = true; if (wake) { wake(); wake = null; } });
  return {
    proc: p,
    write: (data) => new Promise((res, rej) => p.stdin.write(Buffer.from(data), (e) => (e ? rej(e) : res()))),
    async read() {
      while (!queue.length) {
        if (ended) return null;
        await new Promise((r) => (wake = r));
      }
      return queue.shift();
    },
  };
}

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const bin = process.argv[2] || '../../device/link_host';
const MB = 1 << 20;
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ipodos-link-'));
const t = harness(bin, 42, root);
const link = new Link(t);
const results = [];
const step = async (name, fn) => { await fn(); results.push(name); };

await step('hello', async () => {
  assert.match(await link.hello(), /^ipodos-link\/1 host-harness$/);
});

await step('echo x50 with crc', async () => {
  for (let i = 0; i < 50; i++) {
    const p = new Uint8Array(1 + ((i * 977) % 4000)).map((_, k) => (k * 7 + i) & 0xff);
    await link.echo(p);
  }
});

await step('echo max size', async () => { await link.echo(new Uint8Array(65536).fill(0xab)); });

await step('echo too large -> error', async () => {
  await assert.rejects(link.echo(new Uint8Array(65537)), LinkError);
});

await step('resync after garbage', async () => {
  await t.write(new Uint8Array([1, 2, 3, 0x49, 9, 9, 0x50]));
  assert.match(await link.hello(), /host-harness/);
});

await step('unknown type -> error', async () => {
  await link.send(0x42, new Uint8Array(10));
  await assert.rejects(link.expect(T.HELLO_R), /unknown frame type/);
});

await step('sink 8 MB crc ok', async () => {
  const r = await link.sink(8 * MB);
  assert.equal(r.bytes, 8 * MB);
  assert.equal(r.ok, true);
});

await step('sink corrupt crc detected', async () => {
  const r = await link.sink(100_001, { corrupt: true, chunk: 3000 });
  assert.equal(r.ok, false);
});

await step('sink without crc', async () => {
  const r = await link.sink(2 * MB, { crc: false });
  assert.equal(r.ok, true);
});

await step('source 8 MB crc', async () => {
  const r = await link.source(8 * MB);
  assert.equal(r.bytes, 8 * MB);
  assert.equal(r.crcErrors, 0);
  assert.equal(r.frames, 512);
});

await step('source odd sizes', async () => {
  const r = await link.source(100_001, { chunk: 3000 });
  assert.equal(r.bytes, 100_001);
  assert.equal(r.frames, 34);
  assert.equal(r.crcErrors, 0);
});

await step('bad source request -> error', async () => {
  await link.send(T.SOURCE, new Uint8Array(5));
  await assert.rejects(link.expect(T.SOURCE_DONE), /bad source request/);
});

await step('payload crc mismatch -> error', async () => {
  await t.write(header(T.ECHO, 1, 99, 4, 0xdeadbeef));
  await t.write(new Uint8Array([1, 2, 3, 4]));
  await assert.rejects(link.expect(T.ECHO_R), /crc mismatch/);
});

// --- file operations ---

await step('mkdir and stat', async () => {
  await link.mkdir('/Music');
  const st = await link.stat('/Music');
  assert.equal(st.kind, 2);
  assert.equal((await link.stat('/nope')).kind, 0);
});

await step('put, stat, get round trip', async () => {
  const data = new Uint8Array(100_000).map((_, i) => (i * 31 + 7) & 0xff);
  await link.putFile('/Music/Artist/Album/track.flac', data);
  const st = await link.stat('/Music/Artist/Album/track.flac');
  assert.equal(st.kind, 1);
  assert.equal(st.size, data.length);
  const back = await link.getFile('/Music/Artist/Album/track.flac');
  assert.equal(back.length, data.length);
  assert.deepEqual(back, data);
});

await step('put creates missing parents', async () => {
  await link.putFile('/a/b/c/d.txt', new TextEncoder().encode('hi'));
  assert.equal((await link.stat('/a/b/c/d.txt')).size, 2);
});

await step('empty file', async () => {
  await link.putFile('/empty.bin', new Uint8Array(0));
  assert.equal((await link.stat('/empty.bin')).size, 0);
  assert.equal((await link.getFile('/empty.bin')).length, 0);
});

await step('bad crc leaves no file behind', async () => {
  const data = new Uint8Array(5000).fill(3);
  const head = new Uint8Array(9 + new TextEncoder().encode('/bad.bin').length);
  const v = new DataView(head.buffer);
  v.setUint32(0, data.length, true);
  v.setUint32(4, 0xdeadbeef, true); // wrong on purpose
  head[8] = 1;
  head.set(new TextEncoder().encode('/bad.bin'), 9);
  await link.send(T.PUT_BEGIN, head);
  await link.expect(T.OK);
  await link.send(T.PUT_DATA, data);
  await link.send(T.PUT_END);
  await assert.rejects(link.expect(T.OK), /transfer failed/);
  assert.equal((await link.stat('/bad.bin')).kind, 0, 'partial file must not be committed');
});

await step('list', async () => {
  const entries = await link.list('/Music/Artist/Album');
  assert.equal(entries.length, 1);
  assert.equal(entries[0].name, 'track.flac');
  assert.equal(entries[0].kind, 1);
  assert.equal(entries[0].size, 100_000);
});

await step('list many entries spans frames', async () => {
  await link.mkdir('/many');
  for (let i = 0; i < 300; i++) await link.putFile(`/many/file-${i}.txt`, new Uint8Array(4));
  const entries = await link.list('/many');
  assert.equal(entries.length, 300);
  assert.equal(new Set(entries.map((e) => e.name)).size, 300);
});

await step('delete', async () => {
  await link.remove('/empty.bin');
  assert.equal((await link.stat('/empty.bin')).kind, 0);
});

await step('path traversal refused', async () => {
  await assert.rejects(link.stat('/../etc/passwd'), /bad path/);
  await assert.rejects(link.mkdir('/..'), /bad path/);
  await assert.rejects(link.stat('/Music/../../etc'), /bad path/);
  await assert.rejects(link.stat('relative/path'), /bad path/);
});

await step('dots inside names are allowed', async () => {
  // Real filenames contain runs of dots; only a whole ".." component escapes the root.
  const p = '/Music/A Boogie Wit da Hoodie/Artist 2.0/13. R.O.D..m4a';
  await link.putFile(p, new Uint8Array(64).fill(9));
  assert.equal((await link.stat(p)).size, 64);
  const entries = await link.list('/Music/A Boogie Wit da Hoodie/Artist 2.0');
  assert.ok(entries.some((e) => e.name === '13. R.O.D..m4a'));
});

await step('free space', async () => {
  const f = await link.free();
  assert.ok(f.total > 0 && f.free > 0 && f.free <= f.total);
});

await step('sync done', async () => { await link.syncDone(); });

await step('transport still works after file ops', async () => {
  assert.match(await link.hello(), /host-harness/);
  const r = await link.sink(1 * MB);
  assert.equal(r.ok, true);
});

t.proc.stdin.end();
await new Promise((r) => t.proc.on('exit', r));
fs.rmSync(root, { recursive: true, force: true });
console.log(`link protocol: ${results.length} checks passed`);
