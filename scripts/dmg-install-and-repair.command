#!/usr/bin/env bash

set -euo pipefail

APP_NAME="Wabity.app"
APP_DISPLAY_NAME="Wabity"
APP_BUNDLE_ID="com.wabity"
SOURCE_APP_DIR="$(cd "$(dirname "$0")" && pwd)/${APP_NAME}"
TARGET_APP_DIR="/Applications/${APP_NAME}"
SOURCE_INFO_PLIST="${SOURCE_APP_DIR}/Contents/Info.plist"
TARGET_INFO_PLIST="${TARGET_APP_DIR}/Contents/Info.plist"
TARGET_APP_PARENT_DIR="$(dirname "$TARGET_APP_DIR")"

run_osascript() {
	/usr/bin/osascript "$@"
}

show_error() {
	run_osascript -e "display alert \"${APP_DISPLAY_NAME} 安装失败\" message \"$1\" as critical"
}

show_info() {
	run_osascript -e "display dialog \"$1\" buttons {\"好\"} default button \"好\""
}

ask_choice() {
	local prompt="$1"
	local default_button="$2"
	shift 2
	local buttons=("$@")
	local applescript_buttons=""
	local first=1
	local button

	for button in "${buttons[@]}"; do
		if [[ $first -eq 1 ]]; then
			applescript_buttons="\"${button}\""
			first=0
		else
			applescript_buttons="${applescript_buttons}, \"${button}\""
		fi
	done

	run_osascript -e "button returned of (display dialog \"$prompt\" buttons {${applescript_buttons}} default button \"${default_button}\")"
}

read_plist_value() {
	local plist_path="$1"
	local key="$2"

	if [[ ! -f "$plist_path" ]]; then
		return 1
	fi

	/usr/libexec/PlistBuddy -c "Print :${key}" "$plist_path" 2>/dev/null || true
}

normalize_version() {
	local version="${1:-0}"
	local cleaned="${version//[^0-9.]/.}"

	while [[ "$cleaned" == .* ]]; do
		cleaned="${cleaned#.}"
	done

	while [[ "$cleaned" == *. ]]; do
		cleaned="${cleaned%.}"
	done

	if [[ -z "$cleaned" ]]; then
		cleaned="0"
	fi

	echo "$cleaned"
}

compare_versions() {
	local left
	local right
	left="$(normalize_version "$1")"
	right="$(normalize_version "$2")"

	local IFS=.
	local left_parts=($left)
	local right_parts=($right)
	local left_len="${#left_parts[@]}"
	local right_len="${#right_parts[@]}"
	local max_len="$left_len"

	if ((right_len > max_len)); then
		max_len="$right_len"
	fi

	local i left_num right_num
	for ((i = 0; i < max_len; i++)); do
		left_num="${left_parts[i]:-0}"
		right_num="${right_parts[i]:-0}"

		if ((10#$left_num > 10#$right_num)); then
			echo 1
			return 0
		fi

		if ((10#$left_num < 10#$right_num)); then
			echo -1
			return 0
		fi
	done

	echo 0
}

is_app_running() {
	pgrep -x "$APP_DISPLAY_NAME" >/dev/null 2>&1
}

wait_until_stopped() {
	local timeout_seconds="$1"
	local waited=0

	while is_app_running; do
		if ((waited >= timeout_seconds)); then
			return 1
		fi

		sleep 1
		((waited += 1))
	done

	return 0
}

request_app_quit() {
	run_osascript -e "tell application id \"${APP_BUNDLE_ID}\" to quit" >/dev/null 2>&1 || true
}

force_kill_app() {
	pkill -x "$APP_DISPLAY_NAME" >/dev/null 2>&1 || true
}

handle_running_app() {
	if ! is_app_running; then
		return 0
	fi

	local choice
	choice="$(ask_choice "${APP_DISPLAY_NAME} 当前正在运行。安装前需要先退出它。\\n\\n建议先正常退出，再继续安装。" "退出并继续" "退出并继续" "强制结束后继续" "取消安装")"

	case "$choice" in
	"退出并继续")
		request_app_quit
		if ! wait_until_stopped 15; then
			show_error "${APP_DISPLAY_NAME} 在 15 秒内没有退出。请先手动关闭应用后再重新运行这个安装脚本。"
			exit 1
		fi
		;;
	"强制结束后继续")
		force_kill_app
		if ! wait_until_stopped 5; then
			show_error "无法结束正在运行的 ${APP_DISPLAY_NAME} 进程。请先手动关闭应用后再重新运行这个安装脚本。"
			exit 1
		fi
		;;
	*)
		exit 0
		;;
	esac
}

describe_install_action() {
	local source_version="$1"
	local target_version="$2"
	local compare_result="$3"

	if [[ -z "$target_version" ]]; then
		echo "将安装 ${APP_DISPLAY_NAME} ${source_version} 到 /Applications。"
		return 0
	fi

	case "$compare_result" in
	1)
		echo "检测到已安装版本 ${target_version}，将升级到 ${source_version}。"
		;;
	0)
		echo "检测到已安装版本 ${target_version}，本次将执行覆盖安装。"
		;;
	-1)
		echo "检测到已安装版本 ${target_version}，当前 DMG 版本 ${source_version} 更旧，本次属于降级安装。"
		;;
	esac
}

