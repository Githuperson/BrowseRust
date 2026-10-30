#!/bin/bash
# usage: run.sh LABEL N -- cmd args...   (N heavy tabs; N=0 means one blank tab)
label=$1; n=$2; shift 3
export XDG_RUNTIME_DIR=/tmp/xdg WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 LIBGL_ALWAYS_SOFTWARE=1
curl -s localhost:8770/reset >/dev/null
urls=""
if [ "$n" = "0" ]; then urls="http://127.0.0.1:8770/blank"; else for i in $(seq 0 $((n-1))); do urls="$urls http://127.0.0.1:8770/h$i"; done; fi
start=$(date +%s.%N)
"$@" $urls >/dev/null 2>&1 &
pid=$!
if [ "$n" != "0" ]; then
  for t in $(seq 1 180); do c=$(curl -s localhost:8770/stats | python3 -c "import sys,json;print(len(json.load(sys.stdin)))"); [ "$c" -ge "$n" ] && break; sleep 0.5; done
  loadt=$(curl -s localhost:8770/stats | python3 -c "import sys,json;d=json.load(sys.stdin);print('%.1f'%max(d.values()) if d else 'none')")
else loadt="-"; sleep 8; fi
sleep 8
read pss procs < <(/tmp/claude-0/-home-user-BrowseRust/37c70a63-b6e4-591c-a003-03913cdd5cb1/scratchpad/bench/pss_tree.sh $pid)
echo "$label | tabs=$n | all pages built after ${loadt}s | total PSS ${pss} MiB across ${procs} processes"
tree=$pid; fr=$pid
while [ -n "$fr" ]; do nx=""; for p in $fr; do nx="$nx $(pgrep -P $p | tr '\n' ' ')"; done; fr=$(echo $nx); tree="$tree $fr"; done
kill $tree 2>/dev/null; sleep 1; kill -9 $tree 2>/dev/null; true
