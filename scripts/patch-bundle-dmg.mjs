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

if (source.includes(patchMarker)) {
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

if (!source.includes(helperNeedle)) {
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

if (!retryBlockPattern.test(source)) {
	console.error(`failed to find AppleScript retry block in ${absolutePath}`);
	process.exit(1);
}

const patched = source.replace(helperNeedle, helperPatch).replace(retryBlockPattern, retryPatch);

fs.writeFileSync(absolutePath, patched);
console.error(`patched ${absolutePath}`);
