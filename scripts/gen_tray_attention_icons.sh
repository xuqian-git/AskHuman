#!/usr/bin/env bash
set -euo pipefail

# Build template-compatible attention variants from the hand-composed tray icons.
# A transparent moat separates the smaller solid badge from the robot outline.
script_dir="$(cd "$(dirname "$0")" && pwd)"
tray_dir="$script_dir/../src-tauri/icons/tray"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

for state in idle stopped; do
  source_icon="$tray_dir/tray-$state.png"
  mask="$tmp_dir/$state-mask.png"

  magick "$source_icon" -alpha extract \
    -fill black -stroke none -draw 'circle 39,8 39,1.5' \
    -fill white -draw 'circle 39,8 39,4' \
    "$mask"
  magick -size 48x36 xc:black "$mask" -alpha off \
    -compose CopyOpacity -composite -strip \
    "PNG32:$tray_dir/tray-$state-attention.png"
done
