#!/usr/bin/env bash
# Renders every film into $OUT (default: ../../../reins-ads-out next to the checkout), then prints
# each file's size, frame count and loudness. Usage: ./render.sh [film ...]
set -euo pipefail
cd "$(dirname "$0")"
OUT="${OUT:-$(realpath ../..)/../reins-ads-out}"
mkdir -p "$OUT"
FILMS=("$@")
[ ${#FILMS[@]} -eq 0 ] && FILMS=(hero-wide hero-tall hook-rmrf hook-push hook-tokens pay-wide pay-tall)
declare -A NAME=(
  [hero-wide]=reins-hero-16x9 [hero-tall]=reins-hero-9x16
  [hook-rmrf]=reins-hook-rmrf-9x16 [hook-push]=reins-hook-push-9x16 [hook-tokens]=reins-hook-tokens-9x16
  [pay-wide]=reins-pay-16x9 [pay-tall]=reins-pay-9x16
)
[ -f assets/music_hero.wav ] || bun sound/build.ts
cargo build --release
for f in "${FILMS[@]}"; do
  file="$OUT/${NAME[$f]}.mp4"
  echo "== $f -> $file"
  cargo run --release -q -- --film "$f" render -o "$file"
  ffprobe -v error -select_streams v:0 -show_entries stream=width,height,nb_frames,duration -of csv=p=0 "$file"
  ffmpeg -hide_banner -nostats -i "$file" -af ebur128=peak=true -f null - 2>&1 | grep -E "^\s+(I:|Peak:)" | tr -s ' '
done
