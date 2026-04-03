import type {
	AcpMcpServerConfig,
	AcpAgentLaunchMode,
	BuiltinMcpConfig,
	BuiltinMcpModuleKey,
	LlmProviderConfig,
	PromptsSettings,
} from "../../lib/tauri/types";

export type SettingsSectionId =
	| "general"
	| "prompts"
	| "llm"
	| "rag"
	| "acp"
	| "mcp"
	| "skills"
	| "about";

export type McpPanelMode = "edit" | "create";

export interface SettingsQuickLink {
	id: string;
	label: string;
	hint: string;
}

export interface AcpAgentDraft {
	id: string;
	name: string;
	command: string;
	launchMode: AcpAgentLaunchMode;
}

export type McpTransport = AcpMcpServerConfig["transport"];
export type BuiltinMcpFieldKey = BuiltinMcpModuleKey | "enabled";
export type AcpFieldKey = "name" | "command" | "launchMode";
export type McpFieldKey = "name" | "command" | "url" | "envText" | "headersText";
export type LlmFieldKey = "name" | "baseUrl" | "model";
export type LlmModelFieldKey = "model";
export type RagFieldKey = "embeddingProviderId" | "sourceDirectories" | "ignoreGlobs";
export type LlmProviderKind =
	| "llm_responses_stateless"
	| "llm_responses_stateful"
	| "llm_chat_completions"
	| "embedding";

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

export interface AcpDraftValidation {
	totalIssues: number;
	agentIssues: Record<string, string[]>;
	agentFieldIssues: Record<string, FieldIssueMap<AcpFieldKey>>;
}

export interface SavedAcpDraftState {
	agents: AcpAgentDraft[];
	defaultAgentId: string | null;
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
	embeddingProviderId: string | null;
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

export interface AcpIssueFocusTarget {
	agentId: string;
	fieldKey: AcpFieldKey;
}

export interface McpIssueFocusTarget {
	serverId: string;
	serverFieldKey: McpFieldKey;
}

export interface LlmIssueFocusTarget {
	providerId: string;
	fieldKey: LlmFieldKey;
}

export const defaultPromptsSettings: PromptsSettings = {
	translationPrompt: [
		"You are an expert translation engine specialized in English ↔ Simplified Chinese.",
		"",
		"Translate the following text accurately, naturally, and fluently.",
		"",
		"Rules:",
		"- If the user specifies a target language, follow it exactly.",
		"- If no target language is specified:",
		"  - Primarily Simplified Chinese → English",
		"  - Primarily English → Simplified Chinese",
		"  - Other languages → Simplified Chinese",
		"- Preserve original meaning, tone, style, and all formatting (Markdown, code blocks, URLs, proper nouns, etc.).",
		"- Return ONLY the translation. No explanations, notes, or extra text.",
	].join("\n"),
	ragAnswerSystemPrompt: [
		"You are a precise tool-augmented assistant. Answer questions using only the tools available.",
		"",
		"Core Rules:",
		"- Always ground your answers in tool results. Never assert repository-specific, document-specific, or system-specific facts without first using RAG/search/file-reading/MCP tools to gather evidence.",
		"- Use tools in multiple rounds if needed: start broad, then drill down to exact files and line ranges until evidence is sufficient.",
		"- For exact file content, always call the file reading tool with the precise path and line window. Do not guess.",
		"- For broader context, use RAG or MCP tools instead of assuming.",
		"- In your final answer, cite concrete file paths and line numbers when tool results provide them.",
		"- Clearly distinguish facts from inferences. Label any inference explicitly.",
		"- You may add concise general background knowledge when helpful, but never fabricate file paths, APIs, behaviors, code, or configuration values.",
		"- If tool results are conflicting, incomplete, or insufficient, state it clearly and explain what is missing.",
		"",
		"Return Markdown only.",
	].join("\n"),
};
