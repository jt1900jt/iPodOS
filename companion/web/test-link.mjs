// Runs the browser client against the device-side protocol code compiled for the host
// (device/tools/link_host). Usage: node test-link.mjs path/to/link_host
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import { Link, T, header, LinkError } from './ipodlink.js';

function harness(bin, seed) {
  const p = spawn(bin, [String(seed)], { stdio: ['pipe', 'pipe', 'inherit'] });
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

const bin = process.argv[2] || '../../device/link_host';
const MB = 1 << 20;
const t = harness(bin, 42);
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

t.proc.stdin.end();
await new Promise((r) => t.proc.on('exit', r));
console.log(`link protocol: ${results.length} checks passed`);
