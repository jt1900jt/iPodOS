// Loads the library builder compiled to WebAssembly and wraps its C ABI.
// See companion/src/wasm.rs; build it with companion/build-wasm.sh.

const PAGE = 65536;

export class Builder {
  constructor(instance) {
    this.x = instance.exports;
    this.enc = new TextEncoder();
    this.dec = new TextDecoder();
  }

  static async load(url = './ipdb.wasm') {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`cannot load ${url}: run companion/build-wasm.sh first`);
    const bytes = await res.arrayBuffer();
    // The module is freestanding apart from a few stubs Rust emits for panics.
    let exports = null;
    const imports = {
      env: {
        __wbindgen_throw: () => { throw new Error('builder panicked'); },
      },
      // Stubs for the handful of WASI calls Rust's std emits. The builder does no I/O
      // of its own; these exist so the module links.
      wasi_snapshot_preview1: {
        proc_exit: (code) => { throw new Error('builder exited with ' + code); },
        fd_write: () => 0,
        fd_close: () => 0,
        fd_seek: () => 0,
        random_get: (ptr, len) => {
          crypto.getRandomValues(new Uint8Array(exports.memory.buffer, ptr, len));
          return 0;
        },
        environ_sizes_get: () => 0,
        environ_get: () => 0,
      },
    };
    const { instance } = await WebAssembly.instantiate(bytes, imports);
    exports = instance.exports;
    return new Builder(instance);
  }

  get mem() {
    return new Uint8Array(this.x.memory.buffer);
  }

  /** Copies bytes into wasm memory; returns [ptr, len] for the caller to free. */
  put(bytes) {
    const ptr = this.x.ipdb_alloc(bytes.length || 1);
    this.mem.set(bytes, ptr);
    return [ptr, bytes.length];
  }

  putText(s) {
    return this.put(this.enc.encode(s));
  }

  free(ptr, len) {
    this.x.ipdb_free(ptr, len || 1);
  }

  read(ptr, len) {
    return this.mem.slice(ptr, ptr + len);
  }

  reset() {
    this.x.ipdb_reset();
  }

  /** Parses one audio file. Returns false if its tags could not be read. */
  addTrack(relPath, bytes, mtime = 0) {
    const [pp, pl] = this.putText(relPath);
    const [dp, dl] = this.put(bytes);
    const ok = this.x.ipdb_add_track(pp, pl, dp, dl, mtime >>> 0) === 1;
    this.free(dp, dl);
    this.free(pp, pl);
    return ok;
  }

  addFolderArt(dir, bytes) {
    const [pp, pl] = this.putText(dir);
    const [dp, dl] = this.put(bytes);
    const used = this.x.ipdb_add_folder_art(pp, pl, dp, dl) === 1;
    this.free(dp, dl);
    this.free(pp, pl);
    return used;
  }

  addPlaylist(name, entries) {
    const [np, nl] = this.putText(name);
    const [ep, el] = this.putText(entries.join('\n'));
    this.x.ipdb_add_playlist(np, nl, ep, el);
    this.free(ep, el);
    this.free(np, nl);
  }

  /** uid the database derives from a library-relative path. */
  pathUid(relPath) {
    const [p, l] = this.putText(relPath);
    const uid = this.x.ipdb_path_uid(p, l) >>> 0;
    this.free(p, l);
    return uid;
  }

  /** Feeds one track's running history in; call before build() to get smart playlists. */
  addHistory(uid, s) {
    this.x.ipdb_add_history(uid >>> 0, s.plays >>> 0, s.skips >>> 0,
                            s.lastPlayed >>> 0, s.firstSeen >>> 0, s.rating >>> 0);
  }

  get trackCount() {
    return this.x.ipdb_track_count();
  }

  /** Builds the database and art pack. Returns { db, art, warnings }. */
  build(prefix = '/Music', generation = Math.floor(Date.now() / 1000)) {
    const [pp, pl] = this.putText(prefix);
    const n = this.x.ipdb_build(pp, pl, BigInt(generation));
    this.free(pp, pl);
    if (n < 0) throw new Error('library build failed: ' + this.warnings());
    return {
      tracks: n,
      db: this.read(this.x.ipdb_db_ptr(), this.x.ipdb_db_len()),
      art: this.read(this.x.ipdb_art_ptr(), this.x.ipdb_art_len()),
      warnings: this.warnings(),
    };
  }

  /** Builds every font atlas. Returns { 'row-14.ipfn': Uint8Array, ... }. */
  fonts() {
    const out = {};
    const count = this.x.ipdb_font_count();
    const nameBuf = this.x.ipdb_alloc(64);
    for (let i = 0; i < count; i++) {
      const n = this.x.ipdb_font_name(i, nameBuf, 64);
      if (!n) continue;
      const name = this.dec.decode(this.read(nameBuf, n));
      if (this.x.ipdb_build_font(i) !== 1) continue;
      out[name] = this.read(this.x.ipdb_font_ptr(), this.x.ipdb_font_len());
    }
    this.free(nameBuf, 64);
    return out;
  }

  warnings() {
    const n = this.x.ipdb_warnings();
    return n ? this.dec.decode(this.read(this.x.ipdb_warn_ptr(), this.x.ipdb_warn_len())) : '';
  }
}

export const WASM_PAGE = PAGE;
