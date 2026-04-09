#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
readonly DEFAULT_PORT="1420"
readonly DEFAULT_URL="http://localhost:${DEFAULT_PORT}"

dev_port="${WABITY_DEV_SERVER_PORT:-${DEFAULT_PORT}}"
dev_url="${WABITY_DEV_SERVER_URL:-${DEFAULT_URL}}"

fail() {
	printf 'Error: %s\n' "$1" >&2
	exit 1
}

find_listening_pids() {
	lsof -tiTCP:"${dev_port}" -sTCP:LISTEN 2>/dev/null || true
}

is_expected_vite_process() {
	local pid="$1"
	local command

	command="$(ps -p "${pid}" -o command= 2>/dev/null || true)"
	[[ -n "${command}" ]] || return 1
	[[ "${command}" == *"${REPO_ROOT}/node_modules/.bin/vite"* ]] || return 1
	[[ "${command}" == *"--port ${dev_port}"* ]] || return 1
}

is_dev_url_ready() {
	curl --silent --show-error --fail --max-time 3 "${dev_url}" >/dev/null 2>&1
}

reuse_existing_server_if_possible() {
	local pid

	while IFS= read -r pid; do
		[[ -n "${pid}" ]] || continue

		if is_expected_vite_process "${pid}" && is_dev_url_ready; then
			printf 'Reusing existing Vite dev server on %s (pid %s).\n' "${dev_url}" "${pid}"
			exit 0
		fi
	done < <(find_listening_pids)

	return 1
}

validate_port_is_free() {
	local pids

	pids="$(find_listening_pids)"
	[[ -z "${pids}" ]] || fail "port ${dev_port} is already in use by a non-reusable process: ${pids}"
}

main() {
	if reuse_existing_server_if_possible; then
		return 0
	fi

	validate_port_is_free

	cd "${REPO_ROOT}"
	exec npm run dev
}

main "$@"
