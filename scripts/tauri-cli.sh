#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly DEFAULT_DEV_TARGET_DIR="${REPO_ROOT}/src-tauri/target"
readonly CLEAN_TARGET_SCRIPT="${SCRIPT_DIR}/clean-target.sh"

run_default_dev_cache_cleanup() {
	printf 'Cleaning stale debug cache under: %s\n' "${DEFAULT_DEV_TARGET_DIR}"
	bash "${CLEAN_TARGET_SCRIPT}" --scope debug-cache --hours 4
}

run_tauri_dev() {
	local dev_target_dir="${WABITY_TAURI_DEV_TARGET_DIR:-${DEFAULT_DEV_TARGET_DIR}}"

	mkdir -p "${dev_target_dir}"

	printf 'Using dev target dir: %s\n' "${dev_target_dir}"

	if [[ "${dev_target_dir}" == "${DEFAULT_DEV_TARGET_DIR}" ]]; then
		run_default_dev_cache_cleanup
	else
		printf 'Skipping default debug-cache cleanup because dev target dir is overridden.\n'
	fi

	CARGO_TARGET_DIR="${dev_target_dir}" exec tauri dev "$@"
}

main() {
	[[ $# -gt 0 ]] || exec tauri

	case "$1" in
	dev)
		shift
		run_tauri_dev "$@"
		;;
	*)
		exec tauri "$@"
		;;
	esac
}

main "$@"
