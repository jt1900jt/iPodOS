# Firmware (Rockbox fork)

The device side lives in a fork of Rockbox, branch `ipodos`, based on upstream commit
`f61d379824bbec737a059f5d25e18076cf527b4c`. Patches touch three upstream files; everything
else is new code in `apps/shell/`.

| Change | Purpose |
|---|---|
| `firmware/export/config/ipod6g.h` | defines `HAVE_IPODOS_SHELL` (iPod Classic 6G/6.5G/7G) |
| `firmware/export/config/ipodvideo.h` | defines `HAVE_IPODOS_SHELL` (iPod Video 5G/5.5G) |
| `apps/main.c` | calls `shell_main()` instead of `root_menu()` when the flag is set |
| `apps/SOURCES` | builds `apps/shell/*.c` when the flag is set |
| `apps/shell/` | the shell, plus a copy of the ipdb reader (`device/ipdb/` in this repo) |

Keep `apps/shell/ipdb.{c,h}` identical to `device/ipdb/`. The fuzzing and tests run against
this repo's copy.

## Targets

| Rockbox target | Devices | SoC | RAM |
|---|---|---|---|
| `ipod6g` | Classic 6G, 6.5G, 7G | Samsung S5L8702, ARM926EJ-S | 64 MB |
| `ipodvideo` | Video 5G, 5.5G | PortalPlayer PP5021C, dual ARM7TDMI | 32 MB (30 GB models) or 64 MB |

The shell uses only Rockbox APIs that both targets provide. Both share a 320×240 RGB565 LCD
and click wheel button codes. Build both targets for every change; `ipod6g` is the primary target.

The 5G sets the performance and memory floor. It has a slower CPU without ARMv5E instructions,
so optimized blit routines need an ARMv4 path. Its LCD updates go through the Broadcom video
chip, and 30 GB models have 32 MB RAM, which lowers the library size cap.

## Boundary with Rockbox

The shell replaces the UI layer only. From Rockbox it uses:

- the kernel, threads, and the buflib allocator (`core_alloc`; the library DB is one pinned allocation)
- the file API, LCD drawing, bitmap fonts, and the button driver
- `default_event_handler` for USB, power-off and other system events
- the USB stack, with two added class drivers: `usbstack/usb_bulk.c` (vendor-class bulk for
  the companion link) and a reworked `usb_serial.c` data path
- the playlist API (`playlist_create`, `playlist_insert_track`, `playlist_start`) and playback (`audio_*`)
- `global_settings` for shuffle and volume
- `root_menu()` as a fallback, opened from the ROCKBOX home item or when no valid library exists

Nothing in `apps/` is removed yet. The tree browser, menus, WPS and tagcache are still compiled
and reachable through the fallback. They go once the shell covers settings and the USB screen.

## Simulator

Build (Linux, SDL2 development headers installed):

    git clone -b ipodos https://github.com/jt1900jt/rockbox.git && cd rockbox && mkdir build-sim && cd build-sim
    ../tools/configure --target=ipod6g --type=s     (or --target=ipodvideo for the 5G)
    make -j && make fullinstall

`make fullinstall` puts fonts and codecs into `build-sim/simdisk`, which is the simulator's disk root.

Stage a library. `DUR` sets track length in seconds so playback can be observed:

    DUR=180 OUT=/path/to/rockbox/build-sim/simdisk/Music ipod-os/companion/tests/make-fixtures.sh
    ipdb build --source /path/to/rockbox/build-sim/simdisk/Music --out /path/to/rockbox/build-sim/simdisk/.ipodos

Run interactively with `./rockboxui`. Keys: Up/Down = wheel, Enter = select, Esc = menu,
Space = play/pause, Left/Right = previous/next.

### Scripted runs

`IPODOS_SCRIPT` replaces button input with a token list and writes screenshots to
`simdisk/shots/NAME.bmp`. It works without a display:

    SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy IPODOS_SCRIPT="w50 d d s shot:albums s w100 shot:np q" ./rockboxui --nobackground

| Token | Meaning |
|---|---|
| `u` / `d` | wheel back / forward |
| `s` | select |
| `m` | menu (back) |
| `M` | menu held (home) |
| `p` | play/pause |
| `l` / `r` | previous / next (letter jump in lists, track skip in Now Playing) |
| `wN` | wait N ticks (100 ticks = 1 s) |
| `shot:NAME` | write the current frame |
| `q` | quit |

Selecting the ROCKBOX home item leaves the shell, and scripted input stops at that point.

## Rendering

`shell_gfx.c` draws straight into Rockbox's RGB565 framebuffer and tracks a dirty rectangle;
`gfx_flush()` pushes it once per frame. A full-screen push costs ~25 ms on the 7G and scales
with pixel count, so screens repaint only what changed: moving the selection repaints two
rows, and the Now Playing tick repaints the progress area.

Text comes from `.ipfn` atlases (see font-format.md) with 8-bit coverage, blended per pixel,
so it stays sharp over art and gradients. Rockbox's own fonts are 1-bit and are not used by
the shell.

Gradients, washes and blends are ordered-dithered. Dark gradients band badly in RGB565, and
the design is mostly dark gradients.

## Album art

`shell_art.c` keeps a small cache of pre-rendered RGB565 slots read from `artwork.ipap`, with
a background loader thread at `PRIORITY_BACKGROUND`. `art_get()` returns a cached image or
NULL and queues a load, so the UI thread never waits on storage; the album's stored dominant
colour stands in until the art arrives. Rows just outside the viewport are prefetched so
scrolling finds them resident, and the queue is cleared on a view change so stale prefetches
do not crowd out what is now on screen.

The pack is ignored when its generation does not match the library, which would otherwise
pair covers with the wrong albums.

## Library load

The CRC covers the whole file and dominates load time (589 ms for a 7 MB library on the 7G,
against 24 ms for the structural checks). `/.ipodos/.verified` records the generation, size
and CRC last verified, so an unchanged library skips the CRC on later boots. The structural
checks, which are what make the accessors bounds-safe, always run.

## Remaining limits

- No wheel acceleration. Long lists use previous/next to jump by letter.
- Now Playing finds the DB track by a linear path search once per track change.
- A play request queues at most `max_playlist_size` tracks, centered on the selection.
- No resume of the last queue at boot.
- No animated transitions between screens.

## Sync

`shell_sync.c` provides the filesystem operations the companion drives over the link
(see link-protocol.md). The iPod stays in the shell while connected, so syncing needs no
disk mode and does not interrupt playback.

Transfers are staged to a temporary file and renamed on commit. The companion writes music
first, then deletions, then artwork, and the library last, so an interrupted sync leaves
the device showing its previous library rather than one referencing files that never
arrived. The verification stamp is removed with each new library, forcing a fresh checksum
on the next boot.
