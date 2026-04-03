import { defaultAppearanceSettings } from "../../app/appearance";
import { defaultRagIgnoreGlobs } from "../../lib/tauri/client";
import type {
	AcpAgentConfig,
	AcpMcpServerConfig,
	AppSettings,
	BuiltinMcpConfig,
	BuiltinLlmProviderTemplate,
	BuiltinLlmProviderTemplateModel,
	GeneralSettings,
	LlmProviderConfig,
	LlmSettings,
	NotificationSettings,
	OcrSettings,
	PromptsSettings,
	RagSettings,
	ShortcutConfig,
	WorkspaceState,
} from "../../lib/tauri/types";
import { getMcpTransportMeta } from "./settingsShared";
import type {
	AcpAgentDraft,
	AcpDraftValidation,
	AcpFieldKey,
	AcpIssueFocusTarget,
	AcpMcpServerDraft,
	FieldIssueMap,
	LlmDraftValidation,
	LlmFieldKey,
	LlmIssueFocusTarget,
	LlmProviderKind,
	McpDraftValidation,
	McpFieldKey,
	McpIssueFocusTarget,
	McpTransport,
	RagDraftValidation,
	RagFieldKey,
	SavedAcpDraftState,
	SavedLlmDraftState,
	SavedMcpDraftState,
	SavedRagDraftState,
} from "./settingsTypes";
import { defaultPromptsSettings } from "./settingsTypes";

const fixedRagIgnoreGlobSet = new Set<string>(defaultRagIgnoreGlobs);
const shortcutModifierCodes = new Set([
	"MetaLeft",
	"MetaRight",
	"ControlLeft",
	"ControlRight",
	"AltLeft",
	"AltRight",
	"ShiftLeft",
	"ShiftRight",
]);

const shortcutCodeKeyMap: Record<string, string> = {
	Space: "Space",
	Enter: "Enter",
	NumpadEnter: "Enter",
	Escape: "Escape",
	Tab: "Tab",
	Backspace: "Backspace",
	Delete: "Delete",
	ArrowUp: "ArrowUp",
	ArrowDown: "ArrowDown",
	ArrowLeft: "ArrowLeft",
	ArrowRight: "ArrowRight",
	Home: "Home",
	End: "End",
	PageUp: "PageUp",
	PageDown: "PageDown",
	Backquote: "Backquote",
	Minus: "Minus",
	Equal: "Equal",
	BracketLeft: "BracketLeft",
	BracketRight: "BracketRight",
	Backslash: "Backslash",
	Semicolon: "Semicolon",
	Quote: "Quote",
	Comma: "Comma",
	Period: "Period",
	Slash: "Slash",
};

const validMcpRemoteUrlProtocols = new Set(["http:", "https:"]);

export const extraRagIgnoreGlobPlaceholder = "可选：每行一个额外 glob，例如 **/storybook-static/**";

export const ragSupportedFileExtensions = [
	"md",
	"mdx",
	"txt",
	"markdown",
	"rst",
	"adoc",
	"docx",
	"pdf",
] as const;

export function normalizeRecordedShortcutKey(code: string, key: string) {
	if (shortcutModifierCodes.has(code)) {
		return null;
	}

	if (/^Key[A-Z]$/.test(code)) {
		return code.slice(3);
	}

	if (/^Digit[0-9]$/.test(code)) {
		return code.slice(5);
	}

	if (/^F([1-9]|1[0-2])$/.test(code)) {
		return code;
	}

	if (code in shortcutCodeKeyMap) {
		return shortcutCodeKeyMap[code];
	}

	// Prefer physical key codes so Alt/Option combinations still record the base key on macOS.
	if (key === " ") {
		return "Space";
	}

	if (key.length === 1) {
		return key.toUpperCase();
	}

	if (key.length > 1) {
		return key.charAt(0).toUpperCase() + key.slice(1);
	}

	return null;
}

export function normalizeRagIgnoreGlobs(ignoreGlobs: string[]) {
	const seen = new Set<string>();
	const extras = ignoreGlobs.filter((pattern) => {
		if (fixedRagIgnoreGlobSet.has(pattern) || seen.has(pattern)) {
			return false;
		}
		seen.add(pattern);
		return true;
	});
	return [...defaultRagIgnoreGlobs, ...extras];
}

export function splitExtraRagIgnoreGlobs(ignoreGlobs: string[]) {
	return ignoreGlobs.filter((pattern) => !fixedRagIgnoreGlobSet.has(pattern));
}

export function getErrorMessage(error: unknown, fallbackMessage: string) {
	return error instanceof Error ? error.message : fallbackMessage;
}

export function createDefaultGeneralSettings(): GeneralSettings {
	return {
		autoStart: false,
		showInDock: true,
		language: "zh-CN",
	};
}

export function createDefaultNotificationSettings(): NotificationSettings {
	return {
		enabled: false,
		notifyQuestionAnswerCompletion: true,
		notifyAcpPromptCompletion: true,
		onlyWhenLauncherInBackground: true,
		contentPreview: "brief",
	};
}

