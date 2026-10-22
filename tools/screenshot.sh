#!/bin/sh
# The main window captured to a PNG on macOS: tools/screenshot.sh OUT.png [rsharp's arguments]. WAIT (seconds,
# 8 by default) is how long the IDE gets to open and settle. Needs Screen Recording for the terminal that runs it.
set -eu
out=$1; shift
target/debug/rsharp "$@" &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT
sleep "${WAIT:-8}"
id=$(swift -e "
import CoreGraphics
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in windows where (w[kCGWindowOwnerPID as String] as? Int32) == $pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    print(w[kCGWindowNumber as String]!); break
}")
[ -n "$id" ] || { echo "screenshot: RSharp showed no window" >&2; exit 2; }
screencapture -x -o -l"$id" "$out"
