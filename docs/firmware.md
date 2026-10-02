# Firmware (Rockbox fork)

The device side lives in a fork of Rockbox, branch `ipodos`, based on upstream commit
`f61d379824bbec737a059f5d25e18076cf527b4c`. Patches touch three upstream files; everything
else is new code in `apps/shell/`.

| Change | Purpose |
|---|---|
| `firmware/export/config/ipod6g.h` | defines `HAVE_IPODOS_SHELL` |
| `apps/main.c` | calls `shell_main()` instead of `root_menu()` when the flag is set |
| `apps/SOURCES` | builds `apps/shell/*.c` when the flag is set |
| `apps/shell/` | the shell, plus a copy of the ipdb reader (`device/ipdb/` in this repo) |

Keep `apps/shell/ipdb.{c,h}` identical to `device/ipdb/`. The fuzzing and tests run against
this repo's copy.

## Boundary with Rockbox

The shell replaces the UI layer only. From Rockbox it uses:

- the kernel, threads, and the buflib allocator (`core_alloc`; the library DB is one pinned allocation)
- the file API, LCD drawing, bitmap fonts, and the button driver
- `default_event_handler` for USB, power-off and other system events
- the playlist API (`playlist_create`, `playlist_insert_track`, `playlist_start`) and playback (`audio_*`)
- `global_settings` for shuffle and volume
- `root_menu()` as a fallback, opened from the ROCKBOX home item or when no valid library exists

Nothing in `apps/` is removed yet. The tree browser, menus, WPS and tagcache are still compiled
and reachable through the fallback. They go once the shell covers settings and the USB screen.

## Simulator

Build (Linux, SDL2 development headers installed):

    git clone -b ipodos https://github.com/jt1900jt/rockbox.git && cd rockbox && mkdir build-sim && cd build-sim
    ../tools/configure --target=ipod6g --type=s
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

## Phase 1 limits

- Bitmap fonts and full-screen `lcd_update()`. The compositor, dirty rects and AA fonts are phase 3.
- Thumbnails are the album's dominant color, not art. The art loader is phase 3.
- No wheel acceleration. Long lists use previous/next to jump by letter.
- Now Playing finds the DB track by a linear path search once per track change.
- A play request queues at most `max_playlist_size` tracks, centered on the selection.
- No resume of the last queue at boot.
