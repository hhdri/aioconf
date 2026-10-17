#!/bin/bash
# Hold one GPU memory offset under continuous qwen3:8b generation and check
# every answer against the stock answer.
#   REF=<stock hash from bench.py> tools/gpu/longrun.sh [OFFSET] [MINUTES]
REF=${REF:-}
S=$(dirname "$0"); OFF=${1:-3000}; MIN=${2:-30}
conf=$(mktemp)
set_offset() { printf 'gpu.mem_offset = %s\n' "$1" > "$conf"; sudo aioconf-nvidia apply "$conf" >/dev/null; }
trap 'set_offset 0; rm -f "$conf"; echo "offset restored to 0"' EXIT
set_offset "$OFF" || exit 1
since=$(date '+%Y-%m-%d %H:%M:%S'); end=$((SECONDS + MIN * 60)); chunk=0; total=0
while [ $SECONDS -lt $end ]; do
  chunk=$((chunk + 1))
  res=$(python3 -I "$S/bench.py" "long+$OFF chunk$chunk" 8) || { echo "chunk $chunk: benchmark failed"; exit 1; }
  total=$((total + 8))
  echo "$(date +%H:%M:%S) $res"
  hash=$(python3 -I -c "import json,sys; print(','.join(json.loads(sys.argv[1])['output_hashes']))" "$res")
  xid=$(sudo journalctl -k --since "$since" --no-pager 2>/dev/null | grep -ciE 'NVRM: Xid')
  [ -n "$REF" ] && [ "$hash" != "$REF" ] && { echo "STOP after $total answers: output changed ($hash)"; exit 1; }
  [ "$xid" -gt 0 ] && { echo "STOP after $total answers: $xid driver errors"; exit 1; }
done
echo "PASS: $total answers in $MIN minutes, no driver errors${REF:+, all identical to the reference}"
