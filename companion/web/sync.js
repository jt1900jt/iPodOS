// Sync engine: compares the host library with what is on the device, transfers the
// difference, then rebuilds the library and art pack.
//
// Order matters. Music files go first, then deletions, then artwork, and the database
// last: the database is what makes tracks visible, so writing it last means an
// interrupted sync leaves the device showing its previous library rather than a
// database that references files which never arrived.

import { LinkError } from './ipodlink.js';

export const DEVICE_MUSIC = '/Music';
export const DEVICE_DIR = '/.ipodos';

const AUDIO_EXT = new Set([
  'mp3', 'm4a', 'm4b', 'mp4', 'aac', 'flac', 'ogg', 'oga', 'opus',
  'wav', 'aif', 'aiff', 'wv', 'ape', 'mpc',
]);

export function isAudio(name) {
  const dot = name.lastIndexOf('.');
  return dot > 0 && AUDIO_EXT.has(name.slice(dot + 1).toLowerCase());
}

// FAT32 rejects these, and a track whose tags contain one would otherwise fail to
// transfer with a confusing device-side error.
export function sanitize(part) {
  return part
    .replace(/[<>:"/\\|?*\x00-\x1f]/g, '_')
    .replace(/[. ]+$/, '')
    .slice(0, 100) || '_';
}

/** Walks a File System Access directory handle, yielding audio files with their paths. */
export async function* walk(dir, prefix = '') {
  for await (const [name, handle] of dir.entries()) {
    if (name.startsWith('.')) continue;
    const path = prefix ? `${prefix}/${name}` : name;
    if (handle.kind === 'directory') {
      yield* walk(handle, path);
    } else if (isAudio(name)) {
      yield { path, handle };
    }
  }
}

/** Every file currently under /Music on the device, as path -> size. */
export async function deviceIndex(link, onProgress) {
  const found = new Map();
  const stack = [DEVICE_MUSIC];
  while (stack.length) {
    const dir = stack.pop();
    let entries;
    try {
      entries = await link.list(dir);
    } catch (e) {
      if (e instanceof LinkError) continue; // missing directory: nothing indexed yet
      throw e;
    }
    for (const e of entries) {
      const full = `${dir}/${e.name}`;
      if (e.kind === 2) stack.push(full);
      else found.set(full, e.size);
    }
    if (onProgress) onProgress(found.size);
  }
  return found;
}

/**
 * Decides what to transfer. A file counts as present when the device has the same path
 * at the same size; sizes come free with the directory listing, while hashing every
 * track would mean reading the whole library back over the link.
 */
export function plan(hostFiles, deviceFiles) {
  const wanted = new Map();
  for (const f of hostFiles) wanted.set(f.devicePath, f);

  const upload = [];
  for (const [path, f] of wanted) {
    const have = deviceFiles.get(path);
    if (have === undefined || have !== f.size) upload.push(f);
  }
  const remove = [];
  for (const path of deviceFiles.keys()) if (!wanted.has(path)) remove.push(path);

  const bytes = upload.reduce((n, f) => n + f.size, 0);
  return { upload, remove, bytes };
}

/** Maps a host-relative path to its device path, sanitizing each component. */
export function devicePath(relPath) {
  const parts = relPath.split('/').map(sanitize);
  return `${DEVICE_MUSIC}/${parts.join('/')}`;
}

export class SyncError extends Error {}

/**
 * Runs a sync. `report` receives { phase, done, total, detail } as it progresses.
 * `build` is called with the transferred file list and must return
 * { db, art, fonts? } as Uint8Arrays.
 */
export async function run(link, hostFiles, build, report = () => {}) {
  report({ phase: 'index', detail: 'reading the device' });
  const deviceFiles = await deviceIndex(link, (n) => report({ phase: 'index', done: n }));
  const p = plan(hostFiles, deviceFiles);

  const free = await link.free();
  const needed = p.upload.reduce((n, f) => n + f.size, 0);
  const freed = p.remove.reduce((n, path) => n + (deviceFiles.get(path) || 0), 0);
  if (needed > free.free + freed) {
    throw new SyncError(
      `not enough space: need ${fmtBytes(needed)}, have ${fmtBytes(free.free + freed)}`,
    );
  }

  // 1. Music first: the database that will reference these files is written last.
  let sent = 0;
  for (const f of p.upload) {
    report({ phase: 'copy', done: sent, total: p.bytes, detail: f.relPath });
    const data = new Uint8Array(await (await f.handle.getFile()).arrayBuffer());
    await link.putFile(f.devicePath, data, {
      onProgress: (n) => report({ phase: 'copy', done: sent + n, total: p.bytes, detail: f.relPath }),
    });
    sent += f.size;
  }

  // 2. Remove what is no longer in the library.
  let removed = 0;
  for (const path of p.remove) {
    report({ phase: 'remove', done: ++removed, total: p.remove.length, detail: path });
    try {
      await link.remove(path);
    } catch (e) {
      if (!(e instanceof LinkError)) throw e; // already gone is fine
    }
  }

  // 3. Build and write the library. Art before the database, since the database
  //    references art slots.
  report({ phase: 'build', detail: 'building the library' });
  const built = await build(hostFiles);

  await link.mkdir(DEVICE_DIR);
  if (built.fonts) {
    await link.mkdir(`${DEVICE_DIR}/fonts`);
    for (const [name, data] of Object.entries(built.fonts)) {
      report({ phase: 'write', detail: `fonts/${name}` });
      await link.putFile(`${DEVICE_DIR}/fonts/${name}`, data);
    }
  }
  report({ phase: 'write', detail: 'artwork' });
  await link.putFile(`${DEVICE_DIR}/artwork.ipap`, built.art, {
    onProgress: (n, t) => report({ phase: 'write', done: n, total: t, detail: 'artwork' }),
  });
  report({ phase: 'write', detail: 'library' });
  await link.putFile(`${DEVICE_DIR}/library.ipdb`, built.db, {
    onProgress: (n, t) => report({ phase: 'write', done: n, total: t, detail: 'library' }),
  });

  // The stamp records the last verified library; a new one must re-verify.
  try {
    await link.remove(`${DEVICE_DIR}/.verified`);
  } catch (e) {
    if (!(e instanceof LinkError)) throw e;
  }

  await link.syncDone();
  report({ phase: 'done', detail: `${p.upload.length} copied, ${p.remove.length} removed` });
  return { uploaded: p.upload.length, removed: p.remove.length, bytes: p.bytes };
}

/** Reads the device journal and clears it. Returns the raw bytes, or null if absent. */
export async function pullJournal(link) {
  const path = `${DEVICE_DIR}/journal.bin`;
  const st = await link.stat(path);
  if (st.kind !== 1 || st.size === 0) return null;
  const data = await link.getFile(path);
  await link.remove(path);
  return data;
}

export function fmtBytes(n) {
  if (n >= 1e9) return (n / 1e9).toFixed(1) + ' GB';
  if (n >= 1e6) return (n / 1e6).toFixed(1) + ' MB';
  if (n >= 1e3) return (n / 1e3).toFixed(0) + ' KB';
  return n + ' B';
}
