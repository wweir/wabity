export const RELEASE_CHECK_TIMEOUT_MS = 8000;

export interface GithubReleaseSummary {
	tag_name: string;
	html_url: string;
}

export interface ResolvedReleaseUpdate {
	status: "up-to-date" | "available" | "error";
	latestVersion: string;
	releaseUrl: string;
}

interface ParsedSemanticVersion {
	major: number;
	minor: number;
	patch: number;
	prerelease: string[];
}

const SEMVER_PATTERN =
	/^v?(?<major>0|[1-9]\d*)\.(?<minor>0|[1-9]\d*)\.(?<patch>0|[1-9]\d*)(?:-(?<prerelease>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/u;

export function normalizeReleaseVersion(version: string): string {
	return version.trim().replace(/^v/u, "");
}

export function compareSemanticVersions(left: string, right: string): number | null {
	const parsedLeft = parseSemanticVersion(left);
	const parsedRight = parseSemanticVersion(right);
	if (!parsedLeft || !parsedRight) {
		return null;
	}

	if (parsedLeft.major !== parsedRight.major) {
		return parsedLeft.major - parsedRight.major;
	}

	if (parsedLeft.minor !== parsedRight.minor) {
		return parsedLeft.minor - parsedRight.minor;
	}

	if (parsedLeft.patch !== parsedRight.patch) {
		return parsedLeft.patch - parsedRight.patch;
	}

	return comparePrerelease(parsedLeft.prerelease, parsedRight.prerelease);
}

export function resolveReleaseUpdate(
	currentVersion: string,
	release: GithubReleaseSummary,
	fallbackReleaseUrl: string,
): ResolvedReleaseUpdate {
	const latestVersion = normalizeReleaseVersion(release.tag_name);
	const releaseUrl = release.html_url || fallbackReleaseUrl;
	const comparison = compareSemanticVersions(currentVersion, latestVersion);

	if (comparison === null) {
		return {
			status: "error",
			latestVersion,
			releaseUrl,
		};
	}

	return {
		status: comparison < 0 ? "available" : "up-to-date",
		latestVersion,
		releaseUrl,
	};
}

function parseSemanticVersion(version: string): ParsedSemanticVersion | null {
	const normalized = normalizeReleaseVersion(version);
	const match = normalized.match(SEMVER_PATTERN);
	if (!match?.groups) {
		return null;
	}

	return {
		major: Number(match.groups.major),
		minor: Number(match.groups.minor),
		patch: Number(match.groups.patch),
		prerelease: match.groups.prerelease ? match.groups.prerelease.split(".") : [],
	};
}

function comparePrerelease(left: string[], right: string[]): number {
	if (left.length === 0 && right.length === 0) {
		return 0;
	}

	if (left.length === 0) {
		return 1;
	}

	if (right.length === 0) {
		return -1;
	}

	const maxLength = Math.max(left.length, right.length);
	for (let index = 0; index < maxLength; index += 1) {
		const leftIdentifier = left[index];
		const rightIdentifier = right[index];
		if (leftIdentifier === undefined) {
			return -1;
		}
		if (rightIdentifier === undefined) {
			return 1;
		}

		const comparison = comparePrereleaseIdentifier(leftIdentifier, rightIdentifier);
		if (comparison !== 0) {
			return comparison;
		}
	}

	return 0;
}

function comparePrereleaseIdentifier(left: string, right: string): number {
	const leftIsNumeric = /^\d+$/u.test(left);
	const rightIsNumeric = /^\d+$/u.test(right);

	if (leftIsNumeric && rightIsNumeric) {
		return Number(left) - Number(right);
	}

	if (leftIsNumeric) {
		return -1;
	}

	if (rightIsNumeric) {
		return 1;
	}

	if (left === right) {
		return 0;
	}

	return left < right ? -1 : 1;
}
