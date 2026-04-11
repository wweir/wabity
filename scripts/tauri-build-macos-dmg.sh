#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG_SCRIPT_PATH="$ROOT_DIR/src-tauri/target/release/bundle/dmg/bundle_dmg.sh"
PATCH_SCRIPT_PATH="$ROOT_DIR/scripts/patch-bundle-dmg.mjs"
TAURI_BIN="$ROOT_DIR/node_modules/.bin/tauri"
RUN_DMG_PATCH_WATCHER=1
RETRY_PATCH_MARKER="wabity-dmg-tolerance-patch"
EXTRAS_PATCH_MARKER="wabity-dmg-extra-files-patch"
VOLUME_ICON_PATCH_MARKER="wabity-dmg-volume-icon-patch"

bundle_script_needs_patch() {
	local bundle_script_path="$1"

	[[ -f "$bundle_script_path" ]] ||
		return 1

	grep -q "Running AppleScript to make Finder stuff pretty" "$bundle_script_path" ||
		return 1

	! grep -q "$RETRY_PATCH_MARKER" "$bundle_script_path" ||
		! grep -q "$EXTRAS_PATCH_MARKER" "$bundle_script_path" ||
		! grep -q "$VOLUME_ICON_PATCH_MARKER" "$bundle_script_path"
}

watch_bundle_script_and_patch() {
	while true; do
		if bundle_script_needs_patch "$DMG_SCRIPT_PATH"; then
			node "$PATCH_SCRIPT_PATH" "$DMG_SCRIPT_PATH"
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

if [[ "$RUN_DMG_PATCH_WATCHER" -eq 1 ]]; then
	watch_bundle_script_and_patch &
	watcher_pid=$!
fi

"$TAURI_BIN" build --bundles dmg
