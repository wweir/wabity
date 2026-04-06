import { lazy, Suspense, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LauncherPage } from "../features/launcher/LauncherPage";
import {
	getAppSettings,
	getShortcutRuntimeStatus,
	onOcrTranslationStarted,
	onRevealLauncherMainPanel,
	onShortcutRuntimeStatusChanged,
} from "../lib/tauri/client";
import type { ShortcutRuntimeStatus } from "../lib/tauri/types";
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
	const [shortcutRuntimeStatus, setShortcutRuntimeStatus] = useState<ShortcutRuntimeStatus>({
		toggle_launcher: { configuredShortcut: "Alt+Space", registered: true, message: null },
		ocr_translate: { configuredShortcut: "Alt+D", registered: true, message: null },
		open_clipboard_history: { configuredShortcut: "Alt+V", registered: true, message: null },
	});
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
		let active = true;
		let unlistenShortcutRuntime: (() => void) | null = null;

		void getShortcutRuntimeStatus()
			.then((status) => {
				if (active) {
					setShortcutRuntimeStatus(status);
				}
			})
			.catch(() => {});

		void onShortcutRuntimeStatusChanged((status) => {
			setShortcutRuntimeStatus(status);
		})
			.then((unlisten) => {
				if (!unlisten) {
					return;
				}

				if (!active) {
					unlisten();
					return;
				}

				unlistenShortcutRuntime = unlisten;
			})
			.catch((error: unknown) => {
				console.warn("failed to subscribe shortcut runtime status event in App", error);
			});

		return () => {
			active = false;
			unlistenShortcutRuntime?.();
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
		return <LauncherPage shortcutRuntimeStatus={shortcutRuntimeStatus} windowKind={windowKind} />;
	}

	return (
		<>
			<div hidden={!launcherVisible}>
				<LauncherPage
					active={launcherVisible}
					shortcutRuntimeStatus={shortcutRuntimeStatus}
					windowKind={windowKind}
					onOpenSettings={() => setCurrentView("settings")}
				/>
			</div>
			{currentView === "settings" ? (
				<Suspense fallback={<div className="app-loading-state">加载设置中...</div>}>
					<SettingsPage
						onAppearanceChange={setAppearanceSettings}
						onBack={() => setCurrentView("launcher")}
						shortcutRuntimeStatus={shortcutRuntimeStatus}
					/>
				</Suspense>
			) : null}
		</>
	);
}
