#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const bundleScriptPath = process.argv[2];

if (!bundleScriptPath) {
	console.error("usage: patch-bundle-dmg.mjs <bundle_dmg.sh>");
	process.exit(1);
}

const absolutePath = path.resolve(bundleScriptPath);
const source = fs.readFileSync(absolutePath, "utf8");
const patchMarker = "# wabity-dmg-tolerance-patch";
const backgroundMarker = "# wabity-dmg-background-v1-patch";
const legacyLayoutMarker = "# wabity-dmg-extra-files-patch";
const previousLayoutMarker = "# wabity-dmg-layout-patch";
const layoutMarker = "# wabity-dmg-layout-v2-patch";
const previousVolumeIconMarker = "# wabity-dmg-volume-icon-patch";
const volumeIconMarker = "# wabity-dmg-volume-icon-v2-patch";

const hasRetryPatch = source.includes(patchMarker);
const hasBackgroundPatch = source.includes(backgroundMarker);
const hasLegacyLayoutPatch = source.includes(legacyLayoutMarker);
const hasPreviousLayoutPatch = source.includes(previousLayoutMarker);
const hasLayoutPatch = source.includes(layoutMarker);
const hasPreviousVolumeIconPatch = source.includes(previousVolumeIconMarker);
const hasVolumeIconPatch = source.includes(volumeIconMarker);

if (hasRetryPatch && hasBackgroundPatch && hasLayoutPatch && hasVolumeIconPatch) {
	process.exit(0);
}

const helperNeedle = "\n# Argument parsing\n";
const helperPatch = `
${patchMarker}
function run_applescript_with_retry() {
\tlocal script_file="$1"
\tlocal volume_name="$2"
\tlocal max_attempts=5
\tlocal attempt=1
\tlocal sleep_seconds=2

\twhile (( attempt <= max_attempts )); do
\t\tif /usr/bin/osascript "$script_file" "$volume_name"; then
\t\t\treturn 0
\t\tfi

\t\tlocal exit_code=$?
\t\techo >&2 "AppleScript attempt $attempt/$max_attempts failed with exit code $exit_code."
\t\tif (( attempt == max_attempts )); then
\t\t\treturn "$exit_code"
\t\tfi

\t\techo >&2 "Retrying AppleScript after \${sleep_seconds}s..."
\t\tsleep "$sleep_seconds"
\t\tsleep_seconds=$(( sleep_seconds + 2 ))
\t\t(( attempt++ ))
\tdone
}

# Argument parsing
`;

if (!hasRetryPatch && !source.includes(helperNeedle)) {
	console.error(`failed to find helper insertion point in ${absolutePath}`);
	process.exit(1);
}

const retryPatch = `\t\techo "Running AppleScript to make Finder stuff pretty: /usr/bin/osascript "\${APPLESCRIPT_FILE}" "\${VOLUME_NAME}""
\t\tif run_applescript_with_retry "\${APPLESCRIPT_FILE}" "\${VOLUME_NAME}"; then
\t\t\techo "Done running the AppleScript..."
\t\telse
\t\t\techo >&2 "Warning: Finder AppleScript failed after retries; continuing without DMG prettification"
\t\tfi`;
const retryBlockPattern =
	/\t\techo "Running AppleScript to make Finder stuff pretty:[^\n]*"\n\t\tif \/usr\/bin\/osascript "\$\{APPLESCRIPT_FILE\}" "\$\{VOLUME_NAME\}"; then\n\t\t\t# Okay, we're cool\n\t\t\ttrue\n\t\telse\n\t\t\techo >&2 "Failed running AppleScript"\n\t\t\thdiutil_detach_retry "\$\{DEV_NAME\}"\n\t\t\texit 64\n\t\tfi\n\t\techo "Done running the AppleScript\.\.\."/;

if (!hasRetryPatch && !retryBlockPattern.test(source)) {
	console.error(`failed to find AppleScript retry block in ${absolutePath}`);
	process.exit(1);
}

const backgroundNeedle = `if [[ -n "$BACKGROUND_FILE" ]]; then
`;

