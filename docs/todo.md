# TODO

Deferred items, outside the current phase. See firmware.md for phase-by-phase scope.

## Branding

- **Custom boot logo.** The Rockbox logo and version text are suppressed when
  `HAVE_IPODOS_SHELL` is set, so the firmware now boots to a plain dark screen. Two things
  are still open:
  - A boot screen for the ~1.4 s before the shell appears. Options: a wordmark, or drawing
    Home immediately with the list filling in once the library loads.
  - The bootloader logo, which appears before the firmware runs. It lives in the flashed
    bootloader (`bootloader/ipod-s5l87xx.c` for the 6G/7G), so replacing it means building
    and flashing a bootloader. Riskier than a firmware swap, and best done once the rest is
    stable.

## Targets

- **iPod Video (5G/5.5G).** The `ipodvideo` target builds and runs in the simulator, but it
  is untested on hardware; Rockbox isn't installed on the 5.5G yet. Revisit after the 7G
  work settles. The 5G sets the performance floor: slower CPU, no ARMv5E instructions, LCD
  updates through the Broadcom chip, and 32 MB RAM on 30 GB models.

## Sync

- **Throughput.** A full 15.1 GB library synced at 4.1 MB/s against the 9.6 MB/s the
  transport benchmarks at. Frames are now batched into 256 KB USB writes and the next
  file is read while the current one transfers; measure again before chasing further.
  Remaining candidates: pipelining PUT_BEGIN/PUT_END round trips across files, and
  letting the device acknowledge less often.

## UI

- **Cover Flow motion on hardware.** The slide is in, but its frame rate has only been
  seen in the simulator; check it against the 39 fps ceiling on the device and shorten the
  slide if it drags.

- **Screen transitions.** The 7G LCD tops out at 39 fps full-screen, so a ~200 ms slide is
  feasible; needs measuring against the partial-redraw budget first.

## Companion

- **Partial reads for MP3.** FLAC, Ogg and Opus are now parsed from a one-megabyte
  prefix. MP3 still needs the whole file because its duration is derived from the stream
  length, and MP4 because its index can sit at either end; both would need the real length
  passed alongside a prefix.

- **Scrobbling.** `ipdb journal --scrobble` writes an Audioscrobbler log, but nothing
  uploads it; the browser could submit to Last.fm or ListenBrainz directly.
- **Playlist editing.** Playlists come only from .m3u files or the generated smart lists;
  the browser could build them against the scanned library.
- **Transcode on sync.** ffmpeg compiled to WebAssembly, for hi-res down to 16/44 or
  lossless to lossy when space is short.

## Audio

- **Headphone hiss.** Present in both the shell and stock Rockbox, with and without
  playback, so it is below our code: either Rockbox's codec driver or the hardware.
  Next step is comparing against the Apple firmware. If Apple is quiet, look at gain
  staging on the Cirrus codec (Rockbox may run the analog amp hot and attenuate
  digitally); if not, suspect the iFlash adapter coupling into the analog ground, the
  jack, or simply low-impedance headphones exposing the noise floor.

## Firmware

- **Release toolchain.** Builds here use the distro ARM GCC 13 rather than Rockbox's pinned
  GCC 9.5. Fine for development; release builds should use Rockbox's toolchain
  (`tools/rockboxdev.sh`).
- **Retire unused Rockbox UI.** The tree browser, menus, WPS and tagcache are still compiled
  and reachable through the ROCKBOX fallback. Drop them once the shell covers settings and
  the USB screen.
