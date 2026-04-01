import { lazy, Suspense, useEffect, useState } from "react";
import { LauncherPage } from "../features/launcher/LauncherPage";
import { getAppSettings } from "../lib/tauri/client";
import { defaultAppearanceSettings, syncAppearanceSettings } from "./appearance";

const SettingsPage = lazy(() =>
	import("../features/settings/SettingsPage").then((module) => ({
		default: module.SettingsPage,
	})),
);

export type AppView = "launcher" | "settings";

export function App() {
	const [currentView, setCurrentView] = useState<AppView>("launcher");
	const [appearanceSettings, setAppearanceSettings] = useState(defaultAppearanceSettings);

	useEffect(() => {
		return syncAppearanceSettings(appearanceSettings);
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
