#!/bin/bash
# usage: pss_tree.sh ROOT_PID  -> prints "<total PSS MiB> <process count>" over the root and all descendants
root=$1
pids=$root; frontier=$root
while [ -n "$frontier" ]; do next=""; for p in $frontier; do next="$next $(pgrep -P $p | tr '\n' ' ')"; done; frontier=$(echo $next); pids="$pids $frontier"; done
tot=0; n=0
for p in $pids; do v=$(awk '/^Pss:/{print $2}' /proc/$p/smaps_rollup 2>/dev/null); [ -n "$v" ] && tot=$((tot+v)) && n=$((n+1)); done
echo "$((tot/1024)) $n"
