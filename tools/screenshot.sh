#!/bin/sh
# The main window captured to a PNG on macOS: tools/screenshot.sh OUT.png [rsharp's arguments]. WAIT (seconds,
# 8 by default) is how long the IDE gets to open and settle. Needs Screen Recording for the terminal that runs it.
# SHOT_WINDOW names the window to capture by its title (Settings), else the main window.
# Its state is RSHARP_STATE when set, else a folder of its own removed as it ends: never the person's.
set -eu
out=$1; shift
own=""
if [ -z "${RSHARP_STATE:-}" ]; then
    own=$(mktemp -d)
    export RSHARP_STATE="$own"
fi
target/debug/rsharp "$@" &
pid=$!
trap 'kill $pid 2>/dev/null || true; [ -n "$own" ] && rm -rf "$own"' EXIT
sleep "${WAIT:-8}"
id=$(swift -e "
import CoreGraphics
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
let wanted = \"${SHOT_WINDOW:-}\"
for w in windows where (w[kCGWindowOwnerPID as String] as? Int32) == $pid && (w[kCGWindowLayer as String] as? Int) == 0
    && (wanted.isEmpty || (w[kCGWindowName as String] as? String) == wanted) {
    print(w[kCGWindowNumber as String]!); break
}")
[ -n "$id" ] || { echo "screenshot: RSharp showed no window" >&2; exit 2; }
screencapture -x -o -l"$id" "$out"
