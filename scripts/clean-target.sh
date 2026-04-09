#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly TARGET_DIR="${REPO_ROOT}/src-tauri/target"
readonly DEBUG_DIR="${TARGET_DIR}/debug"
readonly DEFAULT_DAYS=3
readonly DEFAULT_HOURS=6

days="${DEFAULT_DAYS}"
hours=""
dry_run=0
remove_all=0
scope="all"
now_epoch="$(date +%s)"

usage() {
	cat <<'EOF'
Usage: scripts/clean-target.sh [options]

Clean stale artifacts under src-tauri/target.

Options:
  --days <n>      Remove artifacts older than n days. Default: 3
  --hours <n>     Remove artifacts older than n hours
  --scope <name>  Remove targets for a named scope: all | debug-cache. Default: all
  --all           Remove all recognized target artifacts, ignoring age
  --dry-run       Print candidates without deleting them
  -h, --help      Show this help message
EOF
}

fail() {
	printf 'Error: %s\n' "$1" >&2
	exit 1
}

collect_paths() {
	local list_file="$1"
	local path
	local profile_dir

	find "${TARGET_DIR}" -type d \
		\( \
			-name '.fingerprint' -o \
			-name 'build' -o \
			-name 'bundle' -o \
			-name 'deps' -o \
			-name 'examples' -o \
			-name 'incremental' \
		\) | sort -u | while IFS= read -r path; do
		append_if_stale "${list_file}" "${path}"
	done

	find "${TARGET_DIR}" -type f \
		\( -name '.DS_Store' -o -name '.rustc_info.json' \) | sort -u | while IFS= read -r path; do
		append_if_stale "${list_file}" "${path}"
	done

	while IFS= read -r profile_dir; do
		find "${profile_dir}" -mindepth 1 -maxdepth 1 -type f \
			! -name '.cargo-lock' | sort -u | while IFS= read -r path; do
			append_if_stale "${list_file}" "${path}"
		done
	done < <(find "${TARGET_DIR}" -type d \( -name 'debug' -o -name 'release' \) | sort -u)
}

collect_debug_cache_paths() {
	local list_file="$1"
	local cache_dir

	for cache_dir in "${DEBUG_DIR}/deps" "${DEBUG_DIR}/incremental"; do
		append_if_stale "${list_file}" "${cache_dir}"
	done
}

mtime_epoch() {
	local path="$1"

	if stat -f '%m' "${path}" >/dev/null 2>&1; then
		stat -f '%m' "${path}"
	else
		stat -c '%Y' "${path}"
	fi
}

append_if_stale() {
	local list_file="$1"
	local path="$2"
	local path_mtime
	local age_seconds

	[[ -e "${path}" ]] || return 0

	if [[ "${remove_all}" -eq 1 ]]; then
		printf '%s\n' "${path}" >>"${list_file}"
		return 0
	fi

	path_mtime="$(mtime_epoch "${path}")"
	age_seconds="$((now_epoch - path_mtime))"

	if [[ -n "${hours}" ]]; then
		if ((age_seconds >= hours * 3600)); then
			printf '%s\n' "${path}" >>"${list_file}"
		fi
	elif ((age_seconds >= days * 86400)); then
		printf '%s\n' "${path}" >>"${list_file}"
	fi
}

estimate_kb() {
	local total_kb="0"
	local path
	local size_kb

	while IFS= read -r path; do
		[[ -e "${path}" ]] || continue
		size_kb="$(du -sk "${path}" | awk '{print $1}')"
		total_kb="$((total_kb + size_kb))"
	done <"$1"

	printf '%s' "${total_kb}"
}

print_paths() {
	local path

	while IFS= read -r path; do
		[[ -n "${path}" ]] || continue
		printf '  %s\n' "${path}"
	done <"$1"
}

delete_paths() {
	local path

	while IFS= read -r path; do
		[[ -e "${path}" ]] || continue
		rm -rf -- "${path}"
	done <"$1"
}

while [[ $# -gt 0 ]]; do
	case "$1" in
	--days)
		shift
		[[ $# -gt 0 ]] || fail "--days requires a value"
		days="$1"
		;;
	--hours)
		shift
		[[ $# -gt 0 ]] || fail "--hours requires a value"
		hours="$1"
		;;
	--scope)
		shift
		[[ $# -gt 0 ]] || fail "--scope requires a value"
		scope="$1"
		;;
	--all)
		remove_all=1
		;;
	--dry-run)
		dry_run=1
		;;
	-h | --help)
		usage
		exit 0
		;;
	*)
		fail "unknown option: $1"
		;;
	esac
	shift
done

[[ -d "${TARGET_DIR}" ]] || fail "target directory not found: ${TARGET_DIR}"
[[ "${days}" =~ ^[0-9]+$ ]] || fail "--days must be a non-negative integer"
[[ -z "${hours}" || "${hours}" =~ ^[0-9]+$ ]] || fail "--hours must be a non-negative integer"
[[ "${scope}" == "all" || "${scope}" == "debug-cache" ]] || fail "--scope must be one of: all, debug-cache"

if [[ "${scope}" == "debug-cache" && -z "${hours}" ]]; then
	hours="${DEFAULT_HOURS}"
fi

candidate_file="$(mktemp)"
trap 'rm -f "${candidate_file}"' EXIT

case "${scope}" in
all)
	collect_paths "${candidate_file}"
	;;
debug-cache)
	collect_debug_cache_paths "${candidate_file}"
	;;
esac

sort -u "${candidate_file}" -o "${candidate_file}"

if [[ ! -s "${candidate_file}" ]]; then
	if [[ "${remove_all}" -eq 1 ]]; then
		printf 'No target artifacts matched for scope %s.\n' "${scope}"
	else
		if [[ -n "${hours}" ]]; then
			printf 'No target artifacts older than %s hour(s) matched for scope %s.\n' "${hours}" "${scope}"
		else
			printf 'No target artifacts older than %s day(s) matched for scope %s.\n' "${days}" "${scope}"
		fi
	fi
	exit 0
fi

total_kb="$(estimate_kb "${candidate_file}")"
total_mb="$((total_kb / 1024))"

printf 'Target directory: %s\n' "${TARGET_DIR}"
printf 'Scope: %s\n' "${scope}"
if [[ "${remove_all}" -eq 1 ]]; then
	printf 'Mode: remove all recognized target artifacts\n'
else
	if [[ -n "${hours}" ]]; then
		printf 'Mode: remove artifacts older than %s hour(s)\n' "${hours}"
	else
		printf 'Mode: remove artifacts older than %s day(s)\n' "${days}"
	fi
fi
printf 'Estimated reclaim: %s MB\n' "${total_mb}"
printf 'Candidates:\n'
print_paths "${candidate_file}"

if [[ "${dry_run}" -eq 1 ]]; then
	printf 'Dry run only. No files were deleted.\n'
	exit 0
fi

delete_paths "${candidate_file}"
printf 'Cleanup completed.\n'
