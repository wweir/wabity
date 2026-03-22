import { lazy, Suspense, useEffect, useState } from "react";
import { LauncherPage } from "../features/launcher/LauncherPage";
import { getAppSettings } from "../lib/tauri/client";
import type { AppearanceSettings } from "../lib/tauri/types";
import { applyAppearanceSettings, defaultAppearanceSettings } from "./appearance";

const SettingsPage = lazy(() =>
	import("../features/settings/SettingsPage").then((module) => ({
		default: module.SettingsPage,
	})),
);

export type AppView = "launcher" | "settings";

export function App() {
	const [currentView, setCurrentView] = useState<AppView>("launcher");
	const [appearanceSettings, setAppearanceSettings] =
		useState<AppearanceSettings>(defaultAppearanceSettings);

	useEffect(() => {
		let removeMediaListener: (() => void) | null = null;

		const bindSystemThemeListener = (themePreference: string) => {
			removeMediaListener?.();
			removeMediaListener = null;

			if (themePreference !== "auto" || typeof window.matchMedia !== "function") {
				return;
			}

			const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
			const handleChange = () => {
				const fontSize =
					document.documentElement.dataset.fontSize ?? defaultAppearanceSettings.fontSize;
				applyAppearanceSettings({
					theme: "auto",
					fontSize,
				});
			};

			if (typeof mediaQuery.addEventListener === "function") {
				mediaQuery.addEventListener("change", handleChange);
				removeMediaListener = () => mediaQuery.removeEventListener("change", handleChange);
				return;
			}

			mediaQuery.addListener(handleChange);
			removeMediaListener = () => mediaQuery.removeListener(handleChange);
		};

		applyAppearanceSettings(appearanceSettings);
		bindSystemThemeListener(appearanceSettings.theme);

		return () => {
			removeMediaListener?.();
		};
	}, [appearanceSettings]);

	useEffect(() => {
		let cancelled = false;

		void getAppSettings()
			.then((settings) => {
				if (!cancelled) {
					setAppearanceSettings(settings.appearance);
				}
			})
			.catch(() => {
				if (!cancelled) {
					setAppearanceSettings(defaultAppearanceSettings);
				}
			});

		return () => {
			cancelled = true;
		};
	}, []);

	if (currentView === "settings") {
		return (
			<Suspense fallback={<div className="app-loading-state">加载设置中...</div>}>
				<SettingsPage
					onAppearanceChange={setAppearanceSettings}
					onBack={() => setCurrentView("launcher")}
				/>
			</Suspense>
		);
	}

	return <LauncherPage onOpenSettings={() => setCurrentView("settings")} />;
}
