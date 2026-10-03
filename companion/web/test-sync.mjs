// Sync engine tests against the device protocol code compiled for the host.
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { Link } from './ipodlink.js';
import * as sync from './sync.js';

function harness(bin, root) {
  const p = spawn(bin, ['1', root], { stdio: ['pipe', 'pipe', 'inherit'] });
  const queue = [];
  let wake = null, ended = false;
  p.stdout.on('data', (d) => { queue.push(new Uint8Array(d)); if (wake) { wake(); wake = null; } });
  p.stdout.on('end', () => { ended = true; if (wake) { wake(); wake = null; } });
  return {
    proc: p,
    write: (data) => new Promise((res, rej) => p.stdin.write(Buffer.from(data), (e) => (e ? rej(e) : res()))),
    async read() {
      while (!queue.length) { if (ended) return null; await new Promise((r) => (wake = r)); }
      return queue.shift();
    },
  };
}

// Stands in for a File System Access handle.
const fileHandle = (bytes) => ({ getFile: async () => ({ arrayBuffer: async () => bytes.buffer }) });
const hostFile = (rel, bytes) => ({
  relPath: rel,
  devicePath: sync.devicePath(rel),
  size: bytes.length,
  handle: fileHandle(bytes),
});

const bin = process.argv[2] || '../../device/link_host';
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ipodos-sync-'));
const t = harness(bin, root);
const link = new Link(t);
const results = [];
const step = async (name, fn) => { await fn(); results.push(name); };

const bytes = (n, seed = 1) => new Uint8Array(n).map((_, i) => (i * seed + 11) & 0xff);
const build = async () => ({
  db: new TextEncoder().encode('IPDB-fake-database'),
  art: new TextEncoder().encode('IPAP-fake-artwork'),
  fonts: { 'row-14.ipfn': new TextEncoder().encode('IPFN-fake') },
});

await step('path sanitizing', () => {
  assert.equal(sync.sanitize('AC/DC'), 'AC_DC');
  assert.equal(sync.sanitize('why?'), 'why_');
  assert.equal(sync.sanitize('trailing. '), 'trailing');
  assert.equal(sync.sanitize(''), '_');
  assert.equal(sync.devicePath('A/B/c.flac'), '/Music/A/B/c.flac');
});

await step('plan: first run uploads everything', () => {
  const host = [hostFile('a/one.flac', bytes(10)), hostFile('b/two.mp3', bytes(20))];
  const p = sync.plan(host, new Map());
  assert.equal(p.upload.length, 2);
  assert.equal(p.remove.length, 0);
  assert.equal(p.bytes, 30);
});

await step('plan: unchanged files are skipped, stale removed, resized re-sent', () => {
  const host = [hostFile('a/one.flac', bytes(10)), hostFile('b/two.mp3', bytes(20))];
  const device = new Map([
    ['/Music/a/one.flac', 10],   // unchanged
    ['/Music/b/two.mp3', 999],   // different size
    ['/Music/old/gone.flac', 5], // no longer in the library
  ]);
  const p = sync.plan(host, device);
  assert.deepEqual(p.upload.map((f) => f.relPath), ['b/two.mp3']);
  assert.deepEqual(p.remove, ['/Music/old/gone.flac']);
});

await step('full sync writes music, art and database', async () => {
  const host = [hostFile('Kesh/Signal Bloom/01 Glass Weather.flac', bytes(5000, 3))];
  const phases = [];
  const r = await sync.run(link, host, build, (e) => phases.push(e.phase));
  assert.equal(r.uploaded, 1);
  assert.ok(phases.includes('copy') && phases.includes('build') && phases.includes('done'));

  const track = fs.readFileSync(path.join(root, 'Music/Kesh/Signal Bloom/01 Glass Weather.flac'));
  assert.equal(track.length, 5000);
  assert.equal(fs.readFileSync(path.join(root, '.ipodos/library.ipdb'), 'utf8'), 'IPDB-fake-database');
  assert.equal(fs.readFileSync(path.join(root, '.ipodos/artwork.ipap'), 'utf8'), 'IPAP-fake-artwork');
  assert.ok(fs.existsSync(path.join(root, '.ipodos/fonts/row-14.ipfn')));
});

await step('second sync transfers nothing', async () => {
  const host = [hostFile('Kesh/Signal Bloom/01 Glass Weather.flac', bytes(5000, 3))];
  const r = await sync.run(link, host, build, () => {});
  assert.equal(r.uploaded, 0);
  assert.equal(r.removed, 0);
});

await step('removing a track from the library deletes it from the device', async () => {
  const r = await sync.run(link, [], build, () => {});
  assert.equal(r.removed, 1);
  assert.ok(!fs.existsSync(path.join(root, 'Music/Kesh/Signal Bloom/01 Glass Weather.flac')));
});

await step('names unsafe for FAT are rewritten', async () => {
  const host = [hostFile('AC/DC: Live?/01 Back*Black.flac', bytes(100))];
  await sync.run(link, host, build, () => {});
  assert.ok(fs.existsSync(path.join(root, 'Music/AC/DC_ Live_/01 Back_Black.flac')));
});

await step('refuses to start when the device is too small', async () => {
  const huge = [{ relPath: 'big.flac', devicePath: '/Music/big.flac', size: 2 ** 48, handle: fileHandle(new Uint8Array(0)) }];
  await assert.rejects(sync.run(link, huge, build, () => {}), /not enough space/);
});

await step('stale verification stamp is cleared', async () => {
  fs.writeFileSync(path.join(root, '.ipodos/.verified'), 'x');
  await sync.run(link, [], build, () => {});
  assert.ok(!fs.existsSync(path.join(root, '.ipodos/.verified')));
});

await step('journal pull and clear', async () => {
  fs.writeFileSync(path.join(root, '.ipodos/journal.bin'), Buffer.alloc(32, 7));
  const data = await sync.pullJournal(link);
  assert.equal(data.length, 32);
  assert.ok(!fs.existsSync(path.join(root, '.ipodos/journal.bin')));
  assert.equal(await sync.pullJournal(link), null);
});

t.proc.stdin.end();
await new Promise((r) => t.proc.on('exit', r));
fs.rmSync(root, { recursive: true, force: true });
console.log(`sync engine: ${results.length} checks passed`);
