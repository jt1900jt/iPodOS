# ipod-os

A replacement UI for the iPod Classic (6G/6.5G/7G), built as a fork of Rockbox's application
layer with a companion that prepares the library on the host.

## Layout

| Path | Contents |
|---|---|
| `docs/ipdb-format.md` | Spec for `library.ipdb` (library database) and `artwork.ipap` (pre-rendered art) |
| `companion/` | Rust crate `ipdb`: scanner, builder, writer, validating reader, CLI. Core compiles without filesystem deps for the browser companion. |
| `device/ipdb/` | Device-side C reader: no allocation, no I/O, ~3 KB on ARM926 |
| `device/tools/` | Host tools for the C reader: `ipdb_dump`, and `ipdb_fuzz` (mutation fuzzer, ASan/UBSan) |

## Build

Requires Rust 1.80+, a C compiler, and ffmpeg (only to regenerate test fixtures).

    cd companion && cargo build --release

Build a library onto a mounted iPod:

    ./companion/target/release/ipdb build --source /path/to/music --out /media/IPOD/.ipodos --prefix /Music

`--source` must mirror the device's music folder: a file at `<source>/A/B.flac` is referenced as
`<prefix>/A/B.flac` on the device.

Other commands:

    ipdb verify DIR
    ipdb dump DIR --albums --tracks
    ipdb art-export DIR --class LRGE --id 0 --out cover.png

## Tests

    cd companion && cargo test
    cd companion && cargo test --release --test scale -- --ignored --nocapture
    cd device && make test
    cd device && make arm-check

`cargo test` writes `companion/target/test-library`, which `make test` loads through the C reader
and then fuzzes. `tests/make-fixtures.sh` regenerates the fixture library.
