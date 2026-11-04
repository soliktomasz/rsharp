#!/bin/sh
# RSharp as a macOS app bundle: tools/package-macos.sh [OUT.app] (dist/RSharp.app by default), built in release unless
# PROFILE names another profile. The sidecars this build made go inside it, beside the executable
# (Contents/MacOS/sidecars), which is where RSharp looks for them first — so the bundle runs on any Mac with the .NET
# SDK installed, wherever it is moved to. It is ad-hoc signed, enough to run on the Mac that built it; CODESIGN_IDENTITY
# names a Developer ID to sign with, and notarizing is phase 13's own step.
set -eu

die() { echo "package-macos: $*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || die "an app bundle is macOS's, and this is $(uname -s)"
out=${1:-dist/RSharp.app}
case $out in *.app) ;; *) die "OUT must end in .app: $out" ;; esac
profile=${PROFILE:-release}
case $profile in
    release) dir=target/release ;;
    debug | dev) dir=target/debug ;;
    *) dir=target/$profile ;;
esac

# The permission macOS grants RSharp (Screen Recording, Accessibility) is keyed to the bundle's identifier: it stays
# the same across releases, so a person grants it once. BUNDLE_ID changes it knowingly.
bundle_id=${BUNDLE_ID:-com.rsharp.ide}

cargo=$(command -v cargo || true)
[ -n "$cargo" ] || cargo=$HOME/.cargo/bin/cargo
[ -x "$cargo" ] || die "cargo was not found: install Rust, or put its bin folder on PATH"
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -1)
[ -n "$version" ] || die "Cargo.toml names no version"

echo "package-macos: building rsharp ($profile)" >&2
"$cargo" build --profile "$profile" --package rsharp-app --bin rsharp
[ -x "$dir/rsharp" ] || die "the build made no $dir/rsharp"

rm -rf "$out"
mkdir -p "$out/Contents/MacOS" "$out/Contents/Resources"
cp "$dir/rsharp" "$out/Contents/MacOS/rsharp"
if [ -d "$dir/sidecars" ]; then
    # -L takes the links the build made (Roslyn's server is one, into the NuGet cache) as what they point at.
    # Resources, not MacOS: a bundle's MacOS folder is its code, which codesign reads file by file, and npm's tree is
    # not code (paths::carried looks here, and beside the executable, for them).
    cp -RL "$dir/sidecars" "$out/Contents/Resources/sidecars"
    # Roslyn's server carries its symbols and its documentation, which nothing reads while it runs.
    find "$out/Contents/Resources/sidecars" -name '*.pdb' -delete
else
    echo "package-macos: this build carries no sidecars: C#, XAML, web files and the debugger will not work in this bundle" >&2
fi

# gpui asks for a real application (its window is the app's), and Finder reads these to know what it opened.
cat >"$out/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key><string>en</string>
	<key>CFBundleExecutable</key><string>rsharp</string>
	<key>CFBundleIdentifier</key><string>$bundle_id</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundleName</key><string>RSharp</string>
	<key>CFBundleDisplayName</key><string>RSharp</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>$version</string>
	<key>CFBundleVersion</key><string>$version</string>
	<key>CFBundleSupportedPlatforms</key><array><string>MacOSX</string></array>
	<key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
	<key>LSMinimumSystemVersion</key><string>11.0</string>
	<key>NSHighResolutionCapable</key><true/>
	<key>NSHumanReadableCopyright</key><string>MIT Licence</string>
	<key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

# A bundle copied from another Mac carries quarantine and old signatures; the signature must be the last thing written.
xattr -cr "$out"
# The bundle alone, not `--deep`: npm's `node_modules/.bin` shim folders read as bundles to a deep walk, which stops
# there. The Mach-O files inside were signed where the .NET packages were built; notarization (phase 13) is what has to
# sign each with the Developer ID before sealing.
if [ -n "${CODESIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$out"
else
    # Ad-hoc: what Apple Silicon insists on for any executable. A bundle that travels to another Mac still needs a
    # Developer ID and notarization before Gatekeeper opens it without a fight (phase 13's own step).
    codesign --force --sign - "$out"
    echo "package-macos: ad-hoc signed; see CODESIGN_IDENTITY for a Developer ID" >&2
fi

echo "package-macos: $(du -sh "$out" | cut -f1) at $(cd "$(dirname "$out")" && pwd)/$(basename "$out")" >&2
