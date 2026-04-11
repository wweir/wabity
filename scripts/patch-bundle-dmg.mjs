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
const extrasMarker = "# wabity-dmg-extra-files-patch";
const volumeIconMarker = "# wabity-dmg-volume-icon-patch";

const hasRetryPatch = source.includes(patchMarker);
const hasExtrasPatch = source.includes(extrasMarker);
const hasVolumeIconPatch = source.includes(volumeIconMarker);

if (hasRetryPatch && hasExtrasPatch && hasVolumeIconPatch) {
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

const extraFilesNeedle = `if [[ -n "$ADD_FILE_SOURCES" ]]; then
\techo "Copying custom files..."
\tfor i in "\${!ADD_FILE_SOURCES[@]}"; do
\t\techo "\${ADD_FILE_SOURCES[$i]}"
\t\tcp -a "\${ADD_FILE_SOURCES[$i]}" "$MOUNT_DIR/\${ADD_FILE_TARGETS[$i]}"
\tdone
fi

VOLUME_NAME=$(basename $MOUNT_DIR)
`;

const extraFilesPatch = `if [[ -n "$ADD_FILE_SOURCES" ]]; then
\techo "Copying custom files..."
\tfor i in "\${!ADD_FILE_SOURCES[@]}"; do
\t\techo "\${ADD_FILE_SOURCES[$i]}"
\t\tcp -a "\${ADD_FILE_SOURCES[$i]}" "$MOUNT_DIR/\${ADD_FILE_TARGETS[$i]}"
\tdone
fi

${extrasMarker}
copy_wabity_extra_file() {
\tlocal source_path="$1"
\tlocal target_name="$2"

\tif [[ -f "$source_path" ]]; then
\t\techo "Copying Wabity DMG helper file '$target_name'..."
\t\tcp -a "$source_path" "$MOUNT_DIR/$target_name"
\telse
\t\techo >&2 "Warning: missing Wabity DMG helper file: $source_path"
\tfi
}

copy_wabity_extra_file "$SCRIPT_DIR/../../../../../scripts/dmg-install-and-repair.command" "安装并修复.command"
copy_wabity_extra_file "$SCRIPT_DIR/../../../../../scripts/DMG-首次打开说明.txt" "首次打开说明.txt"

VOLUME_NAME=$(basename $MOUNT_DIR)
`;

if (!source.includes(extraFilesNeedle) && !hasExtrasPatch) {
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

# Make sure it's not world writeable
`;

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

if (!hasExtrasPatch) {
	patched = patched.replace(extraFilesNeedle, extraFilesPatch);
}

if (!hasVolumeIconPatch) {
	patched = patched
		.replace(volumeIconNeedle, volumeIconPatch)
		.replace(volumeIconCallNeedle, volumeIconCallPatch);
}

fs.writeFileSync(absolutePath, patched);
console.error(`patched ${absolutePath}`);