export function createDefaultShortcutSettings(): ShortcutConfig {
	return {
		toggle_launcher: "Alt+Space",
		ocr_translate: "Alt+D",
		open_clipboard_history: "Alt+V",
	};
}

export function createDefaultLlmSettings(): LlmSettings {
	return {
		providers: [],
		translationProviderId: null,
		questionAnswerProviderId: null,
	};
}

export function createDefaultOcrSettings(): OcrSettings {
	return {
		provider: "system",
		llmProviderId: null,
	};
}

export function createDefaultRagSettings(): RagSettings {
	return {
		sourceDirectories: [],
		ignoreGlobs: normalizeRagIgnoreGlobs([...defaultRagIgnoreGlobs]),
		embeddingProviderId: null,
	};
}

export function createDefaultWorkspaceState(): WorkspaceState {
	return {
		rootPath: "",
		recentRoots: [],
		homePath: null,
		displayHomeAsTilde: false,
	};
}

export function createDefaultAppSettings(): AppSettings {
	return {
		general: createDefaultGeneralSettings(),
		notification: createDefaultNotificationSettings(),
		appearance: { ...defaultAppearanceSettings },
		prompts: { ...defaultPromptsSettings },
		llm: createDefaultLlmSettings(),
		ocr: createDefaultOcrSettings(),
		rag: createDefaultRagSettings(),
	};
}