const backgroundPatch = `${backgroundMarker}
WABITY_BACKGROUND_FILE="$SCRIPT_DIR/../../../dmg-background.png"

if [[ -z "$BACKGROUND_FILE" && -f "$WABITY_BACKGROUND_FILE" ]]; then
\tBACKGROUND_FILE="$WABITY_BACKGROUND_FILE"
\tBACKGROUND_FILE_NAME="$(basename "$BACKGROUND_FILE")"
\tBACKGROUND_CLAUSE="set background picture of opts to file \\".background:$BACKGROUND_FILE_NAME\\""
\tREPOSITION_HIDDEN_FILES_CLAUSE="set position of every item to {theBottomRightX + 220, 220}"
fi

if [[ -n "$BACKGROUND_FILE" ]]; then
`;

if (!hasBackgroundPatch && !source.includes(backgroundNeedle)) {
	console.error(`failed to find background insertion point in ${absolutePath}`);
	process.exit(1);
}

const extraFilesNeedle = `if [[ -n "$ADD_FILE_SOURCES" ]]; then
\techo "Copying custom files..."
\tfor i in "\${!ADD_FILE_SOURCES[@]}"; do
\t\techo "\${ADD_FILE_SOURCES[$i]}"
\t\tcp -a "\${ADD_FILE_SOURCES[$i]}" "$MOUNT_DIR/\${ADD_FILE_TARGETS[$i]}"
\tdone
fi

VOLUME_NAME=$(basename $MOUNT_DIR)
`;

const legacyExtraFilesPattern =
	/if \[\[ -n "\$ADD_FILE_SOURCES" \]\]; then[\s\S]*?# wabity-dmg-extra-files-patch[\s\S]*?VOLUME_NAME=\$\(basename \$MOUNT_DIR\)\n/;
const previousLayoutPattern =
	/if \[\[ -n "\$ADD_FILE_SOURCES" \]\]; then[\s\S]*?# wabity-dmg-layout-patch[\s\S]*?VOLUME_NAME=\$\(basename \$MOUNT_DIR\)\n/;

const extraFilesPatch = `if [[ -n "$ADD_FILE_SOURCES" ]]; then
\techo "Copying custom files..."
\tfor i in "\${!ADD_FILE_SOURCES[@]}"; do
\t\techo "\${ADD_FILE_SOURCES[$i]}"
\t\tcp -a "\${ADD_FILE_SOURCES[$i]}" "$MOUNT_DIR/\${ADD_FILE_TARGETS[$i]}"
\tdone
fi

${layoutMarker}
append_wabity_position_clause() {
\tlocal target_name="$1"
\tlocal position_x="$2"
\tlocal position_y="$3"

\tPOSITION_CLAUSE="\${POSITION_CLAUSE}set position of item \\"$target_name\\" to {$position_x, $position_y}
\t\t\t"
}

append_wabity_hidden_extension_clause() {
\tlocal target_name="$1"

\tHIDING_CLAUSE="\${HIDING_CLAUSE}set the extension hidden of item \\"$target_name\\" to true
\t\t\t"
}

copy_wabity_extra_file() {
\tlocal source_path="$1"
\tlocal target_name="$2"
\tlocal position_x="$3"
\tlocal position_y="$4"
\tlocal hide_extension="$5"

\tif [[ -f "$source_path" ]]; then
\t\techo "Copying Wabity DMG helper file '$target_name'..."
\t\tcp -a "$source_path" "$MOUNT_DIR/$target_name"
\t\tappend_wabity_position_clause "$target_name" "$position_x" "$position_y"
\t\tif [[ "$hide_extension" -eq 1 ]]; then
\t\t\tappend_wabity_hidden_extension_clause "$target_name"
\t\tfi
\telse
\t\techo >&2 "Warning: missing Wabity DMG helper file: $source_path"
\tfi
}

if [[ "$ICON_SIZE" == "128" ]]; then
\tICON_SIZE=96
fi

if [[ "$TEXT_SIZE" == "16" ]]; then
\tTEXT_SIZE=13
fi

append_wabity_hidden_extension_clause "Wabity.app"
copy_wabity_extra_file "$SCRIPT_DIR/../../../../../scripts/dmg-install-and-repair.command" "修复.command" 112 292 1

VOLUME_NAME=$(basename $MOUNT_DIR)
`;

