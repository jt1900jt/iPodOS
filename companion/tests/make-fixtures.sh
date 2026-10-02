#!/usr/bin/env bash
# Regenerates tests/fixtures/library: short tagged files in every supported container,
# with embedded art, folder art, a multi-disc album, a compilation, untagged and broken files.
set -euo pipefail
# Env: DUR = track length in seconds (default 1), OUT = library dir (default fixtures/library).
cd "$(dirname "$0")"
L=${OUT:-fixtures/library}
DUR=${DUR:-1}
rm -rf "$L" && mkdir -p "$L"
q=(-hide_banner -loglevel error -y)
tone() { echo "-f lavfi -i sine=frequency=$1:duration=$DUR"; }

ffmpeg "${q[@]}" -f lavfi -i "mandelbrot=size=600x600:end_pts=1" -frames:v 1 fixtures/cover_a.jpg
ffmpeg "${q[@]}" -f lavfi -i "testsrc2=size=500x500" -frames:v 1 fixtures/cover_b.png
ffmpeg "${q[@]}" -f lavfi -i "gradients=size=640x480:c0=0x1d2b64:c1=0xf8a5c2:x0=0:y0=0:x1=640:y1=480" -frames:v 1 fixtures/cover_c.jpg

d="$L/Halcyon Drift/Low Tide Lights"; mkdir -p "$d"
for n in 1 2; do
  t=$([ $n = 1 ] && echo "After the Static" || echo "Half Light")
  ffmpeg "${q[@]}" $(tone $((300+n*40))) -i fixtures/cover_a.jpg -map 0 -map 1 -c:a flac -sample_fmt s16 -ar 44100 \
    -c:v copy -disposition:v attached_pic \
    -metadata title="$t" -metadata artist="Halcyon Drift" -metadata album="Low Tide Lights" \
    -metadata album_artist="Halcyon Drift" -metadata date="2019-03-01" -metadata track="$n" \
    -metadata genre="Electronic" -metadata composer="R. Hale" \
    -metadata REPLAYGAIN_TRACK_GAIN="-6.52 dB" -metadata REPLAYGAIN_ALBUM_GAIN="-7.10 dB" \
    "$d/0$n $t.flac"
done

d="$L/Mara Vell/Paper Moons"; mkdir -p "$d"
for n in 1 2; do
  t=$([ $n = 1 ] && echo "Barrow Lane" || echo "Paper Moons")
  ffmpeg "${q[@]}" $(tone $((400+n*40))) -i fixtures/cover_b.png -map 0 -map 1 -c:a libmp3lame -b:a 192k \
    -c:v copy -id3v2_version 3 -metadata:s:v comment="Cover (front)" \
    -metadata title="$t" -metadata artist="Mara Vell" -metadata album="Paper Moons" -metadata date="2021" \
    -metadata track="$n/2" -metadata genre="Folk" \
    "$d/0$n $t.mp3"
done

for disc in 1 2; do
  d="$L/The Quiet Hours/Northbound/CD$disc"; mkdir -p "$d"
  cp fixtures/cover_c.jpg "$d/cover.jpg"
  t=$([ $disc = 1 ] && echo "Copper Sky" || echo "Northbound")
  codec=$([ $disc = 1 ] && echo "aac -b:a 128k" || echo "alac -sample_fmt s16p")
  ffmpeg "${q[@]}" $(tone $((500+disc*30))) -c:a $codec \
    -metadata title="$t" -metadata artist="The Quiet Hours" -metadata album="Northbound" \
    -metadata date="2017" -metadata track="1" -metadata disc="$disc/2" -metadata genre="Folk" \
    "$d/01 $t.m4a"
done

d="$L/Night Drive Mix"; mkdir -p "$d"
ffmpeg "${q[@]}" $(tone 610) -c:a libvorbis -metadata title="Glass Weather" -metadata artist="Kesh" \
  -metadata album="Night Drive Mix" -metadata TRACKNUMBER=1 -metadata genre="Electronic" "$d/01 Glass Weather.ogg"
ffmpeg "${q[@]}" $(tone 640) -c:a libopus -metadata title="Neon Ferry" -metadata artist="Ondine Park" \
  -metadata album="Night Drive Mix" -metadata TRACKNUMBER=2 "$d/02 Neon Ferry.opus"

d="$L/Ångström/Æther"; mkdir -p "$d"
ffmpeg "${q[@]}" $(tone 700) -c:a flac -sample_fmt s32 -ar 96000 \
  -metadata title="Øresund" -metadata artist="Ångström" -metadata album="Æther" -metadata track=1 "$d/01 Øresund.flac"

d="$L/Loose"; mkdir -p "$d"
ffmpeg "${q[@]}" $(tone 800) -c:a pcm_s16le -map_metadata -1 -fflags +bitexact "$d/Driftwood Radio.wav"
head -c 4096 /dev/urandom > "$d/broken.mp3"

printf '#EXTM3U\n#EXTINF:1,Kesh - Glass Weather\nNight Drive Mix/01 Glass Weather.ogg\nThe Quiet Hours/Northbound/CD1/01 Copper Sky.m4a\nMissing/nothing.mp3\n./Mara Vell/Paper Moons/../Paper Moons/02 Paper Moons.mp3\n' > "$L/Night Drive.m3u8"
echo "fixtures written to $L"
