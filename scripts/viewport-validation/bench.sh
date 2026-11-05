#!/bin/sh
# bench.sh wk|metal|sw <runs.json> <out.jsonl>
SELF=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd); TEV=${TEV:-/tmp/tev}; SNAP=$(ls -d $TEV/snap/* | head -1)
case "$1" in
 wk) exec $TEV/bin-bench/vv-wk "$2" "$3" "$SELF/page" "$TEV/rt" "$TEV/assets" "$TEV/png" ;;
 metal) exec node "$SELF/vv-chrome.mjs" "$2" "$3" "$SELF/page" "$TEV/rt" "$TEV/assets" "$TEV/png" "$SNAP/runtime/node_modules" ;;
 sw) SW=1 exec node "$SELF/vv-chrome.mjs" "$2" "$3" "$SELF/page" "$TEV/rt" "$TEV/assets" "$TEV/png" "$SNAP/runtime/node_modules" ;;
esac
