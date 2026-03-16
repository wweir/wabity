import { lazy, Suspense, useState } from "react";
import { LauncherPage } from "../features/launcher/LauncherPage";

const SettingsPage = lazy(() =>
	import("../features/settings/SettingsPage").then((module) => ({
		default: module.SettingsPage,
	})),
);

export type AppView = "launcher" | "settings";

export function App() {
	const [currentView, setCurrentView] = useState<AppView>("launcher");

	if (currentView === "settings") {
		return (
			<Suspense fallback={<div className="app-loading-state">加载设置中...</div>}>
				<SettingsPage onBack={() => setCurrentView("launcher")} />
			</Suspense>
		);
	}

	return <LauncherPage onOpenSettings={() => setCurrentView("settings")} />;
}
