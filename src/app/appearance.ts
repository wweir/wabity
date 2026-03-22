import type { AppearanceSettings } from "../lib/tauri/types";

export const defaultAppearanceSettings: AppearanceSettings = {
	theme: "auto",
	fontSize: "medium",
};

type ResolvedTheme = "light" | "dark";

function resolveTheme(theme: string): ResolvedTheme {
	if (
		theme === "auto" &&
		typeof window !== "undefined" &&
		typeof window.matchMedia === "function"
	) {
		return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
	}

	return theme === "dark" ? "dark" : "light";
}

function normalizeFontSize(fontSize: string) {
	switch (fontSize) {
		case "small":
		case "large":
			return fontSize;
		default:
			return "medium";
	}
}

export function applyAppearanceSettings(appearance: AppearanceSettings) {
	if (typeof document === "undefined") {
		return;
	}

	const root = document.documentElement;
	const resolvedTheme = resolveTheme(appearance.theme);
	const normalizedFontSize = normalizeFontSize(appearance.fontSize);

	root.dataset.theme = resolvedTheme;
	root.dataset.themePreference = appearance.theme;
	root.dataset.fontSize = normalizedFontSize;
	root.style.colorScheme = resolvedTheme;
}
