import type {
	AcpSessionDetail,
	AppSettings,
	BuiltinAgentToolStatus,
	ClipboardHistorySnapshot,
	RagRuntimeStatus,
	ShortcutConfig,
	WorkspaceState,
} from "../types";
import { defaultPromptsSettings } from "./defaultPrompts";
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
	prompts: { ...defaultPromptsSettings },
	llm: {
		providers: [],
		translationModelId: null,
		questionAnswerModelId: null,
	},
	ocr: {
		provider: "system",
		llmModelId: null,
	},
	rag: {
		sourceDirectories: [],
		ignoreGlobs: [...defaultRagIgnoreGlobs],
		embeddingModelId: null,
	},
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

export const browserBuiltinAgentToolStatus: BuiltinAgentToolStatus = {
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