confirm_version_action() {
	local source_version="$1"
	local target_version="$2"
	local compare_result="$3"
	local prompt action_line choice

	action_line="$(describe_install_action "$source_version" "$target_version" "$compare_result")"
	prompt="${action_line}\\n\\n"

	case "$compare_result" in
	1)
		prompt="${prompt}安装脚本会在复制前尝试退出正在运行的 ${APP_DISPLAY_NAME}，然后替换 /Applications 中的旧版本。"
		choice="$(ask_choice "$prompt" "继续安装" "继续安装" "取消安装")"
		;;
	0)
		prompt="${prompt}如果只是修复首次打开问题，可以继续覆盖安装。"
		choice="$(ask_choice "$prompt" "继续安装" "继续安装" "取消安装")"
		;;
	-1)
		prompt="${prompt}降级可能导致配置或数据兼容性问题，请确认你确实要回退。"
		choice="$(ask_choice "$prompt" "继续降级" "继续降级" "取消安装")"
		;;
	esac

	case "$choice" in
	"继续安装" | "继续降级")
		return 0
		;;
	*)
		exit 0
		;;
	esac
}

if [[ ! -d "$SOURCE_APP_DIR" ]]; then
	show_error "没有在当前磁盘镜像中找到 ${APP_NAME}。请确认这个脚本仍和应用放在同一个 DMG 中。"
	exit 1
fi

if [[ ! -d /Applications ]]; then
	show_error "没有找到 /Applications 目录，无法继续安装。"
	exit 1
fi

SOURCE_VERSION="$(read_plist_value "$SOURCE_INFO_PLIST" "CFBundleShortVersionString")"
SOURCE_VERSION="${SOURCE_VERSION:-未知版本}"
TARGET_VERSION=""

if [[ -f "$TARGET_INFO_PLIST" ]]; then
	TARGET_VERSION="$(read_plist_value "$TARGET_INFO_PLIST" "CFBundleShortVersionString")"
	TARGET_VERSION="${TARGET_VERSION:-未知版本}"
fi

VERSION_COMPARE_RESULT=1
if [[ -n "$TARGET_VERSION" ]]; then
	VERSION_COMPARE_RESULT="$(compare_versions "$SOURCE_VERSION" "$TARGET_VERSION")"
fi

confirm_version_action "$SOURCE_VERSION" "$TARGET_VERSION" "$VERSION_COMPARE_RESULT"
handle_running_app

mkdir -p "$TARGET_APP_PARENT_DIR"

TEMP_APP_DIR="$(mktemp -d "${TARGET_APP_PARENT_DIR}/.${APP_NAME}.tmp.XXXXXX")"
BACKUP_APP_DIR=""
cleanup_temp_dir() {
	if [[ -n "${TEMP_APP_DIR:-}" && -d "$TEMP_APP_DIR" ]]; then
		rm -rf "$TEMP_APP_DIR"
	fi

	if [[ -n "${BACKUP_APP_DIR:-}" && -d "$BACKUP_APP_DIR" ]]; then
		rm -rf "$BACKUP_APP_DIR"
	fi
}
trap cleanup_temp_dir EXIT

cp -R "$SOURCE_APP_DIR" "$TEMP_APP_DIR/$APP_NAME"

if [[ -d "$TARGET_APP_DIR" ]]; then
	BACKUP_APP_DIR="$(mktemp -d "${TARGET_APP_PARENT_DIR}/.${APP_NAME}.backup.XXXXXX")"
	rmdir "$BACKUP_APP_DIR"
	mv "$TARGET_APP_DIR" "$BACKUP_APP_DIR"
fi

if ! mv "$TEMP_APP_DIR/$APP_NAME" "$TARGET_APP_DIR"; then
	if [[ -n "${BACKUP_APP_DIR:-}" && -d "$BACKUP_APP_DIR" ]]; then
		mv "$BACKUP_APP_DIR" "$TARGET_APP_DIR" || true
		BACKUP_APP_DIR=""
	fi
	show_error "复制 ${APP_DISPLAY_NAME} 到 /Applications 失败，已恢复原有安装。请检查磁盘空间和目录写权限后重试。"
	exit 1
fi

if [[ -n "${BACKUP_APP_DIR:-}" && -d "$BACKUP_APP_DIR" ]]; then
	rm -rf "$BACKUP_APP_DIR"
	BACKUP_APP_DIR=""
fi

rmdir "$TEMP_APP_DIR"
TEMP_APP_DIR=""

xattr -dr com.apple.quarantine "$TARGET_APP_DIR" || true
xattr -cr "$TARGET_APP_DIR" || true

open "$TARGET_APP_DIR"

show_info "$(describe_install_action "$SOURCE_VERSION" "$TARGET_VERSION" "$VERSION_COMPARE_RESULT")\\n\\n${APP_DISPLAY_NAME} 已复制到 /Applications，并已尝试清理隔离属性。若系统仍提示无法打开，请前往“系统设置 -> 隐私与安全性”中选择“仍要打开”。"
