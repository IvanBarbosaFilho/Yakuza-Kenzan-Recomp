#!/usr/bin/env bash
# Contact sheets of a frame step (work/step/NN_DDDD_TARGET.ppm, from touch work/step_now):
#   tools/kz_step_sheet.sh [dir] [seq] [target]   -> <dir>/<seq>_<target>_sheet_K.png, 24 draws per sheet
# Each tile is labelled with its draw number; index.txt says which program drew it.
D=${1:-$(dirname "$0")/../work/step}; S=${2:-01}; T=${3:-}
cd "$D" || exit 1
files=$(ls ${S}_*${T}.ppm 2>/dev/null | sort)
[ -z "$files" ] && { echo "no images for $S $T in $D"; exit 1; }
k=0; set -- $files
while [ $# -gt 0 ]; do
  batch=""; n=0
  while [ $# -gt 0 ] && [ $n -lt 24 ]; do batch="$batch $1"; shift; n=$((n+1)); done
  k=$((k+1)); out="${S}_${T:-all}_sheet_$k.png"
  nix-shell -p imagemagick --run "montage $batch -tile 6x4 -geometry +2+2 -pointsize 10 -label '%t' $out" >/dev/null 2>&1
  echo "$D/$out"
done
