#!/bin/sh
# Refuses to start a build, export or heavy measurement when the volume is too full.
#
# Usage: scripts/disk-guard.sh [needed_gib] [floor_gib]
#   needed_gib  space the next step may add (default 1)
#   floor_gib   free space that must remain afterwards (default 6)
# Exit 0 = go ahead, 3 = refuse. It only reads `df`; it never deletes anything. WHY: a past run
# filled the disk during a build; the plan caps own artifacts at 3 GiB per run and a 6 GiB floor.
set -eu
needed=${1:-1}
floor=${2:-6}
dir=${TESSEL_GUARD_DIR:-$(dirname -- "$0")/..}
free_kib=$(df -Pk "$dir" | awk 'NR==2 {print $4}')
free_gib=$((free_kib / 1024 / 1024))
if [ $((free_gib - needed)) -lt "$floor" ]; then
  echo "disk-guard: ${free_gib} GiB free, step needs up to ${needed} GiB and the floor is ${floor} GiB: refusing" >&2
  exit 3
fi
echo "disk-guard: ${free_gib} GiB free (step ${needed} GiB, floor ${floor} GiB): ok"
