export interface ShortcutConfig {
	toggle_launcher: string;
	ocr_translate: string;
	open_clipboard_history: string;
}

export type ShortcutKey = keyof ShortcutConfig;

export interface ShortcutRuntimeStatusEntry {
	configuredShortcut: string;
	registered: boolean;
	message: string | null;
}

export type ShortcutRuntimeStatus = Record<ShortcutKey, ShortcutRuntimeStatusEntry>;

export interface ClipboardHistoryEntry {
	id: string;
	text: string;
	pinned: boolean;
	lastSeenAtMs: number;
	pinnedAtMs: number | null;
}

export interface ClipboardHistorySnapshot {
	pinnedEntries: ClipboardHistoryEntry[];
	recentEntries: ClipboardHistoryEntry[];
}

export type ClipboardHistorySelectionMode = "insert_into_launcher" | "paste_externally";

export interface OpenClipboardHistoryPanelEvent {
	selectionMode: ClipboardHistorySelectionMode;
}

export type RevealLauncherMainPanelEvent = Record<string, never>;

export interface InsertClipboardHistoryTextIntoLauncherEvent {
	text: string;
}

export interface GeneralSettings {
	autoStart: boolean;
	showInDock: boolean;
	language: string;
}

export interface NotificationSettings {
	enabled: boolean;
	notifyQuestionAnswerCompletion: boolean;
	notifyAcpPromptCompletion: boolean;
	onlyWhenLauncherInBackground: boolean;
	contentPreview: "hidden" | "brief";
}

export interface AppearanceSettings {
	theme: string;
	fontSize: string;
}

export interface PromptsSettings {
	translationPrompt: string;
	ragAnswerSystemPrompt: string;
}

export type LlmProviderProtocol = "responses" | "chat_completions";
export type BuiltinLlmTemplateModelType =
	| "llm"
	| "embedding"
	| "image_generation"
	| "video_generation";
export type BuiltinLlmTemplateModelProtocol = "responses" | "chat_completions" | "unsupported";
export type BuiltinLlmTemplateUseCase = "translation" | "rag_answer" | "ocr" | "embedding";

export interface LlmProviderConfig {
	id: string;
	name: string;
	baseUrl: string;
	apiKey: string;
	modelType: "llm" | "embedding";
	protocol: LlmProviderProtocol;
	model: string;
	modelIdentityHint: string | null;
	builtinPresetId: string | null;
	builtinPresetModelId: string | null;
	managedBaseUrl: boolean;
	supportsMultimodal: boolean;
	supportsStateful: boolean;
}

export interface LlmProviderModelEntry {
	id: string;
	identityHint: string | null;
}

export interface BuiltinLlmProviderTemplateModel {
	id: string;
	displayName: string;
	model: string;
	modelType: BuiltinLlmTemplateModelType;
	protocol: BuiltinLlmTemplateModelProtocol;
	supportsMultimodal: boolean;
	supportsStateful: boolean;
	recommendedFor: BuiltinLlmTemplateUseCase[];
	summary: string;
	selectableInCurrentApp: boolean;
	disabledReason: string | null;
}

export interface BuiltinLlmProviderTemplate {
	id: string;
	displayName: string;
	description: string;
	registrationLabel: string;
	registrationUrl: string;
	apiKeyLabel: string;
	apiKeyUrl: string;
	docsLabel: string;
	docsUrl: string;
	defaultBaseUrl: string;
	supportsModelListing: boolean;
	models: BuiltinLlmProviderTemplateModel[];
}

export interface LlmSettings {
	providers: LlmProviderConfig[];
	translationProviderId: string | null;
	questionAnswerProviderId: string | null;
}

export type OcrProviderKind = "disabled" | "system" | "llm_ocr";

export interface OcrSettings {
	provider: OcrProviderKind;
	llmProviderId: string | null;
}

export interface RagSettings {
	sourceDirectories: string[];
	ignoreGlobs: string[];
	embeddingProviderId: string | null;
}

export interface RagScanResult {
	databasePath: string;
	sourceCount: number;
	scannedFileCount: number;
	indexedFileCount: number;
	skippedFileCount: number;
	chunkCount: number;
	warningCount: number;
	recentWarnings: string[];
	finishedAtMs: number;
}

export type RagRuntimePhase = "idle" | "scanning" | "indexing" | "error";

