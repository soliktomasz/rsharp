#!/bin/sh
# Phase 4's acceptance, Passenger's side: the steps of crates/rsharp-app/tests/phase4.rs taken through Passenger's own
# window (tools/passenger-parity/RSharpParityTests.cs, a test added to a copy of vendor/passenger), its solution folder
# written to crates/rsharp-app/tests/fixtures/phase4, which RSharp's run is compared with byte for byte.
# Needs the SDK vendor/passenger's global.json pins (to build and run Passenger) and SDK 10.0.1xx (the steps' own).
# Run it again when the submodule's pin moves.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -R "$root/vendor/passenger" "$work/passenger"
rm -f "$work/passenger/.git"
cp "$root/tools/passenger-parity/RSharpParityTests.cs" "$work/passenger/tests/Passenger.Tests/"
out="$root/crates/rsharp-app/tests/fixtures/phase4"
AVALONIA_TELEMETRY_OPTOUT=1 DOTNET_CLI_TELEMETRY_OPTOUT=1 RSHARP_PARITY_OUT="$out" \
    dotnet test "$work/passenger/tests/Passenger.Tests" --filter "FullyQualifiedName~RSharpParityTests"
echo "Passenger's files are in $out"
