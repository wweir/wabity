import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
	browserAppSettings,
	browserBuiltinLlmProviderTemplates,
	browserBuiltinMcpServerStatus,
	browserPublicSkillCatalog,
	defaultRagIgnoreGlobs,
} from "./defaults";
import { beginTransientWindowInteraction, endTransientWindowInteraction } from "./launcher";
import { invokeIfDesktop, invokeOrDefault, listenIfDesktop } from "./runtime";
import type {
	AcpAgentCatalog,
	AcpAgentConfig,
	AcpMcpServerCatalog,
	AcpMcpServerConfig,
	AppSettings,
	BuiltinLlmProviderTemplate,
	BuiltinMcpServerStatus,
	LlmProviderConfig,
	LlmProviderModelEntry,
	LlmSettings,
	PublicSkillCatalog,
	RagScanResult,
	RagSettings,
	ShortcutConfig,
	ShortcutRuntimeStatus,
} from "../types";

export { defaultRagIgnoreGlobs };

export async function getShortcut(): Promise<ShortcutConfig> {
	return invokeOrDefault("get_shortcut", {
		toggle_launcher: "Alt+Space",
		ocr_translate: "Alt+D",
		open_clipboard_history: "Alt+V",
	});
}

export async function setShortcut(key: string, shortcut: string): Promise<void> {
	return invokeIfDesktop("set_shortcut", { key, shortcut });
}

export async function onShortcutUpdated(callback: (config: ShortcutConfig) => void) {
	return listenIfDesktop("shortcut-updated", callback);
}

export async function getShortcutRuntimeStatus(): Promise<ShortcutRuntimeStatus> {
	return invokeOrDefault("get_shortcut_runtime_status", {
		toggle_launcher: {
			configuredShortcut: "Alt+Space",
			registered: true,
			message: null,
		},
		ocr_translate: {
			configuredShortcut: "Alt+D",
			registered: true,
			message: null,
		},
		open_clipboard_history: {
			configuredShortcut: "Alt+V",
			registered: true,
			message: null,
		},
	});
}

export async function onShortcutRuntimeStatusChanged(
	callback: (status: ShortcutRuntimeStatus) => void,
) {
	return listenIfDesktop("shortcut-runtime-status-changed", callback);
}

export async function getAppSettings(): Promise<AppSettings> {
	return invokeOrDefault("get_app_settings", browserAppSettings);
}

export async function setAppSettings(settings: AppSettings): Promise<AppSettings> {
	return invokeOrDefault("set_app_settings", settings, { settings });
}

export async function getBuiltinMcpServerStatus(): Promise<BuiltinMcpServerStatus> {
	return invokeOrDefault("get_builtin_mcp_server_status", browserBuiltinMcpServerStatus);
}

export async function listLlmProviderModels(
	provider: LlmProviderConfig,
): Promise<LlmProviderModelEntry[]> {
	return invokeOrDefault("list_llm_provider_models", [], { provider });
}

export async function listBuiltinLlmProviderTemplates(): Promise<BuiltinLlmProviderTemplate[]> {
	return invokeOrDefault("list_builtin_llm_provider_templates", browserBuiltinLlmProviderTemplates);
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
	return invokeOrDefault("get_acp_mcp_servers", {
		servers: [],
		builtin: {
			enabled: false,
			enabledModules: [],
		},
	});
}

export async function setAcpMcpServers(
	servers: AcpMcpServerConfig[],
	builtin: AcpMcpServerCatalog["builtin"],
): Promise<AcpMcpServerCatalog> {
	return invokeOrDefault("set_acp_mcp_servers", { servers, builtin }, { servers, builtin });
}
