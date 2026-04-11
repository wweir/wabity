#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG_SCRIPT_PATH="$ROOT_DIR/src-tauri/target/release/bundle/dmg/bundle_dmg.sh"
PATCH_SCRIPT_PATH="$ROOT_DIR/scripts/patch-bundle-dmg.mjs"
DMG_BACKGROUND_SVG_PATH="$ROOT_DIR/scripts/dmg-background.svg"
DMG_BACKGROUND_PNG_PATH="$ROOT_DIR/src-tauri/target/dmg-background.png"
TAURI_BIN="$ROOT_DIR/node_modules/.bin/tauri"
RUN_DMG_PATCH_WATCHER=1
RETRY_PATCH_MARKER="wabity-dmg-tolerance-patch"
BACKGROUND_PATCH_MARKER="wabity-dmg-background-v1-patch"
LAYOUT_PATCH_MARKER="wabity-dmg-layout-v2-patch"
VOLUME_ICON_PATCH_MARKER="wabity-dmg-volume-icon-v2-patch"

prepare_dmg_background() {
	if [[ ! -f "$DMG_BACKGROUND_SVG_PATH" ]]; then
		echo >&2 "Missing DMG background source: $DMG_BACKGROUND_SVG_PATH"
		return 1
	fi

	mkdir -p "$(dirname "$DMG_BACKGROUND_PNG_PATH")"
	sips -s format png "$DMG_BACKGROUND_SVG_PATH" --out "$DMG_BACKGROUND_PNG_PATH" >/dev/null
}

bundle_script_needs_patch() {
	local bundle_script_path="$1"

	[[ -f "$bundle_script_path" ]] ||
		return 1

	grep -q "Running AppleScript to make Finder stuff pretty" "$bundle_script_path" ||
		return 1

	! grep -q "$RETRY_PATCH_MARKER" "$bundle_script_path" ||
		! grep -q "$BACKGROUND_PATCH_MARKER" "$bundle_script_path" ||
		! grep -q "$LAYOUT_PATCH_MARKER" "$bundle_script_path" ||
		! grep -q "$VOLUME_ICON_PATCH_MARKER" "$bundle_script_path"
}

watch_bundle_script_and_patch() {
	while true; do
		if bundle_script_needs_patch "$DMG_SCRIPT_PATH"; then
			node "$PATCH_SCRIPT_PATH" "$DMG_SCRIPT_PATH" || true
		fi

		sleep 0.1
	done
}

watcher_pid=""
cleanup() {
	if [[ -n "$watcher_pid" ]]; then
		kill "$watcher_pid" 2>/dev/null || true
		wait "$watcher_pid" 2>/dev/null || true
	fi
}

trap cleanup EXIT

prepare_dmg_background
rm -f "$DMG_SCRIPT_PATH"

if [[ "$RUN_DMG_PATCH_WATCHER" -eq 1 ]]; then
	watch_bundle_script_and_patch &
	watcher_pid=$!
fi

"$TAURI_BIN" build --bundles dmg
