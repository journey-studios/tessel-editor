#!/bin/sh
# usage: [MODE=seek|frame] [TAG=s] [EXTRA_ENV...] matrix.sh <samples> <reps> ; interleaves before/after: B A, A B, ...
S=$1; N=$2
cd /tmp/bl131
i=1
while [ $i -le $N ]; do
  if [ $((i % 2)) -eq 1 ]; then order="before after"; else order="after before"; fi
  for v in $order; do
    MODE=${MODE:-frame} SAMPLES=$S PASSES=${PASSES:-5} OUT=/tmp/bl131/results/${TAG:-m}$S-$v-$i.json python3 drv.py /tmp/bl131/tessel-editor /tmp/bl131/rt-$v-instr ${TAG:-m}$S-$v-$i || echo FAIL $v $i
  done
  i=$((i+1))
done
