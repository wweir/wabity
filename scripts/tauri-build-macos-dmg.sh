#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG_SCRIPT_PATH="$ROOT_DIR/src-tauri/target/release/bundle/dmg/bundle_dmg.sh"
PATCH_SCRIPT_PATH="$ROOT_DIR/scripts/patch-bundle-dmg.mjs"
TAURI_BIN="$ROOT_DIR/node_modules/.bin/tauri"

watch_bundle_script_and_patch() {
	while true; do
		if [[ -f "$DMG_SCRIPT_PATH" ]] \
			&& grep -q "Running AppleScript to make Finder stuff pretty" "$DMG_SCRIPT_PATH" \
			&& ! grep -q "wabity-dmg-tolerance-patch" "$DMG_SCRIPT_PATH"; then
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

watch_bundle_script_and_patch &
watcher_pid=$!

"$TAURI_BIN" build --bundles dmg
