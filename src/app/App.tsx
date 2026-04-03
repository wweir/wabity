import { lazy, Suspense, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LauncherPage } from "../features/launcher/LauncherPage";
import {
	getAppSettings,
	onOcrTranslationStarted,
	onRevealLauncherMainPanel,
} from "../lib/tauri/client";
import { defaultAppearanceSettings, syncAppearanceSettings } from "./appearance";

const SettingsPage = lazy(() =>
	import("../features/settings/SettingsPage").then((module) => ({
		default: module.SettingsPage,
	})),
);

export type AppView = "launcher" | "settings";
export type AppWindowKind = "main" | "clipboard_history";

function resolveInitialWindowKind(): AppWindowKind {
	try {
		return getCurrentWindow().label === "clipboard" ? "clipboard_history" : "main";
	} catch {
		return "main";
	}
}

export function App() {
	const [currentView, setCurrentView] = useState<AppView>("launcher");
	const [windowKind] = useState<AppWindowKind>(resolveInitialWindowKind);
	const [appearanceSettings, setAppearanceSettings] = useState(defaultAppearanceSettings);
	const launcherVisible = currentView === "launcher";

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

	useEffect(() => {
		if (windowKind !== "main") {
			return;
		}

		let active = true;
		let unlistenOcrStarted: (() => void) | null = null;
		let unlistenRevealMainPanel: (() => void) | null = null;

		void onOcrTranslationStarted(() => {
			setCurrentView("launcher");
		})
			.then((unlisten) => {
				if (!unlisten) {
					return;
				}

				if (!active) {
					unlisten();
					return;
				}

				unlistenOcrStarted = unlisten;
			})
			.catch((error: unknown) => {
				console.warn("failed to subscribe OCR started event in App", error);
			});

		void onRevealLauncherMainPanel(() => {
			setCurrentView("launcher");
		})
			.then((unlisten) => {
				if (!unlisten) {
					return;
				}

				if (!active) {
					unlisten();
					return;
				}

				unlistenRevealMainPanel = unlisten;
			})
			.catch((error: unknown) => {
				console.warn("failed to subscribe launcher reveal event in App", error);
			});

		return () => {
			active = false;
			unlistenOcrStarted?.();
			unlistenRevealMainPanel?.();
		};
	}, [windowKind]);

	if (windowKind === "clipboard_history") {
		return <LauncherPage windowKind={windowKind} />;
	}

	return (
		<>
			<div hidden={!launcherVisible}>
				<LauncherPage
					active={launcherVisible}
					windowKind={windowKind}
					onOpenSettings={() => setCurrentView("settings")}
				/>
			</div>
			{currentView === "settings" ? (
				<Suspense fallback={<div className="app-loading-state">加载设置中...</div>}>
					<SettingsPage
						onAppearanceChange={setAppearanceSettings}
						onBack={() => setCurrentView("launcher")}
					/>
				</Suspense>
			) : null}
		</>
	);
}
