#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DMG_SCRIPT_PATH="$ROOT_DIR/src-tauri/target/release/bundle/dmg/bundle_dmg.sh"
PATCH_SCRIPT_PATH="$ROOT_DIR/scripts/patch-bundle-dmg.mjs"
DMG_BACKGROUND_SVG_PATH="$ROOT_DIR/scripts/dmg-background.svg"
DMG_BACKGROUND_PNG_PATH="$ROOT_DIR/src-tauri/target/dmg-background.png"
DMG_OUTPUT_DIR="$ROOT_DIR/src-tauri/target/release/bundle/dmg"
TAURI_CONFIG_PATH="$ROOT_DIR/src-tauri/tauri.conf.json"
REPAIR_SCRIPT_PATH="$ROOT_DIR/scripts/dmg-install-and-repair.command"
TAURI_BIN="$ROOT_DIR/node_modules/.bin/tauri"
RUN_DMG_PATCH_WATCHER=1
RETRY_PATCH_MARKER="wabity-dmg-tolerance-patch"
BACKGROUND_PATCH_MARKER="wabity-dmg-background-v1-patch"
LAYOUT_PATCH_MARKER="wabity-dmg-layout-v2-patch"
VOLUME_ICON_PATCH_MARKER="wabity-dmg-volume-icon-v2-patch"

is_ci_environment() {
	[[ -n "${GITHUB_ACTIONS:-}" || "${CI:-}" == "true" || "${CI:-}" == "1" ]]
}

normalize_ci_environment() {
	if [[ "${CI:-}" == "1" ]]; then
		export CI=true
	fi
}

read_tauri_product_name() {
	node -e "const fs = require('fs'); const config = JSON.parse(fs.readFileSync(process.argv[1], 'utf8')); process.stdout.write(config.productName);" "$TAURI_CONFIG_PATH"
}

read_package_version() {
	node -p "require('./package.json').version"
}

normalized_arch() {
	case "$(uname -m)" in
	arm64 | aarch64)
		echo "aarch64"
		;;
	x86_64)
		echo "x64"
		;;
	*)
		uname -m
		;;
	esac
}

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

build_ci_dmg_from_app_bundle() {
	local product_name
	local version
	local arch
	local app_bundle_path
	local output_path
	local stage_dir

	product_name="$(read_tauri_product_name)"
	version="$(read_package_version)"
	arch="$(normalized_arch)"
	app_bundle_path="$ROOT_DIR/src-tauri/target/release/bundle/macos/${product_name}.app"
	output_path="$DMG_OUTPUT_DIR/${product_name}_${version}_${arch}.dmg"
	stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/wabity-dmg-stage.XXXXXX")"

	if [[ ! -d "$app_bundle_path" ]]; then
		echo >&2 "Missing app bundle for CI DMG packaging: $app_bundle_path"
		rm -rf "$stage_dir"
		return 1
	fi

	rm -f "$output_path"
	mkdir -p "$DMG_OUTPUT_DIR"

	cp -R "$app_bundle_path" "$stage_dir/"
	ln -s /Applications "$stage_dir/Applications"

	if [[ -f "$REPAIR_SCRIPT_PATH" ]]; then
		cp -p "$REPAIR_SCRIPT_PATH" "$stage_dir/修复.command"
		chmod +x "$stage_dir/修复.command"
	fi

	hdiutil create -volname "$product_name" -srcfolder "$stage_dir" -ov -format UDZO "$output_path"
	rm -rf "$stage_dir"
}

watcher_pid=""
cleanup() {
	if [[ -n "$watcher_pid" ]]; then
		kill "$watcher_pid" 2>/dev/null || true
		wait "$watcher_pid" 2>/dev/null || true
	fi
}

trap cleanup EXIT

normalize_ci_environment

if is_ci_environment; then
	"$TAURI_BIN" build --bundles app
	build_ci_dmg_from_app_bundle
else
	prepare_dmg_background
	rm -f "$DMG_SCRIPT_PATH"

	if [[ "$RUN_DMG_PATCH_WATCHER" -eq 1 ]]; then
		watch_bundle_script_and_patch &
		watcher_pid=$!
	fi

	"$TAURI_BIN" build --bundles dmg
fi
