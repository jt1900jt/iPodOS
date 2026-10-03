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

## UI

- **Wheel acceleration.** Rockbox exposes wheel velocity; the shell currently moves one row
  per click regardless of speed.
- **Screen transitions.** The 7G LCD tops out at 39 fps full-screen, so a ~200 ms slide is
  feasible; needs measuring against the partial-redraw budget first.
- **Resume at boot.** Restore the last queue and position.

## Firmware

- **Release toolchain.** Builds here use the distro ARM GCC 13 rather than Rockbox's pinned
  GCC 9.5. Fine for development; release builds should use Rockbox's toolchain
  (`tools/rockboxdev.sh`).
- **Retire unused Rockbox UI.** The tree browser, menus, WPS and tagcache are still compiled
  and reachable through the ROCKBOX fallback. Drop them once the shell covers settings and
  the USB screen.
