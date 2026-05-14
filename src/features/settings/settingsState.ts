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
	LlmModelConfig,
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
		translationModelId: null,
		questionAnswerModelId: null,
	};
}

export function createDefaultOcrSettings(): OcrSettings {
	return {
		provider: "system",
		llmModelId: null,
	};
}

export function createDefaultRagSettings(): RagSettings {
	return {
		sourceDirectories: [],
		ignoreGlobs: normalizeRagIgnoreGlobs([...defaultRagIgnoreGlobs]),
		embeddingModelId: null,
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
		.map(quoteDirectCommandArgument)
		.join(" ");
}

function quoteDirectCommandArgument(value: string) {
	if (!value) {
		return '""';
	}

	if (isWindowsPlatform()) {
		return quoteWindowsCommandArgument(value);
	}

	return /[\s"'\\]/u.test(value) ? `'${value.replace(/'/gu, `'"'"'`)}'` : value;
}

function quoteWindowsCommandArgument(value: string) {
	if (!/[\s"]/u.test(value)) {
		return value;
	}

	let quoted = '"';
	let backslashes = 0;
	for (const char of value) {
		if (char === "\\") {
			backslashes += 1;
			continue;
		}

		if (char === '"') {
			quoted += "\\".repeat(backslashes * 2 + 1);
			quoted += '"';
			backslashes = 0;
			continue;
		}

		quoted += "\\".repeat(backslashes);
		quoted += char;
		backslashes = 0;
	}

	quoted += "\\".repeat(backslashes * 2);
	quoted += '"';
	return quoted;
}

function isWindowsPlatform() {
	return typeof navigator !== "undefined" && /Windows/iu.test(navigator.userAgent);
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
		models: provider.models.map((model) => ({
			...model,
		})),
		modelConfig:
			(provider.modelConfig && { ...provider.modelConfig }) ??
			(provider.models[0] ? { ...provider.models[0] } : undefined),
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
		embeddingModelId: state.embeddingModelId,
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
	const initialModel = createLlmModelDraft("llm");
	return {
		id: nextDraftId("llm"),
		name: "",
		baseUrl: "https://api.openai.com/v1",
		apiKey: "",
		protocol: "responses",
		models: [initialModel],
		modelConfig: initialModel,
		builtinPresetId: null,
		managedBaseUrl: false,
	};
}

export function createLlmModelDraft(
	modelType: LlmModelConfig["modelType"] = "llm",
): LlmModelConfig {
	return {
		id: nextDraftId("llm-model"),
		modelType,
		model: "",
		modelIdentityHint: null,
		builtinPresetModelId: null,
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
	modelId: string | null = provider.models[0]?.id ?? null,
) {
	const currentModel =
		provider.models.find((model) => model.id === modelId) ??
		provider.models[0] ??
		createLlmModelDraft(templateModel?.modelType === "embedding" ? "embedding" : "llm");
	if (!templateModel) {
		return sanitizeLlmProviderDraft({
			...provider,
			models: provider.models.map((model) =>
				model.id === currentModel.id ? { ...model, builtinPresetModelId: null } : model,
			),
		});
	}

	return sanitizeLlmProviderDraft({
		...provider,
		protocol: templateModel.protocol === "chat_completions" ? "chat_completions" : "responses",
		models: upsertProviderModel(provider, {
			...currentModel,
			model: templateModel.model,
			modelType: templateModel.modelType === "embedding" ? "embedding" : "llm",
			supportsMultimodal:
				templateModel.supportsMultimodal && templateModel.protocol === "responses",
			supportsStateful: templateModel.supportsStateful && templateModel.protocol === "responses",
			builtinPresetModelId: templateModel.id,
		}),
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
		models: provider.models.map((model) => ({
			...model,
			builtinPresetModelId: null,
		})),
		managedBaseUrl: true,
	});
}

export function detachBuiltinTemplateFromProvider(provider: LlmProviderConfig) {
	return sanitizeLlmProviderDraft({
		...provider,
		builtinPresetId: null,
		models: provider.models.map((model) => ({
			...model,
			builtinPresetModelId: null,
		})),
		managedBaseUrl: false,
	});
}

function getProviderModel(
	provider: Pick<LlmProviderConfig, "models">,
	modelId?: string | null,
): LlmModelConfig | null {
	if (modelId) {
		return provider.models.find((model) => model.id === modelId) ?? null;
	}

	return provider.models[0] ?? null;
}

function upsertProviderModel(provider: LlmProviderConfig, nextModel: LlmModelConfig) {
	const existingModels = provider.models.filter((model) => model.id !== nextModel.id);
	return [...existingModels, nextModel];
}

export function clearProviderModelIdentityHints(provider: LlmProviderConfig): LlmProviderConfig {
	return {
		...provider,
		models: provider.models.map((model) => ({
			...model,
			modelIdentityHint: null,
		})),
	};
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
	provider: Pick<LlmProviderConfig, "protocol">,
	model: LlmModelConfig | null,
): ResolvedLlmProviderProfile {
	const configured = Boolean(model?.model.trim());
	if (model?.modelType === "embedding") {
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

	const supportsMultimodal = configured && Boolean(model?.supportsMultimodal);
	const supportsStateful = configured && Boolean(model?.supportsStateful);
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

export function providerIsLlmModel(
	provider: Pick<LlmProviderConfig, "models">,
	modelId?: string | null,
) {
	return getProviderModel(provider, modelId)?.modelType === "llm";
}

export function providerIsEmbeddingModel(
	provider: Pick<LlmProviderConfig, "models">,
	modelId?: string | null,
) {
	return getProviderModel(provider, modelId)?.modelType === "embedding";
}

export function providerHasResponsesModel(
	provider: Pick<LlmProviderConfig, "models" | "protocol">,
	modelId?: string | null,
) {
	const profile = resolveLlmProviderProfile(provider, getProviderModel(provider, modelId));
	return (
		profile.configured &&
		(profile.kind === "llm_responses_stateless" || profile.kind === "llm_responses_stateful")
	);
}

export function providerCanHandleAiTask(
	provider: Pick<LlmProviderConfig, "models" | "protocol">,
	modelId?: string | null,
) {
	return resolveLlmProviderProfile(provider, getProviderModel(provider, modelId)).canHandleAiTask;
}

export function providerCanHandleOcr(
	provider: Pick<LlmProviderConfig, "models" | "protocol">,
	modelId?: string | null,
) {
	return resolveLlmProviderProfile(provider, getProviderModel(provider, modelId)).canHandleOcr;
}

export function providerCanHandleRagEmbedding(
	provider: Pick<LlmProviderConfig, "models" | "protocol">,
	modelId?: string | null,
) {
	return resolveLlmProviderProfile(provider, getProviderModel(provider, modelId))
		.canHandleRagEmbedding;
}

function resolveBuiltinPresetModelId(
	provider: LlmProviderConfig,
	model: LlmModelConfig,
	builtinTemplates?: BuiltinLlmProviderTemplate[],
) {
	if (!provider.builtinPresetId || !model.builtinPresetModelId) {
		return null;
	}

	if (!builtinTemplates) {
		return model.builtinPresetModelId;
	}

	const template = findBuiltinTemplate(builtinTemplates, provider.builtinPresetId);
	return findBuiltinTemplateModel(template, model.builtinPresetModelId)
		? model.builtinPresetModelId
		: null;
}

function sanitizeLlmProviderDraft(
	provider: LlmProviderConfig,
	builtinTemplates?: BuiltinLlmProviderTemplate[],
) {
	const models = provider.models.map((model) => {
		const sanitizedModel = { ...model };
		const profile = resolveLlmProviderProfile(provider, sanitizedModel);
		if (profile.kind !== "llm_responses_stateless" && profile.kind !== "llm_responses_stateful") {
			sanitizedModel.supportsMultimodal = false;
			sanitizedModel.supportsStateful = false;
		}
		if (sanitizedModel.supportsMultimodal && !profile.canHandleOcr) {
			sanitizedModel.supportsMultimodal = false;
		}
		if (sanitizedModel.supportsStateful && !profile.supportsStateful) {
			sanitizedModel.supportsStateful = false;
		}
		if (!sanitizedModel.model.trim()) {
			sanitizedModel.modelIdentityHint = null;
		}
		sanitizedModel.builtinPresetModelId = resolveBuiltinPresetModelId(
			provider,
			sanitizedModel,
			builtinTemplates,
		);
		return sanitizedModel;
	});
	return {
		...provider,
		models,
		modelConfig:
			(provider.modelConfig && models.find((model) => model.id === provider.modelConfig?.id)) ??
			models[0] ??
			createLlmModelDraft("llm"),
	};
}

function resolveLlmRouteModelId(providers: LlmProviderConfig[], modelId: string | null) {
	if (modelId && providers.some((provider) => providerCanHandleAiTask(provider, modelId))) {
		return modelId;
	}

	return null;
}

export function reconcileLlmSettings(
	settings: LlmSettings,
	builtinTemplates?: BuiltinLlmProviderTemplate[],
): LlmSettings {
	const providers = settings.providers.map((provider) =>
		sanitizeLlmProviderDraft(provider, builtinTemplates),
	);
	return {
		providers,
		translationModelId: resolveLlmRouteModelId(providers, settings.translationModelId),
		questionAnswerModelId: resolveLlmRouteModelId(providers, settings.questionAnswerModelId),
	};
}

export function reconcileOcrSettings(
	settings: OcrSettings,
	providers: LlmProviderConfig[],
): OcrSettings {
	if (settings.provider !== "llm_ocr" || !settings.llmModelId) {
		return settings;
	}

	if (providers.some((provider) => providerCanHandleOcr(provider, settings.llmModelId))) {
		return settings;
	}

	return {
		...settings,
		llmModelId: null,
	};
}

export function reconcileRagSettings(
	settings: RagSettings,
	providers: LlmProviderConfig[],
): RagSettings {
	const normalizedIgnoreGlobs = normalizeRagIgnoreGlobs(settings.ignoreGlobs);
	if (!settings.embeddingModelId) {
		return {
			...settings,
			ignoreGlobs: normalizedIgnoreGlobs,
		};
	}

	if (
		providers.some((provider) => providerCanHandleRagEmbedding(provider, settings.embeddingModelId))
	) {
		return {
			...settings,
			ignoreGlobs: normalizedIgnoreGlobs,
		};
	}

	return {
		...settings,
		ignoreGlobs: normalizedIgnoreGlobs,
		embeddingModelId: null,
	};
}

export function summarizeLlmProviderProfile(provider: LlmProviderConfig) {
	const modelCount = provider.models.length;
	if (modelCount === 0) {
		return provider.protocol === "chat_completions"
			? "LLM 组 · 未配置模型"
			: "Provider 组 · 未配置模型";
	}
	const llmCount = provider.models.filter((model) => model.modelType === "llm").length;
	const embeddingCount = provider.models.filter((model) => model.modelType === "embedding").length;
	if (llmCount > 0 && embeddingCount > 0) {
		return `${llmCount} 个 LLM · ${embeddingCount} 个 Embedding`;
	}
	if (llmCount > 0) {
		return `${llmCount} 个 LLM`;
	}
	return `${embeddingCount} 个 Embedding`;
}

export function getLlmProviderKind(
	provider: Pick<LlmProviderConfig, "models" | "protocol">,
	modelId?: string | null,
): LlmProviderKind {
	return resolveLlmProviderProfile(provider, {
		...getProviderModel(provider, modelId),
		model: "__resolved__",
	} as LlmModelConfig).kind;
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
	modelId: string | null = provider.models[0]?.id ?? null,
): LlmProviderConfig {
	const currentModel =
		provider.models.find((model) => model.id === modelId) ??
		provider.models[0] ??
		createLlmModelDraft(kind === "embedding" ? "embedding" : "llm");
	switch (kind) {
		case "llm_responses_stateless":
			return {
				...provider,
				protocol: "responses",
				models: upsertProviderModel(provider, {
					...currentModel,
					modelType: "llm",
					supportsStateful: false,
				}),
			};
		case "llm_responses_stateful":
			return {
				...provider,
				protocol: "responses",
				models: upsertProviderModel(provider, {
					...currentModel,
					modelType: "llm",
					supportsStateful: true,
				}),
			};
		case "llm_chat_completions":
			return {
				...provider,
				protocol: "chat_completions",
				models: upsertProviderModel(provider, {
					...currentModel,
					modelType: "llm",
					supportsMultimodal: false,
					supportsStateful: false,
				}),
			};
		case "embedding":
			return {
				...provider,
				protocol: "responses",
				models: upsertProviderModel(provider, {
					...currentModel,
					modelType: "embedding",
					supportsMultimodal: false,
					supportsStateful: false,
				}),
			};
	}
}

export function getLlmProviderUsageBadges(
	provider: Pick<LlmProviderConfig, "models" | "protocol">,
	modelId?: string | null,
): string[] {
	const profile = resolveLlmProviderProfile(provider, {
		...getProviderModel(provider, modelId),
		model: "__resolved__",
	} as LlmModelConfig);
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
	provider: Pick<LlmProviderConfig, "models" | "protocol">,
	modelId?: string | null,
): string {
	const profile = resolveLlmProviderProfile(provider, {
		...getProviderModel(provider, modelId),
		model: "__resolved__",
	} as LlmModelConfig);
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
	provider: Pick<LlmProviderConfig, "models">,
	modelId?: string | null,
): string {
	return getProviderModel(provider, modelId)?.modelType === "embedding"
		? "text-embedding-3-small"
		: "gpt-4.1-mini";
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
		embeddingModelId: settings.embeddingModelId,
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
			protocol: provider.protocol,
			models: provider.models.map((model) => ({
				id: model.id,
				modelType: model.modelType,
				model: model.model,
				modelIdentityHint: model.modelIdentityHint,
				builtinPresetModelId: model.builtinPresetModelId,
				supportsMultimodal: model.supportsMultimodal,
				supportsStateful: model.supportsStateful,
			})),
			builtinPresetId: provider.builtinPresetId,
			managedBaseUrl: provider.managedBaseUrl,
		})),
	});
}

export function buildPromptsDraftSnapshot(
	settings: PromptsSettings,
	llmSettings: Pick<LlmSettings, "translationModelId" | "questionAnswerModelId">,
) {
	return JSON.stringify({
		translationPrompt: settings.translationPrompt,
		ragAnswerSystemPrompt: settings.ragAnswerSystemPrompt,
		translationModelId: llmSettings.translationModelId,
		questionAnswerModelId: llmSettings.questionAnswerModelId,
	});
}

export function buildTranslationTaskDraftSnapshot(
	settings: Pick<PromptsSettings, "translationPrompt">,
	llmSettings: Pick<LlmSettings, "translationModelId">,
) {
	return JSON.stringify({
		translationPrompt: settings.translationPrompt,
		translationModelId: llmSettings.translationModelId,
	});
}

export function buildQuestionAnswerTaskDraftSnapshot(
	settings: Pick<PromptsSettings, "ragAnswerSystemPrompt">,
	llmSettings: Pick<LlmSettings, "questionAnswerModelId">,
) {
	return JSON.stringify({
		ragAnswerSystemPrompt: settings.ragAnswerSystemPrompt,
		questionAnswerModelId: llmSettings.questionAnswerModelId,
	});
}

export function buildRagDraftSnapshot(settings: RagSettings) {
	return JSON.stringify({
		sourceDirectories: settings.sourceDirectories,
		ignoreGlobs: settings.ignoreGlobs,
		embeddingModelId: settings.embeddingModelId,
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

		if (provider.models.length === 0 || provider.models.some((model) => !model.model.trim())) {
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
		if (provider.models.length === 0) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个 Provider 组至少需要一个模型。`);
			currentProviderFieldIssues.model = "请输入模型名。";
		}
		provider.models.forEach((model, modelIndex) => {
			if (!model.model.trim()) {
				currentProviderIssues.push(
					`第 ${providerIndex + 1} 个 Provider 组的第 ${modelIndex + 1} 个模型缺少模型名。`,
				);
				currentProviderFieldIssues.model = "请输入模型名。";
			}
			if (model.supportsMultimodal && !providerHasResponsesModel(provider, model.id)) {
				currentProviderIssues.push(
					`第 ${providerIndex + 1} 个 Provider 组的第 ${modelIndex + 1} 个模型只有 responses 协议才能开启多模态。`,
				);
			}
			if (model.supportsStateful && !providerHasResponsesModel(provider, model.id)) {
				currentProviderIssues.push(
					`第 ${providerIndex + 1} 个 Provider 组的第 ${modelIndex + 1} 个模型只有 responses 协议才能开启 stateful 请求。`,
				);
			}
		});
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

	if (settings.sourceDirectories.length > 0 && !settings.embeddingModelId) {
		issues.push("配置扫描目录时，必须选择一个 embedding 模型。");
		fieldIssues.embeddingModelId ??= "配置扫描目录时，必须先选择一个 Embedding 条目。";
	}

	if (
		settings.embeddingModelId &&
		!llmSettings.providers.some((provider) =>
			providerCanHandleRagEmbedding(provider, settings.embeddingModelId),
		)
	) {
		issues.push("RAG 选择的 embedding 模型不存在，或者没有启用 embedding 能力。");
		fieldIssues.embeddingModelId ??= "当前选择的 Embedding 条目不可用于 RAG。";
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
