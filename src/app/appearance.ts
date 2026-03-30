import type { AppearanceSettings } from "../lib/tauri/types";

export const defaultAppearanceSettings: AppearanceSettings = {
	theme: "auto",
	fontSize: "medium",
};

const systemThemeMediaQuery = "(prefers-color-scheme: dark)";
const noop = () => {};

type ResolvedTheme = "light" | "dark";

function resolveTheme(theme: string): ResolvedTheme {
	if (
		theme === "auto" &&
		typeof window !== "undefined" &&
		typeof window.matchMedia === "function"
	) {
		return window.matchMedia(systemThemeMediaQuery).matches ? "dark" : "light";
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

export function syncAppearanceSettings(appearance: AppearanceSettings) {
	applyAppearanceSettings(appearance);

	if (appearance.theme !== "auto" || typeof window === "undefined") {
		return noop;
	}

	if (typeof window.matchMedia !== "function") {
		return noop;
	}

	const mediaQuery = window.matchMedia(systemThemeMediaQuery);
	const handleChange = () => {
		applyAppearanceSettings(appearance);
	};

	if (typeof mediaQuery.addEventListener === "function") {
		mediaQuery.addEventListener("change", handleChange);
		return () => mediaQuery.removeEventListener("change", handleChange);
	}

	mediaQuery.addListener(handleChange);
	return () => mediaQuery.removeListener(handleChange);
}
