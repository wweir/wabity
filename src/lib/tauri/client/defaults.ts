import type {
	AcpSessionDetail,
	AppSettings,
	BuiltinMcpServerStatus,
	ClipboardHistorySnapshot,
	PublicSkillCatalog,
	RagRuntimeStatus,
	ShortcutConfig,
	WorkspaceState,
} from "../types";
import { browserBuiltinLlmProviderTemplates } from "./templates";

export const browserShortcutConfig: ShortcutConfig = {
	toggle_launcher: "Alt+Space",
	ocr_translate: "Alt+D",
	open_clipboard_history: "Alt+V",
};

export const browserClipboardHistorySnapshot: ClipboardHistorySnapshot = {
	pinnedEntries: [],
	recentEntries: [],
};

export const defaultRagIgnoreGlobs = [
	"**/.git/**",
	"**/node_modules/**",
	"**/vendor/**",
	"**/Pods/**",
	"**/target/**",
	"**/dist/**",
	"**/build/**",
	"**/out/**",
	"**/.next/**",
	"**/.nuxt/**",
	"**/.svelte-kit/**",
	"**/.turbo/**",
	"**/.cache/**",
	"**/coverage/**",
	"**/.venv/**",
	"**/venv/**",
] as const;

export const browserAppSettings: AppSettings = {
	general: {
		autoStart: false,
		showInDock: true,
		language: "zh-CN",
	},
	notification: {
		enabled: false,
		notifyQuestionAnswerCompletion: true,
		notifyAcpPromptCompletion: true,
		onlyWhenLauncherInBackground: true,
		contentPreview: "brief",
	},
	appearance: {
		theme: "auto",
		fontSize: "medium",
	},
	prompts: {
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
	},
	llm: {
		providers: [],
		translationProviderId: null,
		questionAnswerProviderId: null,
	},
	ocr: {
		provider: "system",
		llmProviderId: null,
	},
	rag: {
		sourceDirectories: [],
		ignoreGlobs: [...defaultRagIgnoreGlobs],
		embeddingProviderId: null,
	},
};

export const browserPublicSkillCatalog: PublicSkillCatalog = {
	rootPath: "~/.agents/skills",
	exists: false,
	skills: [],
};

export const browserWorkspaceState: WorkspaceState = {
	rootPath: "/",
	recentRoots: ["/"],
	homePath: null,
	displayHomeAsTilde: false,
};

export const browserRagRuntimeStatus: RagRuntimeStatus = {
	phase: "idle",
	scannedFileCount: 0,
	completedFileCount: 0,
	totalFileCount: 0,
	pendingFileCount: 0,
	warningCount: 0,
	recentWarnings: [],
	lastError: null,
	updatedAtMs: 0,
};

export const browserBuiltinMcpServerStatus: BuiltinMcpServerStatus = {
	server: {
		transport: "http",
		name: "Wabity Built-in MCP",
		url: "http://127.0.0.1:43189/internal/mcp",
		headers: [],
	},
	running: false,
	lastError: "仅桌面端运行时提供内置 MCP server。",
	availableModules: [
		{
			key: "rag",
			title: "RAG 检索",
			summary: "向量检索本地索引，返回命中 chunk、路径和分数。",
			toolCount: 1,
		},
		{
			key: "document",
			title: "文档读取",
			summary: "按精确行号或 chunk 读取允许范围内的文本与文档摘录。",
			toolCount: 2,
		},
	],
};

export function buildBrowserSessionDetail(sessionId: string): AcpSessionDetail {
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
		runtime: {
			currentModeId: null,
			availableModes: [],
			configOptions: [],
		},
	};
}

export { browserBuiltinLlmProviderTemplates };