if (
	!source.includes(extraFilesNeedle) &&
	!hasLegacyLayoutPatch &&
	!hasPreviousLayoutPatch &&
	!hasLayoutPatch
) {
	console.error(`failed to find extra files insertion point in ${absolutePath}`);
	process.exit(1);
}

const volumeIconNeedle = `if [[ -n "$VOLUME_ICON_FILE" ]]; then
\techo "Copying volume icon file '$VOLUME_ICON_FILE'..."
\tcp "$VOLUME_ICON_FILE" "$MOUNT_DIR/.VolumeIcon.icns"
\tSetFile -c icnC "$MOUNT_DIR/.VolumeIcon.icns"
fi
`;

const volumeIconPatch = `${volumeIconMarker}
run_wabity_finder_position_script() {
\tlocal target_name="$1"
\tlocal target_x="$2"
\tlocal target_y="$3"
\tlocal volume_name
\tvolume_name=$(basename "$MOUNT_DIR")

\t/usr/bin/osascript \\
\t\t-e "tell application \\"Finder\\"" \\
\t\t-e "tell disk \\"$volume_name\\"" \\
\t\t-e "if exists item \\"$target_name\\" then set position of item \\"$target_name\\" to {$target_x, $target_y}" \\
\t\t-e "end tell" \\
\t\t-e "end tell" >/dev/null 2>&1 || true
}

apply_wabity_post_layout_positions() {
\trun_wabity_finder_position_script "Wabity.app" 186 150
\trun_wabity_finder_position_script "Applications" 534 150
\trun_wabity_finder_position_script "修复.command" 122 292
\trun_wabity_finder_position_script ".VolumeIcon.icns" $(( WINW + 180 )) $(( WINH + 180 ))
}

copy_wabity_volume_icon_file() {
\tif [[ -n "$VOLUME_ICON_FILE" ]]; then
\t\techo "Copying volume icon file '$VOLUME_ICON_FILE' after Finder layout..."
\t\tcp "$VOLUME_ICON_FILE" "$MOUNT_DIR/.VolumeIcon.icns"
\t\tSetFile -a V "$MOUNT_DIR/.VolumeIcon.icns"
\t\tSetFile -c icnC "$MOUNT_DIR/.VolumeIcon.icns"
\tfi
}
`;

const volumeIconCallNeedle = `# Make sure it's not world writeable
`;

const volumeIconCallPatch = `copy_wabity_volume_icon_file
apply_wabity_post_layout_positions

# Make sure it's not world writeable
`;
const previousVolumeIconPattern =
	/# wabity-dmg-volume-icon-patch[\s\S]*?copy_wabity_volume_icon_file\napply_wabity_post_layout_positions\n\n# Make sure it's not world writeable\n/;

if (!source.includes(volumeIconNeedle) && !hasVolumeIconPatch) {
	console.error(`failed to find volume icon insertion point in ${absolutePath}`);
	process.exit(1);
}

if (!source.includes(volumeIconCallNeedle) && !hasVolumeIconPatch) {
	console.error(`failed to find volume icon call insertion point in ${absolutePath}`);
	process.exit(1);
}

let patched = source;

if (!hasRetryPatch) {
	patched = patched.replace(helperNeedle, helperPatch).replace(retryBlockPattern, retryPatch);
}

if (!hasBackgroundPatch) {
	patched = patched.replace(backgroundNeedle, backgroundPatch);
}

if (!hasLayoutPatch) {
	if (hasLegacyLayoutPatch) {
		patched = patched.replace(legacyExtraFilesPattern, extraFilesPatch);
	} else if (hasPreviousLayoutPatch) {
		patched = patched.replace(previousLayoutPattern, extraFilesPatch);
	} else {
		patched = patched.replace(extraFilesNeedle, extraFilesPatch);
	}
}

if (!hasVolumeIconPatch) {
	if (hasPreviousVolumeIconPatch) {
		patched = patched.replace(
			previousVolumeIconPattern,
			`${volumeIconPatch}${volumeIconCallPatch}`,
		);
	} else {
		patched = patched
			.replace(volumeIconNeedle, volumeIconPatch)
			.replace(volumeIconCallNeedle, volumeIconCallPatch);
	}
}

fs.writeFileSync(absolutePath, patched);
console.error(`patched ${absolutePath}`);