function formatDirectCommand(program: string, args: string[]) {
	return [program, ...args]
		.filter((value) => value.trim().length > 0)
		.map((value) => (/[\s"]/u.test(value) ? JSON.stringify(value) : value))
		.join(" ");
}

export function deriveProgramFromCommand(command: string) {
	const normalized = command.trim();
	if (!normalized) {
		return "";
	}

	const [program] = normalized.split(/\s+/, 1);
	return program ?? "";
}

export function buildAgentCommand(
	agent: Pick<AcpAgentConfig, "program" | "args" | "shellCommand">,
) {
	return agent.shellCommand?.trim() || formatDirectCommand(agent.program, agent.args);
}

function nextDraftId(prefix: string) {
	return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function splitNonEmptyLines(value: string) {
	return value
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0);
}

export function parseTextLines(value: string) {
	return splitNonEmptyLines(value);
}

function detectPathSeparator(paths: Array<string | null | undefined>) {
	return paths.some((path) => path?.includes("\\")) ? "\\" : "/";
}

function trimTrailingPathSeparators(path: string) {
	return path.replace(/[\\/]+$/u, "");
}

function joinPathForDisplay(basePath: string, childPath: string) {
	const trimmedBasePath = trimTrailingPathSeparators(basePath.trim());
	if (!trimmedBasePath) {
		return childPath;
	}

	const separator = detectPathSeparator([trimmedBasePath]);
	return `${trimmedBasePath}${separator}${childPath
		.split(/[\\/]+/u)
		.filter((segment) => segment.length > 0)
		.join(separator)}`;
}

export function resolveDocumentsDirectoryPath(workspace: WorkspaceState) {
	const homePath = workspace.homePath?.trim() ?? "";
	if (homePath) {
		return joinPathForDisplay(homePath, "Documents");
	}

	return "~/Documents";
}

export function resolveRagDirectoryPickerDefaultPath(workspace: WorkspaceState) {
	return resolveDocumentsDirectoryPath(workspace);
}

export function formatTextLines(lines: string[]) {
	return lines.join("\n");
}

export function parseKeyValueLines(value: string, label: string) {
	return splitNonEmptyLines(value).map((line, index) => {
		const separatorIndex = line.indexOf("=");
		if (separatorIndex <= 0) {
			throw new Error(`${label} 第 ${index + 1} 行必须是 KEY=VALUE`);
		}

		return {
			name: line.slice(0, separatorIndex).trim(),
			value: line.slice(separatorIndex + 1).trim(),
		};
	});
}

function formatKeyValueLines(items: { name: string; value: string }[]) {
	return items.map((item) => `${item.name}=${item.value}`).join("\n");
}

function createRemoteMcpServerDraft(
	server: Extract<AcpMcpServerConfig, { transport: "http" | "sse" }>,
): AcpMcpServerDraft {
	return {
		id: nextDraftId("mcp"),
		transport: server.transport,
		name: server.name,
		command: "",
		url: server.url,
		argsText: "",
		envText: "",
		headersText: formatKeyValueLines(server.headers),
	};
}

export function createMcpServerDraft(transport: McpTransport = "stdio"): AcpMcpServerDraft {
	return {
		id: nextDraftId("mcp"),
		transport,
		name: "",
		command: "",
		url: "",
		argsText: "",
		envText: "",
		headersText: "",
	};
}

export function createMcpServerDraftFromConfig(server: AcpMcpServerConfig): AcpMcpServerDraft {
	switch (server.transport) {
		case "stdio":
			return {
				id: nextDraftId("mcp"),
				transport: "stdio",
				name: server.name,
				command: server.command,
				url: "",
				argsText: formatTextLines(server.args),
				envText: formatKeyValueLines(server.env),
				headersText: "",
			};
		case "http":
		case "sse":
			return createRemoteMcpServerDraft(server);
	}
}

function cloneMcpServerDraft(server: AcpMcpServerDraft): AcpMcpServerDraft {
	return {
		...server,
	};
}

function cloneAgentDraft(agent: AcpAgentDraft): AcpAgentDraft {
	return {
		...agent,
	};
}

export function cloneSavedAcpDraftState(state: SavedAcpDraftState): SavedAcpDraftState {
	return {
		defaultAgentId: state.defaultAgentId,
		agents: state.agents.map(cloneAgentDraft),
	};
}

export function cloneSavedMcpDraftState(state: SavedMcpDraftState): SavedMcpDraftState {
	return {
		servers: state.servers.map(cloneMcpServerDraft),
		builtin: {
			enabled: state.builtin.enabled,
			enabledModules: [...state.builtin.enabledModules],
		},
	};
}

export function cloneLlmProviderDraft(provider: LlmProviderConfig): LlmProviderConfig {
	return {
		...provider,
	};
}

function cloneSavedLlmDraftState(state: SavedLlmDraftState): SavedLlmDraftState {
	return {
		providers: state.providers.map(cloneLlmProviderDraft),
	};
}

function cloneSavedRagDraftState(state: SavedRagDraftState): SavedRagDraftState {
	return {
		sourceDirectories: [...state.sourceDirectories],
		ignoreGlobs: [...state.ignoreGlobs],
		embeddingProviderId: state.embeddingProviderId,
	};
}

export function serializeMcpServerDraft(server: AcpMcpServerDraft): AcpMcpServerConfig {
	if (server.transport === "stdio") {
		return {
			transport: "stdio",
			name: server.name.trim(),
			command: server.command.trim(),
			args: parseTextLines(server.argsText),
			env: parseKeyValueLines(server.envText, "MCP env"),
		};
	}

	return {
		transport: server.transport,
		name: server.name.trim(),
		url: requireValidMcpRemoteUrl(server.url, server.transport),
		headers: parseKeyValueLines(server.headersText, "MCP headers"),
	};
}

export function createAgentDraft(
	name = "",
	command = "",
	launchMode: AcpAgentDraft["launchMode"] = "login_shell",
): AcpAgentDraft {
	return {
		id: nextDraftId("agent"),
		name,
		command,
		launchMode,
	};
}

export function createLlmProviderDraft(): LlmProviderConfig {
	return {
		id: nextDraftId("llm"),
		name: "",
		baseUrl: "https://api.openai.com/v1",
		apiKey: "",
		modelType: "llm",
		protocol: "responses",
		model: "",
		modelIdentityHint: null,
		builtinPresetId: null,
		builtinPresetModelId: null,
		managedBaseUrl: false,
		supportsMultimodal: false,
		supportsStateful: false,
	};
}

export function findBuiltinTemplate(
	templates: BuiltinLlmProviderTemplate[],
	templateId: string | null | undefined,
) {
	if (!templateId) {
		return null;
	}

	return templates.find((template) => template.id === templateId) ?? null;
}

export function findBuiltinTemplateModel(
	template: BuiltinLlmProviderTemplate | null,
	modelId: string | null | undefined,
) {
	if (!template || !modelId) {
		return null;
	}

	return template.models.find((model) => model.id === modelId) ?? null;
}

export function findBuiltinTemplateModelByModelName(
	template: BuiltinLlmProviderTemplate | null,
	modelName: string | null | undefined,
) {
	if (!template || !modelName) {
		return null;
	}

	const normalizedModelName = modelName.trim();
	if (!normalizedModelName) {
		return null;
	}

	return template.models.find((model) => model.model === normalizedModelName) ?? null;
}

export function applyBuiltinTemplateModelMetadata(
	provider: LlmProviderConfig,
	templateModel: BuiltinLlmProviderTemplateModel | null,
) {
	if (!templateModel) {
		return sanitizeLlmProviderDraft({
			...provider,
			builtinPresetModelId: null,
		});
	}

	return sanitizeLlmProviderDraft({
		...provider,
		model: templateModel.model,
		modelType: templateModel.modelType === "embedding" ? "embedding" : "llm",
		protocol: templateModel.protocol === "chat_completions" ? "chat_completions" : "responses",
		supportsMultimodal: templateModel.supportsMultimodal && templateModel.protocol === "responses",
		supportsStateful: templateModel.supportsStateful && templateModel.protocol === "responses",
		builtinPresetModelId: templateModel.id,
	});
}

export function providerUsesBuiltinTemplate(provider: Pick<LlmProviderConfig, "builtinPresetId">) {
	return Boolean(provider.builtinPresetId);
}

export function applyBuiltinTemplateToProvider(
	provider: LlmProviderConfig,
	template: BuiltinLlmProviderTemplate,
) {
	const nextName = provider.name.trim() ? provider.name : template.displayName;
	return sanitizeLlmProviderDraft({
		...provider,
		name: nextName,
		baseUrl: template.defaultBaseUrl,
		builtinPresetId: template.id,
		managedBaseUrl: true,
		builtinPresetModelId: null,
	});
}

export function detachBuiltinTemplateFromProvider(provider: LlmProviderConfig) {
	return sanitizeLlmProviderDraft({
		...provider,
		builtinPresetId: null,
		builtinPresetModelId: null,
		managedBaseUrl: false,
	});
}

export function providerIsLlmModel(provider: Pick<LlmProviderConfig, "modelType">) {
	return provider.modelType === "llm";
}

export function providerIsEmbeddingModel(provider: Pick<LlmProviderConfig, "modelType">) {
	return provider.modelType === "embedding";
}

interface ResolvedLlmProviderProfile {
	configured: boolean;
	kind: LlmProviderKind;
	canHandleAiTask: boolean;
	canHandleOcr: boolean;
	canHandleRagEmbedding: boolean;
	supportsMultimodal: boolean;
	supportsStateful: boolean;
}

function resolveLlmProviderProfile(
	provider: Pick<
		LlmProviderConfig,
		"modelType" | "model" | "protocol" | "supportsMultimodal" | "supportsStateful"
	>,
): ResolvedLlmProviderProfile {
	const configured = provider.model.trim().length > 0;
	if (provider.modelType === "embedding") {
		return {
			configured,
			kind: "embedding",
			canHandleAiTask: false,
			canHandleOcr: false,
			canHandleRagEmbedding: configured,
			supportsMultimodal: false,
			supportsStateful: false,
		};
	}

	if (provider.protocol === "chat_completions") {
		return {
			configured,
			kind: "llm_chat_completions",
			canHandleAiTask: configured,
			canHandleOcr: false,
			canHandleRagEmbedding: false,
			supportsMultimodal: false,
			supportsStateful: false,
		};
	}

	const supportsMultimodal = configured && provider.supportsMultimodal;
	const supportsStateful = configured && provider.supportsStateful;
	return {
		configured,
		kind: supportsStateful ? "llm_responses_stateful" : "llm_responses_stateless",
		canHandleAiTask: configured,
		canHandleOcr: supportsMultimodal,
		canHandleRagEmbedding: false,
		supportsMultimodal,
		supportsStateful,
	};
}

export function providerHasResponsesModel(
	provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol">,
) {
	const profile = resolveLlmProviderProfile({
		...provider,
		supportsMultimodal: false,
		supportsStateful: false,
	});
	return (
		profile.configured &&
		(profile.kind === "llm_responses_stateless" || profile.kind === "llm_responses_stateful")
	);
}

export function providerCanHandleAiTask(
	provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol">,
) {
	return resolveLlmProviderProfile({
		...provider,
		supportsMultimodal: false,
		supportsStateful: false,
	}).canHandleAiTask;
}

export function providerCanHandleOcr(
	provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol" | "supportsMultimodal">,
) {
	return resolveLlmProviderProfile({
		...provider,
		supportsStateful: false,
	}).canHandleOcr;
}

export function providerCanHandleRagEmbedding(
	provider: Pick<LlmProviderConfig, "modelType" | "model">,
) {
	return resolveLlmProviderProfile({
		...provider,
		protocol: "responses",
		supportsMultimodal: false,
		supportsStateful: false,
	}).canHandleRagEmbedding;
}

function sanitizeLlmProviderDraft(provider: LlmProviderConfig) {
	const sanitized = { ...provider };
	const profile = resolveLlmProviderProfile(sanitized);
	if (profile.kind !== "llm_responses_stateless" && profile.kind !== "llm_responses_stateful") {
		sanitized.supportsMultimodal = false;
		sanitized.supportsStateful = false;
	}
	if (sanitized.supportsMultimodal && !profile.canHandleOcr) {
		sanitized.supportsMultimodal = false;
	}
	if (sanitized.supportsStateful && !profile.supportsStateful) {
		sanitized.supportsStateful = false;
	}
	if (!sanitized.model.trim()) {
		sanitized.modelIdentityHint = null;
	}
	if (!sanitized.builtinPresetId) {
		sanitized.builtinPresetModelId = null;
	}

	return sanitized;
}

function resolveLlmRouteProviderId(providers: LlmProviderConfig[], providerId: string | null) {
	if (
		providerId &&
		providers.some((provider) => provider.id === providerId && providerCanHandleAiTask(provider))
	) {
		return providerId;
	}

	return null;
}

export function reconcileLlmSettings(settings: LlmSettings): LlmSettings {
	const providers = settings.providers.map(sanitizeLlmProviderDraft);
	return {
		providers,
		translationProviderId: resolveLlmRouteProviderId(providers, settings.translationProviderId),
		questionAnswerProviderId: resolveLlmRouteProviderId(
			providers,
			settings.questionAnswerProviderId,
		),
	};
}

export function reconcileOcrSettings(
	settings: OcrSettings,
	providers: LlmProviderConfig[],
): OcrSettings {
	if (settings.provider !== "llm_ocr" || !settings.llmProviderId) {
		return settings;
	}

	if (
		providers.some(
			(provider) => provider.id === settings.llmProviderId && providerCanHandleOcr(provider),
		)
	) {
		return settings;
	}

	return {
		...settings,
		llmProviderId: null,
	};
}

export function reconcileRagSettings(
	settings: RagSettings,
	providers: LlmProviderConfig[],
): RagSettings {
	const normalizedIgnoreGlobs = normalizeRagIgnoreGlobs(settings.ignoreGlobs);
	if (!settings.embeddingProviderId) {
		return {
			...settings,
			ignoreGlobs: normalizedIgnoreGlobs,
		};
	}

	if (
		providers.some(
			(provider) =>
				provider.id === settings.embeddingProviderId && providerCanHandleRagEmbedding(provider),
		)
	) {
		return {
			...settings,
			ignoreGlobs: normalizedIgnoreGlobs,
		};
	}

	return {
		...settings,
		ignoreGlobs: normalizedIgnoreGlobs,
		embeddingProviderId: null,
	};
}

export function summarizeLlmProviderProfile(provider: LlmProviderConfig) {
	const profile = resolveLlmProviderProfile(provider);
	if (profile.canHandleAiTask) {
		const kindLabel = getLlmProviderKindLabel(profile.kind);
		return profile.supportsMultimodal ? `${kindLabel} · 多模态` : kindLabel;
	}
	if (profile.canHandleRagEmbedding) {
		return "Embedding";
	}

	return providerIsEmbeddingModel(provider) ? "Embedding · 未配置模型" : "LLM · 未配置模型";
}

export function getLlmProviderKind(
	provider: Pick<LlmProviderConfig, "modelType" | "protocol" | "supportsStateful">,
): LlmProviderKind {
	return resolveLlmProviderProfile({
		...provider,
		model: "__resolved__",
		supportsMultimodal: false,
	}).kind;
}

export function getLlmProviderKindLabel(kind: LlmProviderKind) {
	switch (kind) {
		case "llm_responses_stateless":
			return "LLM · Responses Stateless";
		case "llm_responses_stateful":
			return "LLM · Responses Stateful";
		case "llm_chat_completions":
			return "LLM · Chat Completions";
		case "embedding":
			return "Embedding";
	}
}

export function applyLlmProviderKind(
	provider: LlmProviderConfig,
	kind: LlmProviderKind,
): LlmProviderConfig {
	switch (kind) {
		case "llm_responses_stateless":
			return {
				...provider,
				modelType: "llm",
				protocol: "responses",
				supportsStateful: false,
			};
		case "llm_responses_stateful":
			return {
				...provider,
				modelType: "llm",
				protocol: "responses",
				supportsStateful: true,
			};
		case "llm_chat_completions":
			return {
				...provider,
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
			};
		case "embedding":
			return {
				...provider,
				modelType: "embedding",
				protocol: "responses",
				supportsMultimodal: false,
				supportsStateful: false,
			};
	}
}

export function getLlmProviderUsageBadges(
	provider: Pick<
		LlmProviderConfig,
		"modelType" | "protocol" | "supportsMultimodal" | "supportsStateful"
	>,
): string[] {
	const profile = resolveLlmProviderProfile({
		...provider,
		model: "__resolved__",
	});
	if (profile.kind === "embedding") {
		return ["RAG 索引", "RAG 检索"];
	}

	if (profile.kind === "llm_chat_completions") {
		return ["翻译", "RAG 问答", "Chat Completions"];
	}

	const badges = profile.supportsMultimodal
		? ["翻译", "RAG 问答", "OCR", "Responses"]
		: ["翻译", "RAG 问答", "Responses"];
	if (profile.kind === "llm_responses_stateful") {
		badges.push("Stateful");
	}
	if (profile.kind === "llm_responses_stateless") {
		badges.push("Stateless");
	}
	return badges;
}

export function getLlmProviderUsageDescription(
	provider: Pick<
		LlmProviderConfig,
		"modelType" | "protocol" | "supportsMultimodal" | "supportsStateful"
	>,
): string {
	const profile = resolveLlmProviderProfile({
		...provider,
		model: "__resolved__",
	});
	if (profile.kind === "embedding") {
		return "Embedding 条目只会出现在 RAG 的 embedding 列表，不会进入翻译 LLM、问答 LLM 或 OCR。";
	}

	if (profile.kind === "llm_chat_completions") {
		return "这个条目会走 OpenAI 兼容 chat/completions 协议，当前可供翻译和 RAG 问答复用；继续追问时始终回退到显式历史，不支持 response_id 续链，也不会进入 OCR 列表。";
	}

	const statefulText =
		profile.kind === "llm_responses_stateful"
			? "这是 responses 的 stateful 版本，继续追问时会优先复用上一轮 response_id。"
			: "这是 responses 的 stateless 版本，继续追问时不会复用上一轮 response_id，而是回退到显式历史。";
	if (profile.supportsMultimodal) {
		return `这个条目会走 OpenAI 兼容 responses 协议，当前可供翻译、RAG 问答和 OCR 复用。${statefulText}`;
	}

	return `这个条目会走 OpenAI 兼容 responses 协议，当前可供翻译和 RAG 问答复用；启用多模态后才会进入 OCR 列表。${statefulText}`;
}

export function getLlmProviderModelPlaceholder(
	provider: Pick<LlmProviderConfig, "modelType">,
): string {
	return provider.modelType === "embedding" ? "text-embedding-3-small" : "gpt-4.1-mini";
}

function parseMcpRemoteUrl(url: string): URL | null {
	try {
		return new URL(url.trim());
	} catch {
		return null;
	}
}

function validateMcpRemoteUrl(
	url: string,
	transport: Exclude<McpTransport, "stdio">,
): string | null {
	const trimmed = url.trim();
	if (!trimmed) {
		return "请输入服务 URL。";
	}

	const parsed = parseMcpRemoteUrl(trimmed);
	if (!parsed) {
		return `请输入完整 URL，例如 ${getMcpTransportMeta(transport).example}。`;
	}

	if (!validMcpRemoteUrlProtocols.has(parsed.protocol)) {
		return "只支持 http:// 或 https://。";
	}

	return null;
}

function requireValidMcpRemoteUrl(url: string, transport: Exclude<McpTransport, "stdio">): string {
	const issue = validateMcpRemoteUrl(url, transport);
	if (issue) {
		throw new Error(issue);
	}

	return url.trim();
}

export function buildSavedAcpDraftState(
	agents: AcpAgentDraft[],
	defaultAgentId: string | null,
): SavedAcpDraftState {
	return cloneSavedAcpDraftState({
		agents,
		defaultAgentId,
	});
}

export function buildSavedMcpDraftState(
	servers: AcpMcpServerDraft[],
	builtin: BuiltinMcpConfig,
): SavedMcpDraftState {
	return cloneSavedMcpDraftState({
		servers,
		builtin,
	});
}

export function buildSavedLlmDraftState(providers: LlmProviderConfig[]): SavedLlmDraftState {
	return cloneSavedLlmDraftState({
		providers,
	});
}

export function buildSavedRagDraftState(settings: RagSettings): SavedRagDraftState {
	return cloneSavedRagDraftState({
		sourceDirectories: settings.sourceDirectories,
		ignoreGlobs: settings.ignoreGlobs,
		embeddingProviderId: settings.embeddingProviderId,
	});
}

export function buildAcpDraftSnapshot(agents: AcpAgentDraft[]) {
	return JSON.stringify({
		agents: agents.map((agent) => ({
			id: agent.id,
			name: agent.name,
			command: agent.command,
			launchMode: agent.launchMode,
		})),
	});
}

export function buildMcpDraftSnapshot(servers: AcpMcpServerDraft[], builtin: BuiltinMcpConfig) {
	return JSON.stringify({
		servers: servers.map((server) => ({
			transport: server.transport,
			name: server.name,
			command: server.command,
			url: server.url,
			argsText: server.argsText,
			envText: server.envText,
			headersText: server.headersText,
		})),
		builtin: {
			enabled: builtin.enabled,
			enabledModules: [...builtin.enabledModules].sort(),
		},
	});
}

export function buildLlmDraftSnapshot(settings: Pick<LlmSettings, "providers">) {
	return JSON.stringify({
		providers: settings.providers.map((provider) => ({
			id: provider.id,
			name: provider.name,
			baseUrl: provider.baseUrl,
			apiKey: provider.apiKey,
			modelType: provider.modelType,
			protocol: provider.protocol,
			model: provider.model,
			modelIdentityHint: provider.modelIdentityHint,
			builtinPresetId: provider.builtinPresetId,
			builtinPresetModelId: provider.builtinPresetModelId,
			managedBaseUrl: provider.managedBaseUrl,
			supportsMultimodal: provider.supportsMultimodal,
			supportsStateful: provider.supportsStateful,
		})),
	});
}

export function buildPromptsDraftSnapshot(
	settings: PromptsSettings,
	llmSettings: Pick<LlmSettings, "translationProviderId" | "questionAnswerProviderId">,
) {
	return JSON.stringify({
		translationPrompt: settings.translationPrompt,
		ragAnswerSystemPrompt: settings.ragAnswerSystemPrompt,
		translationProviderId: llmSettings.translationProviderId,
		questionAnswerProviderId: llmSettings.questionAnswerProviderId,
	});
}

export function buildTranslationTaskDraftSnapshot(
	settings: Pick<PromptsSettings, "translationPrompt">,
	llmSettings: Pick<LlmSettings, "translationProviderId">,
) {
	return JSON.stringify({
		translationPrompt: settings.translationPrompt,
		translationProviderId: llmSettings.translationProviderId,
	});
}

export function buildQuestionAnswerTaskDraftSnapshot(
	settings: Pick<PromptsSettings, "ragAnswerSystemPrompt">,
	llmSettings: Pick<LlmSettings, "questionAnswerProviderId">,
) {
	return JSON.stringify({
		ragAnswerSystemPrompt: settings.ragAnswerSystemPrompt,
		questionAnswerProviderId: llmSettings.questionAnswerProviderId,
	});
}

export function buildRagDraftSnapshot(settings: RagSettings) {
	return JSON.stringify({
		sourceDirectories: settings.sourceDirectories,
		ignoreGlobs: settings.ignoreGlobs,
		embeddingProviderId: settings.embeddingProviderId,
	});
}

export function findFirstAcpIssue(agents: AcpAgentDraft[]): AcpIssueFocusTarget | null {
	for (const agent of agents) {
		if (!agent.name.trim()) {
			return {
				agentId: agent.id,
				fieldKey: "name",
			};
		}

		if (!agent.command.trim()) {
			return {
				agentId: agent.id,
				fieldKey: "command",
			};
		}
	}

	return null;
}

export function findFirstMcpIssue(servers: AcpMcpServerDraft[]): McpIssueFocusTarget | null {
	for (const server of servers) {
		if (!server.name.trim()) {
			return {
				serverId: server.id,
				serverFieldKey: "name",
			};
		}

		if (server.transport === "stdio") {
			if (!server.command.trim()) {
				return {
					serverId: server.id,
					serverFieldKey: "command",
				};
			}

			try {
				parseKeyValueLines(server.envText, "MCP env");
			} catch {
				return {
					serverId: server.id,
					serverFieldKey: "envText",
				};
			}
		} else {
			if (!server.url.trim()) {
				return {
					serverId: server.id,
					serverFieldKey: "url",
				};
			}

			try {
				parseKeyValueLines(server.headersText, "MCP headers");
			} catch {
				return {
					serverId: server.id,
					serverFieldKey: "headersText",
				};
			}
		}
	}

	return null;
}

export function findFirstLlmIssue(
	settings: LlmSettings,
	builtinTemplates: BuiltinLlmProviderTemplate[],
): LlmIssueFocusTarget | null {
	for (const provider of settings.providers) {
		if (!provider.name.trim()) {
			return {
				providerId: provider.id,
				fieldKey: "name",
			};
		}

		if (!provider.baseUrl.trim()) {
			return {
				providerId: provider.id,
				fieldKey: "baseUrl",
			};
		}

		if (!provider.model.trim()) {
			return {
				providerId: provider.id,
				fieldKey: "model",
			};
		}

		const template = findBuiltinTemplate(builtinTemplates, provider.builtinPresetId);
		if (provider.builtinPresetId && !template) {
			return {
				providerId: provider.id,
				fieldKey: "model",
			};
		}

		if (
			template &&
			provider.managedBaseUrl &&
			provider.baseUrl.trim() !== template.defaultBaseUrl.trim()
		) {
			return {
				providerId: provider.id,
				fieldKey: "baseUrl",
			};
		}
	}

	return null;
}

export function validateAcpAgents(agents: AcpAgentDraft[]): AcpDraftValidation {
	const agentIssues: Record<string, string[]> = {};
	const agentFieldIssues: Record<string, FieldIssueMap<AcpFieldKey>> = {};
	let totalIssues = 0;

	agents.forEach((agent, agentIndex) => {
		const currentAgentIssues: string[] = [];
		const currentAgentFieldIssues: FieldIssueMap<AcpFieldKey> = {};
		if (!agent.name.trim()) {
			currentAgentIssues.push(`第 ${agentIndex + 1} 个 ACP Agent 缺少名称。`);
			currentAgentFieldIssues.name = "请输入 Agent 名称。";
		}
		if (!agent.command.trim()) {
			currentAgentIssues.push(`第 ${agentIndex + 1} 个 ACP Agent 缺少启动命令。`);
			currentAgentFieldIssues.command = "请输入启动命令。";
		}

		if (currentAgentIssues.length > 0) {
			agentIssues[agent.id] = currentAgentIssues;
			agentFieldIssues[agent.id] = currentAgentFieldIssues;
			totalIssues += currentAgentIssues.length;
		}
	});

	return {
		totalIssues,
		agentIssues,
		agentFieldIssues,
	};
}

export function validateLlmSettings(
	settings: LlmSettings,
	builtinTemplates: BuiltinLlmProviderTemplate[],
): LlmDraftValidation {
	const providerIssues: Record<string, string[]> = {};
	const providerFieldIssues: Record<string, FieldIssueMap<LlmFieldKey>> = {};
	let totalIssues = 0;

	settings.providers.forEach((provider, providerIndex) => {
		const currentProviderIssues: string[] = [];
		const currentProviderFieldIssues: FieldIssueMap<LlmFieldKey> = {};
		if (!provider.name.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个模型条目缺少名称。`);
			currentProviderFieldIssues.name = "请输入条目名称。";
		}
		if (!provider.baseUrl.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个模型条目缺少 Base URL。`);
			currentProviderFieldIssues.baseUrl = "请输入 Base URL。";
		}
		if (!provider.model.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个模型条目缺少模型名。`);
			currentProviderFieldIssues.model = "请输入模型名。";
		}
		if (provider.supportsMultimodal && !providerHasResponsesModel(provider)) {
			currentProviderIssues.push(
				`第 ${providerIndex + 1} 个模型条目只有 responses 协议才能开启多模态。`,
			);
		}
		if (provider.supportsStateful && !providerHasResponsesModel(provider)) {
			currentProviderIssues.push(
				`第 ${providerIndex + 1} 个模型条目只有 responses 协议才能开启 stateful 请求。`,
			);
		}
		const template = findBuiltinTemplate(builtinTemplates, provider.builtinPresetId);
		if (provider.builtinPresetId && !template) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个模型条目引用了未知内置模板。`);
			currentProviderFieldIssues.model = "当前内置模板不存在，请重新选择模型。";
		}

		if (template) {
			if (provider.managedBaseUrl && provider.baseUrl.trim() !== template.defaultBaseUrl.trim()) {
				currentProviderIssues.push(
					`第 ${providerIndex + 1} 个模型条目仍由模板管理 Base URL，但当前值已偏离模板默认地址。`,
				);
				currentProviderFieldIssues.baseUrl = "模板管理模式下 Base URL 必须与模板默认地址一致。";
			}
		}

		if (currentProviderIssues.length > 0) {
			providerIssues[provider.id] = currentProviderIssues;
			providerFieldIssues[provider.id] = currentProviderFieldIssues;
			totalIssues += currentProviderIssues.length;
		}
	});

	return {
		totalIssues,
		providerIssues,
		providerFieldIssues,
	};
}

export function validateRagSettings(
	settings: RagSettings,
	llmSettings: LlmSettings,
): RagDraftValidation {
	const issues: string[] = [];
	const fieldIssues: FieldIssueMap<RagFieldKey> = {};

	settings.sourceDirectories.forEach((directory, index) => {
		if (!directory.trim()) {
			issues.push(`第 ${index + 1} 个扫描目录不能为空。`);
			fieldIssues.sourceDirectories ??= "扫描目录里不能有空行。";
		}
	});

	settings.ignoreGlobs.forEach((pattern, index) => {
		if (!pattern.trim()) {
			issues.push(`第 ${index + 1} 个忽略模式不能为空。`);
			fieldIssues.ignoreGlobs ??= "忽略规则里不能有空行。";
		}
	});

	if (settings.sourceDirectories.length > 0 && !settings.embeddingProviderId) {
		issues.push("配置扫描目录时，必须选择一个 embedding provider。");
		fieldIssues.embeddingProviderId ??= "配置扫描目录时，必须先选择一个 Embedding 条目。";
	}

	if (
		settings.embeddingProviderId &&
		!llmSettings.providers.some(
			(provider) =>
				provider.id === settings.embeddingProviderId && providerCanHandleRagEmbedding(provider),
		)
	) {
		issues.push("RAG 选择的 embedding provider 不存在，或者没有启用 embedding 能力。");
		fieldIssues.embeddingProviderId ??= "当前选择的 Embedding 条目不可用于 RAG。";
	}

	return {
		totalIssues: issues.length,
		issues,
		fieldIssues,
	};
}

export function selectExistingIdOrFirst<T extends { id: string }>(
	items: T[],
	selectedId: string | null,
) {
	if (items.length === 0) {
		return null;
	}

	if (!selectedId || !items.some((item) => item.id === selectedId)) {
		return items[0]?.id ?? null;
	}

	return selectedId;
}

export function validateMcpServers(servers: AcpMcpServerDraft[]): McpDraftValidation {
	const serverIssues: Record<string, string[]> = {};
	const serverFieldIssues: Record<string, FieldIssueMap<McpFieldKey>> = {};
	let totalIssues = 0;

	servers.forEach((server, serverIndex) => {
		const currentServerIssues: string[] = [];
		const currentServerFieldIssues: FieldIssueMap<McpFieldKey> = {};
		if (!server.name.trim()) {
			currentServerIssues.push(`第 ${serverIndex + 1} 个 MCP 服务缺少名称。`);
			currentServerFieldIssues.name = "请输入 MCP 服务名称。";
		}

		if (server.transport === "stdio") {
			if (!server.command.trim()) {
				currentServerIssues.push("本地进程模式必须填写命令。");
				currentServerFieldIssues.command = "本地进程模式必须填写命令。";
			}

			try {
				parseKeyValueLines(server.envText, "MCP env");
			} catch (error: unknown) {
				currentServerIssues.push(getErrorMessage(error, "MCP env 格式错误。"));
				currentServerFieldIssues.envText = "环境变量格式错误，请按每行一个 KEY=VALUE 填写。";
			}
		} else {
			const urlIssue = validateMcpRemoteUrl(server.url, server.transport);
			if (urlIssue) {
				currentServerIssues.push(urlIssue);
				currentServerFieldIssues.url = urlIssue;
			}

			try {
				parseKeyValueLines(server.headersText, "MCP headers");
			} catch (error: unknown) {
				currentServerIssues.push(getErrorMessage(error, "MCP headers 格式错误。"));
				currentServerFieldIssues.headersText = "请求头格式错误，请按每行一个 KEY=VALUE 填写。";
			}
		}

		if (currentServerIssues.length > 0) {
			serverIssues[server.id] = currentServerIssues;
			serverFieldIssues[server.id] = currentServerFieldIssues;
			totalIssues += currentServerIssues.length;
		}
	});

	return {
		totalIssues,
		serverIssues,
		serverFieldIssues,
	};
}
