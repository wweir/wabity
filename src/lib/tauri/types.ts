export interface ShortcutConfig {
	toggle_launcher: string;
	ocr_translate: string;
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
	registrationUrl: string;
	apiKeyUrl: string;
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
	finishedAtMs: number;
}

export type RagRuntimePhase = "idle" | "scanning" | "indexing" | "error";

export interface RagRuntimeStatus {
	phase: RagRuntimePhase;
	scannedFileCount: number;
	completedFileCount: number;
	totalFileCount: number;
	pendingFileCount: number;
	lastError: string | null;
	updatedAtMs: number;
}

export interface BuiltinRagMcpServerStatus {
	server: AcpMcpServerConfig;
	running: boolean;
	lastError: string | null;
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

export interface PublicSkillMetaEntry {
	key: string;
	value: string;
}

export interface PublicSkillMeta {
	name: string | null;
	description: string | null;
	argumentHint: string | null;
	license: string | null;
	metadata: PublicSkillMetaEntry[];
}

export type SkillTreeNodeKind = "directory" | "file";

export interface SkillTreeNode {
	name: string;
	relativePath: string;
	kind: SkillTreeNodeKind;
	children: SkillTreeNode[];
}

export interface PublicSkillEntry {
	id: string;
	directoryName: string;
	relativePath: string;
	meta: PublicSkillMeta;
	directoryCount: number;
	fileCount: number;
	tree: SkillTreeNode;
}

export interface PublicSkillCatalog {
	rootPath: string;
	exists: boolean;
	skills: PublicSkillEntry[];
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

export interface AcpAgentConfig {
	id: string;
	name: string;
	program: string;
	args: string[];
	shellCommand: string | null;
}

export interface AcpAgentCatalog {
	agents: AcpAgentConfig[];
	defaultAgentId: string | null;
}

export interface AcpMcpServerCatalog {
	servers: AcpMcpServerConfig[];
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

export interface AcpSessionDetail {
	session: AcpSessionSummary;
	messages: AcpSessionMessage[];
}

export interface AcpRestoreNotice {
	sessionId: string;
	workspaceRoot: string;
	message: string;
}
