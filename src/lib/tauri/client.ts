import { invoke, isTauri, Channel } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
	ActionMatch,
	ExecutionRequest,
	ExecutionResult,
	FileSearchMatch,
	InstalledAppMatch,
	QueryPayload,
} from "../../features/launcher/types";
import type {
	AcpAgentCatalog,
	AcpAgentConfig,
	AcpMcpServerCatalog,
	AcpMcpServerConfig,
	AcpRestoreNotice,
	AcpSessionDetail,
	AcpSessionSummary,
	AppSettings,
	LlmProviderConfig,
	LlmSettings,
	PublicSkillCatalog,
	RagScanResult,
	RagSettings,
	ShortcutConfig,
	WorkspaceState,
} from "./types";
import {
	executeActionFallback,
	launchAppFallback,
	matchActionsFallback,
	searchAppsFallback,
} from "../../features/launcher/fallback";

const browserShortcutConfig: ShortcutConfig = {
	toggle_launcher: "Cmd+Shift+Space",
	ocr_capture: "Cmd+Shift+O",
};

const browserAppSettings: AppSettings = {
	general: {
		autoStart: false,
		showInDock: true,
		language: "zh-CN",
	},
	appearance: {
		theme: "auto",
		fontSize: "medium",
	},
	llm: {
		providers: [],
		defaultProviderId: null,
	},
	ocr: {
		provider: "system",
		llmProviderId: null,
	},
	rag: {
		sourceDirectories: [],
		ignoreGlobs: [],
		embeddingProviderId: null,
	},
};

const browserPublicSkillCatalog: PublicSkillCatalog = {
	rootPath: "~/.agents/skills",
	exists: false,
	skills: [],
};

const browserWorkspaceState: WorkspaceState = {
	rootPath: "/",
	recentRoots: ["/"],
	homePath: null,
	displayHomeAsTilde: false,
};

function canUseTauriInvoke() {
	if (!isTauri()) {
		return false;
	}

	const tauriInternals = (globalThis as { __TAURI_INTERNALS__?: { invoke?: unknown } })
		.__TAURI_INTERNALS__;
	return typeof tauriInternals?.invoke === "function";
}

export function isDesktopRuntimeAvailable() {
	return canUseTauriInvoke();
}

async function invokeDesktop<T>(command: string, args?: Record<string, unknown>): Promise<T> {
	if (args) {
		return invoke<T>(command, args);
	}

	return invoke<T>(command);
}

async function invokeOrDefault<T>(
	command: string,
	fallbackValue: T | (() => T),
	args?: Record<string, unknown>,
): Promise<T> {
	if (!canUseTauriInvoke()) {
		return typeof fallbackValue === "function" ? (fallbackValue as () => T)() : fallbackValue;
	}

	return invokeDesktop<T>(command, args);
}

async function invokeIfDesktop(command: string, args?: Record<string, unknown>): Promise<void> {
	if (!canUseTauriInvoke()) {
		return;
	}

	await invokeDesktop<void>(command, args);
}

async function listenIfDesktop<T>(
	eventName: string,
	callback: (payload: T) => void,
): Promise<UnlistenFn | null> {
	if (!canUseTauriInvoke()) {
		return null;
	}

	return listen<T>(eventName, (event) => {
		callback(event.payload);
	});
}

function buildBrowserSessionDetail(sessionId: string): AcpSessionDetail {
	return {
		session: {
			sessionId,
			workspaceRoot: "",
			title: "session",
			agentId: null,
			agentName: "agent",
			status: "running",
			errorLevel: null,
			attention: false,
			isActive: true,
			lastError: null,
			lastUpdatedAtMs: Date.now(),
		},
		messages: [],
	};
}

export async function matchActions(query: QueryPayload): Promise<ActionMatch[]> {
	return invokeOrDefault("match_actions", () => matchActionsFallback(query), { query });
}

export async function searchFiles(query: string, limit = 8): Promise<FileSearchMatch[]> {
	return invokeOrDefault("search_files", [], { query, limit });
}

export async function searchApps(query: string, limit = 8): Promise<InstalledAppMatch[]> {
	return invokeOrDefault("search_apps", () => searchAppsFallback(query), { query, limit });
}

export async function executeAction(request: ExecutionRequest): Promise<ExecutionResult> {
	return invokeOrDefault("execute_action", () => executeActionFallback(request), { request });
}

export async function launchApp(path: string): Promise<ExecutionResult> {
	return invokeOrDefault("launch_app", () => launchAppFallback(path), { path });
}

export async function hideLauncherWindow(): Promise<void> {
	return invokeIfDesktop("hide_launcher_window");
}

export async function beginTransientWindowInteraction(): Promise<void> {
	return invokeIfDesktop("begin_transient_window_interaction");
}

export async function endTransientWindowInteraction(): Promise<void> {
	return invokeIfDesktop("end_transient_window_interaction");
}

let lastLauncherWindowSize: { width: number; height: number } | null = null;

export async function resizeLauncherWindow(size: { width: number; height: number }): Promise<void> {
	if (!canUseTauriInvoke()) {
		return;
	}

	const nextWidth = Math.max(1, Math.ceil(size.width));
	const nextHeight = Math.max(1, Math.ceil(size.height));

	if (
		lastLauncherWindowSize?.width === nextWidth &&
		lastLauncherWindowSize?.height === nextHeight
	) {
		return;
	}

	lastLauncherWindowSize = { width: nextWidth, height: nextHeight };

	await invokeDesktop<void>("resize_launcher_window", { width: nextWidth, height: nextHeight });
}

export async function onSelectedText(callback: (text: string) => void): Promise<UnlistenFn | null> {
	return listenIfDesktop("selected-text", callback);
}