export interface RagRuntimeStatus {
	phase: RagRuntimePhase;
	scannedFileCount: number;
	completedFileCount: number;
	totalFileCount: number;
	pendingFileCount: number;
	warningCount: number;
	recentWarnings: string[];
	lastError: string | null;
	updatedAtMs: number;
}

export type BuiltinMcpModuleKey = "rag" | "document";

export interface BuiltinMcpConfig {
	enabled: boolean;
	enabledModules: BuiltinMcpModuleKey[];
}

export interface BuiltinMcpModuleStatus {
	key: BuiltinMcpModuleKey;
	title: string;
	summary: string;
	toolCount: number;
}

export interface BuiltinMcpServerStatus {
	server: AcpMcpServerConfig;
	running: boolean;
	lastError: string | null;
	availableModules: BuiltinMcpModuleStatus[];
}

export interface AppSettings {
	general: GeneralSettings;
	notification: NotificationSettings;
	appearance: AppearanceSettings;
	prompts: PromptsSettings;
	llm: LlmSettings;
	ocr: OcrSettings;
	rag: RagSettings;
}

export interface WorkspaceState {
	rootPath: string;
	recentRoots: string[];
	homePath: string | null;
	displayHomeAsTilde: boolean;
}

export interface AcpNameValuePair {
	name: string;
	value: string;
}

export interface AcpMcpServerStdioConfig {
	transport: "stdio";
	name: string;
	command: string;
	args: string[];
	env: AcpNameValuePair[];
}

export interface AcpMcpServerHttpConfig {
	transport: "http";
	name: string;
	url: string;
	headers: AcpNameValuePair[];
}

export interface AcpMcpServerSseConfig {
	transport: "sse";
	name: string;
	url: string;
	headers: AcpNameValuePair[];
}

export type AcpMcpServerConfig =
	| AcpMcpServerStdioConfig
	| AcpMcpServerHttpConfig
	| AcpMcpServerSseConfig;

export type AcpAgentLaunchMode = "direct" | "login_shell" | "interactive_shell";

export interface AcpAgentConfig {
	id: string;
	name: string;
	program: string;
	args: string[];
	shellCommand: string | null;
	launchMode: AcpAgentLaunchMode;
}

export interface AcpAgentCatalog {
	agents: AcpAgentConfig[];
	defaultAgentId: string | null;
}

export interface AcpMcpServerCatalog {
	servers: AcpMcpServerConfig[];
	builtin: BuiltinMcpConfig;
}

export type AcpSessionStatus = "starting" | "idle" | "running" | "error" | "exited";
export type AcpSessionErrorLevel = "recoverable" | "fatal";

export type AcpMessageRole = "user" | "assistant" | "system";

export interface AcpActionEvent {
	kind: string;
	title: string;
	correlationId: string | null;
	detail: string | null;
}

export type AcpMessageBlock =
	| { type: "thought"; content: string }
	| { type: "actions"; items: AcpActionEvent[] }
	| { type: "content"; text: string };

export interface AcpSessionMessage {
	id: string;
	role: AcpMessageRole;
	blocks: AcpMessageBlock[];
	pending: boolean;
}

export interface AcpSessionSummary {
	sessionId: string;
	workspaceRoot: string;
	title: string;
	agentId: string | null;
	agentName: string;
	status: AcpSessionStatus;
	errorLevel: AcpSessionErrorLevel | null;
	attention: boolean;
	isActive: boolean;
	lastError: string | null;
	lastUpdatedAtMs: number;
}

export interface AcpModeOption {
	id: string;
	name: string;
	description: string | null;
}

export interface AcpConfigValueOption {
	valueId: string;
	name: string;
	description: string | null;
}

export interface AcpConfigOptionGroup {
	id: string;
	name: string;
	options: AcpConfigValueOption[];
}

export type AcpConfigOptionKind = {
	type: "select";
	currentValueId: string;
	options: AcpConfigValueOption[];
	groups: AcpConfigOptionGroup[];
};

export interface AcpConfigOption {
	id: string;
	name: string;
	description: string | null;
	category: string | null;
	kind: AcpConfigOptionKind;
}

export interface AcpSessionRuntimeState {
	currentModeId: string | null;
	availableModes: AcpModeOption[];
	configOptions: AcpConfigOption[];
}

export interface AcpSessionDetail {
	session: AcpSessionSummary;
	messages: AcpSessionMessage[];
	runtime: AcpSessionRuntimeState;
}

export interface AcpRestoreNotice {
	sessionId: string;
	workspaceRoot: string;
	message: string;
}
