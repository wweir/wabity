import test from "node:test";
import assert from "node:assert/strict";
import {
	compareSemanticVersions,
	resolveReleaseUpdate,
} from "../src/features/settings/sections/aboutUpdate.ts";

test("treats a lower release as up to date instead of update available", () => {
	const result = resolveReleaseUpdate(
		"0.3.0",
		{
			tag_name: "v0.2.0",
			html_url: "https://example.com/releases/v0.2.0",
		},
		"https://example.com/releases/latest",
	);

	assert.equal(result.status, "up-to-date");
	assert.equal(result.latestVersion, "0.2.0");
});

test("treats a stable release as newer than a matching prerelease", () => {
	const result = resolveReleaseUpdate(
		"0.2.0-beta.1",
		{
			tag_name: "v0.2.0",
			html_url: "https://example.com/releases/v0.2.0",
		},
		"https://example.com/releases/latest",
	);

	assert.equal(result.status, "available");
	assert.equal(result.latestVersion, "0.2.0");
});

test("returns error when release tag is not a semantic version", () => {
	const result = resolveReleaseUpdate(
		"0.2.0",
		{
			tag_name: "release-2026-04-11",
			html_url: "",
		},
		"https://example.com/releases/latest",
	);

	assert.equal(result.status, "error");
	assert.equal(result.releaseUrl, "https://example.com/releases/latest");
});

test("compares prerelease identifiers with semver precedence", () => {
	assert.ok(compareSemanticVersions("1.0.0-beta.2", "1.0.0-beta.11") < 0);
	assert.ok(compareSemanticVersions("1.0.0-rc.1", "1.0.0") < 0);
	assert.ok(compareSemanticVersions("1.2.0", "1.1.9") > 0);
	assert.ok(compareSemanticVersions("1.0.0-alpha", "1.0.0-Alpha") > 0);
});
