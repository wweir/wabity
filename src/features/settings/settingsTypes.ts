import type {
	AcpMcpServerConfig,
	BuiltinMcpConfig,
	BuiltinMcpModuleKey,
	LlmProviderConfig,
} from "../../lib/tauri/types";
export { defaultPromptsSettings } from "../../lib/tauri/client/defaultPrompts";

export interface DependencyHealthItem {
	status: "ok" | "warning" | "info";
	label: string;
	detail: string;
}

export type SettingsSectionId = "general" | "prompts" | "llm" | "rag" | "mcp" | "about";

export type McpPanelMode = "edit" | "create";

export interface SettingsQuickLink {
	id: string;
	label: string;
	hint: string;
}

export type McpTransport = AcpMcpServerConfig["transport"];
export type BuiltinMcpFieldKey = BuiltinMcpModuleKey | "enabled";
export type McpFieldKey = "name" | "command" | "url" | "envText" | "headersText";
export type LlmFieldKey = "name" | "baseUrl" | "model";
export type LlmEditableFieldKey = LlmFieldKey | "apiKey" | "supportsMultimodal";
export type LlmModelFieldKey = "model";
export type RagFieldKey = "embeddingModelId" | "sourceDirectories" | "ignoreGlobs";
export type LlmProviderKind =
	"llm_responses_stateless" | "llm_responses_stateful" | "llm_chat_completions" | "embedding";

export type FieldIssueMap<FieldKey extends string> = Partial<Record<FieldKey, string>>;

export interface AcpMcpServerDraft {
	id: string;
	transport: McpTransport;
	name: string;
	command: string;
	url: string;
	argsText: string;
	envText: string;
	headersText: string;
}

export interface SavedMcpDraftState {
	servers: AcpMcpServerDraft[];
	builtin: BuiltinMcpConfig;
}

export interface LlmDraftValidation {
	totalIssues: number;
	providerIssues: Record<string, string[]>;
	providerFieldIssues: Record<string, FieldIssueMap<LlmFieldKey>>;
}

export interface SavedLlmDraftState {
	providers: LlmProviderConfig[];
}

export interface RagDraftValidation {
	totalIssues: number;
	issues: string[];
	fieldIssues: FieldIssueMap<RagFieldKey>;
}

export interface SavedRagDraftState {
	sourceDirectories: string[];
	ignoreGlobs: string[];
	embeddingModelId: string | null;
}

export interface McpDraftValidation {
	totalIssues: number;
	serverIssues: Record<string, string[]>;
	serverFieldIssues: Record<string, FieldIssueMap<McpFieldKey>>;
}

export interface AcpInlineNotice {
	tone: "info" | "warn";
	text: string;
}

export interface PendingMcpFocusTarget {
	serverId: string;
	fieldKey: McpFieldKey;
	scrollToForm: boolean;
}

export interface McpIssueFocusTarget {
	serverId: string;
	serverFieldKey: McpFieldKey;
}

export interface LlmIssueFocusTarget {
	providerId: string;
	fieldKey: LlmFieldKey;
}