export async function onOcrError(callback: (message: string) => void): Promise<UnlistenFn | null> {
	return listenIfDesktop("ocr-error", callback);
}

// Shortcut configuration
export async function getShortcut(): Promise<ShortcutConfig> {
	return invokeOrDefault("get_shortcut", browserShortcutConfig);
}

export async function setShortcut(key: string, shortcut: string): Promise<void> {
	return invokeIfDesktop("set_shortcut", { key, shortcut });
}

export async function onShortcutUpdated(
	callback: (config: ShortcutConfig) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("shortcut-updated", callback);
}

export async function getAppSettings(): Promise<AppSettings> {
	return invokeOrDefault("get_app_settings", browserAppSettings);
}

export async function setAppSettings(settings: AppSettings): Promise<AppSettings> {
	return invokeOrDefault("set_app_settings", settings, { settings });
}

export async function listLlmProviderModels(provider: LlmProviderConfig): Promise<string[]> {
	return invokeOrDefault("list_llm_provider_models", [], { provider });
}

export async function scanRagSources(
	ragSettings: RagSettings,
	llmSettings: LlmSettings,
): Promise<RagScanResult> {
	return invokeOrDefault(
		"scan_rag_sources",
		{
			databasePath: "",
			sourceCount: 0,
			scannedFileCount: 0,
			indexedFileCount: 0,
			skippedFileCount: 0,
			chunkCount: 0,
			finishedAtMs: Date.now(),
		},
		{ ragSettings, llmSettings },
	);
}

export async function getPublicSkillCatalog(): Promise<PublicSkillCatalog> {
	return invokeOrDefault("get_public_skill_catalog", browserPublicSkillCatalog);
}

export async function getWorkspace(): Promise<WorkspaceState> {
	return invokeOrDefault("get_workspace", browserWorkspaceState);
}

export async function setWorkspace(rootPath: string): Promise<WorkspaceState> {
	return invokeOrDefault(
		"set_workspace",
		{ ...browserWorkspaceState, rootPath, recentRoots: [rootPath] },
		{ rootPath },
	);
}

export async function chooseDirectory(defaultPath?: string): Promise<string | null> {
	if (!isTauri()) {
		return null;
	}

	await beginTransientWindowInteraction();

	try {
		const selection = await open({
			directory: true,
			multiple: false,
			defaultPath,
		});

		return typeof selection === "string" ? selection : null;
	} finally {
		await endTransientWindowInteraction();
	}
}

export async function chooseWorkspaceDirectory(defaultPath?: string): Promise<string | null> {
	return chooseDirectory(defaultPath);
}

export async function onWorkspaceUpdated(
	callback: (workspace: WorkspaceState) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("workspace-updated", callback);
}

export async function getAcpAgents(): Promise<AcpAgentCatalog> {
	return invokeOrDefault("get_acp_agents", { agents: [], defaultAgentId: null });
}

export async function setAcpAgents(
	agents: AcpAgentConfig[],
	defaultAgentId: string | null,
): Promise<AcpAgentCatalog> {
	return invokeOrDefault("set_acp_agents", { agents, defaultAgentId }, { agents, defaultAgentId });
}

export async function getAcpMcpServers(): Promise<AcpMcpServerCatalog> {
	return invokeOrDefault("get_acp_mcp_servers", { servers: [] });
}

export async function setAcpMcpServers(
	servers: AcpMcpServerConfig[],
): Promise<AcpMcpServerCatalog> {
	return invokeOrDefault("set_acp_mcp_servers", { servers }, { servers });
}

export async function listAcpSessions(): Promise<AcpSessionSummary[]> {
	return invokeOrDefault("list_acp_sessions", []);
}

export async function takeAcpRestoreNotices(): Promise<AcpRestoreNotice[]> {
	return invokeOrDefault("take_acp_restore_notices", []);
}

export async function getAcpSessionDetail(sessionId: string): Promise<AcpSessionDetail | null> {
	return invokeOrDefault("get_acp_session_detail", null, { sessionId });
}

export async function createAcpSession(agentId: string | null = null): Promise<AcpSessionDetail> {
	return invokeOrDefault("create_acp_session", () => buildBrowserSessionDetail("browser-session"), {
		agentId,
	});
}

export async function activateAcpSession(sessionId: string | null): Promise<AcpSessionSummary[]> {
	return invokeOrDefault("activate_acp_session", [], { sessionId });
}

export async function sendAcpPrompt(sessionId: string, prompt: string): Promise<AcpSessionDetail> {
	return invokeOrDefault("send_acp_prompt", () => buildBrowserSessionDetail(sessionId), {
		sessionId,
		prompt,
	});
}

export async function cancelAcpSession(sessionId: string): Promise<void> {
	return invokeIfDesktop("cancel_acp_session", { sessionId });
}

export async function closeAcpSession(sessionId: string): Promise<void> {
	return invokeIfDesktop("close_acp_session", { sessionId });
}

export async function subscribeAcpSessionUpdates(
	callback: (detail: AcpSessionDetail) => void,
): Promise<void> {
	if (!canUseTauriInvoke()) {
		return;
	}

	const channel = new Channel<AcpSessionDetail>();
	channel.onmessage = callback;
	await invoke("subscribe_acp_session_updates", { onEvent: channel });
}

export async function subscribeAcpSessionRemovals(
	callback: (sessionId: string) => void,
): Promise<void> {
	if (!canUseTauriInvoke()) {
		return;
	}

	const channel = new Channel<string>();
	channel.onmessage = callback;
	await invoke("subscribe_acp_session_removals", { onEvent: channel });
}
