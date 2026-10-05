// Running listening history, kept in the browser between syncs.
//
// The device journal only covers events since the last sync and is cleared when read;
// this is the cumulative total it feeds into, and what the smart playlists are built
// from. Stored in IndexedDB rather than localStorage: a large library's history is
// hundreds of kilobytes and localStorage is both small and synchronous.

const DB_NAME = 'ipodos';
const STORE = 'history';
const TAGS = 'tags';

function open() {
  return new Promise((res, rej) => {
    const req = indexedDB.open(DB_NAME, 2);
    req.onupgradeneeded = (e) => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE);
      if (!db.objectStoreNames.contains(TAGS)) db.createObjectStore(TAGS);
    };
    req.onsuccess = () => res(req.result);
    req.onerror = () => rej(req.error);
  });
}

async function idb(mode, fn, store = STORE) {
  const db = await open();
  try {
    return await new Promise((res, rej) => {
      const tx = db.transaction(store, mode);
      const out = fn(tx.objectStore(store));
      tx.oncomplete = () => res(out.result ?? out);
      tx.onerror = () => rej(tx.error);
    });
  } finally {
    db.close();
  }
}

// --- parsed tag cache ---
//
// Parsing tags means reading every file, which dominates sync time on a large library
// even when nothing changed. Each entry holds the builder's own blob (metadata plus the
// cover image) keyed by path, size and mtime, so an edited or replaced file misses the
// cache and is read again.

const tagKey = (f) => `${f.relPath}|${f.size}|${f.mtime}`;

/** @returns {Map<string, Uint8Array>} */
export async function loadTags() {
  const raw = await idb('readonly', (st) => st.get('blobs'), TAGS);
  return raw instanceof Map ? raw : new Map();
}

export async function saveTags(map) {
  await idb('readwrite', (st) => st.put(map, 'blobs'), TAGS);
}

export async function clearTags() {
  await idb('readwrite', (st) => st.delete('blobs'), TAGS);
}

/** Drops entries for files no longer in the library, so the cache cannot grow forever. */
export function pruneTags(map, files) {
  const live = new Set(files.map(tagKey));
  let dropped = 0;
  for (const key of [...map.keys()]) {
    if (!live.has(key)) { map.delete(key); dropped++; }
  }
  return dropped;
}

export { tagKey };

/** @returns {Map<number, {plays,skips,lastPlayed,firstSeen,rating}>} */
export async function load() {
  const raw = await idb('readonly', (st) => st.get('tracks'));
  return raw instanceof Map ? raw : new Map();
}

export async function save(map) {
  await idb('readwrite', (st) => st.put(map, 'tracks'));
}

export async function clear() {
  await idb('readwrite', (st) => st.delete('tracks'));
}

const blank = () => ({ plays: 0, skips: 0, lastPlayed: 0, firstSeen: 0, rating: 0 });

/** Folds a raw journal (16-byte records) into the history map, in place. */
export function mergeJournal(map, data) {
  let plays = 0, skips = 0, ratings = 0;
  for (let o = 0; o + 16 <= data.length; o += 16) {
    const v = new DataView(data.buffer, data.byteOffset + o);
    const kind = data[o];
    const uid = v.getUint32(4, true);
    const when = v.getUint32(8, true);
    const value = v.getUint32(12, true);
    const s = map.get(uid) ?? blank();
    if (kind === 1) { s.plays++; s.lastPlayed = Math.max(s.lastPlayed, when); plays++; }
    else if (kind === 2) { s.skips++; s.lastPlayed = Math.max(s.lastPlayed, when); skips++; }
    else if (kind === 3) { s.rating = Math.min(value, 5); ratings++; }
    else continue;
    map.set(uid, s);
  }
  return { plays, skips, ratings };
}

/** Dates tracks the first time they are seen, so "recently added" means added here. */
export function noteSeen(map, uids, now = Math.floor(Date.now() / 1000)) {
  let added = 0;
  for (const uid of uids) {
    const s = map.get(uid) ?? blank();
    if (!s.firstSeen) { s.firstSeen = now; added++; }
    map.set(uid, s);
  }
  return added;
}
