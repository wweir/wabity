import { invoke, isTauri, Channel } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
	ClipboardHistorySnapshot,
	InsertClipboardHistoryTextIntoLauncherEvent,
	OpenClipboardHistoryPanelEvent,
	RevealLauncherMainPanelEvent,
	AcpRestoreNotice,
	AcpSessionDetail,
	AcpSessionSummary,
	AppSettings,
	BuiltinLlmProviderTemplate,
	BuiltinMcpServerStatus,
	LlmProviderConfig,
	LlmProviderModelEntry,
	LlmSettings,
	PublicSkillCatalog,
	RagRuntimeStatus,
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
	toggle_launcher: "Alt+Space",
	ocr_translate: "Alt+D",
	open_clipboard_history: "Alt+V",
};

const browserClipboardHistorySnapshot: ClipboardHistorySnapshot = {
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

export interface OcrTranslationResultEvent {
	sourceMode: "ocr" | "selection";
	sourceText: string;
	result: ExecutionResult;
}

export interface OcrTranslationStartedEvent {
	sourceMode: "ocr" | "selection";
	sourceText: string;
}

export interface OcrTranslationStreamEvent {
	sourceMode: "ocr" | "selection";
	sourceText: string;
	partialText: string;
}

export interface ExecutionProgressEvent {
	actionId: string;
	statusText: string;
	partialText?: string | null;
}

const browserAppSettings: AppSettings = {
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

const browserPublicSkillCatalog: PublicSkillCatalog = {
	rootPath: "~/.agents/skills",
	exists: false,
	skills: [],
};

const browserBuiltinLlmProviderTemplates: BuiltinLlmProviderTemplate[] = [
	{
		id: "openai",
		displayName: "OpenAI",
		description:
			"官方 API 模板。先登录 OpenAI 平台、创建 API Key，再从常用 Responses / Embedding 模型里选择。",
		registrationLabel: "注册 / 登录",
		registrationUrl: "https://platform.openai.com/signup",
		apiKeyLabel: "API Key 页面",
		apiKeyUrl: "https://platform.openai.com/api-keys",
		docsLabel: "模型与 API 文档",
		docsUrl: "https://platform.openai.com/docs/overview",
		defaultBaseUrl: "https://api.openai.com/v1",
		supportsModelListing: true,
		models: [
			{
				id: "gpt-5.4-mini",
				displayName: "GPT-5.4-mini",
				model: "gpt-5.4-mini",
				modelType: "llm",
				protocol: "responses",
				supportsMultimodal: true,
				supportsStateful: true,
				recommendedFor: ["translation", "rag_answer", "ocr"],
				summary: "通用小型模型，适合翻译、问答和截图理解，默认成本比旗舰档更低。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "gpt-5.4",
				displayName: "GPT-5.4",
				model: "gpt-5.4",
				modelType: "llm",
				protocol: "responses",
				supportsMultimodal: true,
				supportsStateful: true,
				recommendedFor: ["translation", "rag_answer", "ocr"],
				summary: "旗舰通用模型，适合高质量翻译、复杂问答和多模态理解。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "gpt-5.4-nano",
				displayName: "GPT-5.4-nano",
				model: "gpt-5.4-nano",
				modelType: "llm",
				protocol: "responses",
				supportsMultimodal: true,
				supportsStateful: true,
				recommendedFor: ["translation", "rag_answer", "ocr"],
				summary: "更轻量的通用模型，适合低延迟翻译和基础问答。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "text-embedding-3-small",
				displayName: "text-embedding-3-small",
				model: "text-embedding-3-small",
				modelType: "embedding",
				protocol: "responses",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["embedding"],
				summary: "常用 Embedding 模型，适合给 RAG 建立通用文本向量索引。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "text-embedding-3-large",
				displayName: "text-embedding-3-large",
				model: "text-embedding-3-large",
				modelType: "embedding",
				protocol: "responses",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["embedding"],
				summary: "更高质量的 Embedding 模型，适合更重视召回质量的 RAG 场景。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
		],
	},
	{
		id: "openrouter",
		displayName: "OpenRouter",
		description:
			"聚合网关模板。先登录 OpenRouter、创建 API Key，再按当前账号可见的远端模型目录选择模型。",
		registrationLabel: "注册 / 登录",
		registrationUrl: "https://openrouter.ai/",
		apiKeyLabel: "API Key 页面",
		apiKeyUrl: "https://openrouter.ai/settings/keys",
		docsLabel: "官方文档",
		docsUrl: "https://openrouter.ai/docs/quickstart",
		defaultBaseUrl: "https://openrouter.ai/api/v1",
		supportsModelListing: true,
		models: [],
	},
	{
		id: "deepseek",
		displayName: "DeepSeek",
		description:
			"官方 API 模板。先登录 DeepSeek 平台、创建 API Key，再从常用聊天或推理模型里选择。",
		registrationLabel: "注册 / 登录",
		registrationUrl: "https://platform.deepseek.com/",
		apiKeyLabel: "API Key 页面",
		apiKeyUrl: "https://platform.deepseek.com/api_keys",
		docsLabel: "官方文档",
		docsUrl: "https://api-docs.deepseek.com/",
		defaultBaseUrl: "https://api.deepseek.com",
		supportsModelListing: true,
		models: [
			{
				id: "deepseek-chat",
				displayName: "DeepSeek-Chat",
				model: "deepseek-chat",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "通用对话模型，适合翻译、日常问答和轻量文本生成。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "deepseek-reasoner",
				displayName: "DeepSeek-Reasoner",
				model: "deepseek-reasoner",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "推理模型，适合复杂问答、分析和需要多步思考的场景。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
		],
	},
	{
		id: "ollama",
		displayName: "Ollama",
		description:
			"本地 OpenAI-compatible 模板。先安装 Ollama 并 pull 模型；默认连接本机 `http://localhost:11434/v1`，API Key 可以留空。",
		registrationLabel: "下载 / 安装",
		registrationUrl: "https://ollama.com/download",
		apiKeyLabel: "OpenAI 兼容说明",
		apiKeyUrl: "https://docs.ollama.com/openai",
		docsLabel: "模型与文档",
		docsUrl: "https://docs.ollama.com/",
		defaultBaseUrl: "http://localhost:11434/v1",
		supportsModelListing: true,
		models: [
			{
				id: "qwen3-8b",
				displayName: "Qwen3 8B",
				model: "qwen3:8b",
				modelType: "llm",
				protocol: "responses",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "常见本地文本模型，适合日常翻译、问答和低成本试配。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "gpt-oss-20b",
				displayName: "gpt-oss 20B",
				model: "gpt-oss:20b",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "常见本地推理模型，适合复杂问答、解释和代码辅助场景。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "qwen3-vl-8b",
				displayName: "Qwen3-VL 8B",
				model: "qwen3-vl:8b",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: true,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "常见本地图像理解模型，适合截图和文档理解；当前不会进入 OCR 列表。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "embeddinggemma",
				displayName: "EmbeddingGemma",
				model: "embeddinggemma",
				modelType: "embedding",
				protocol: "responses",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["embedding"],
				summary: "常见本地 Embedding 模型，适合给 RAG 建立向量索引。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
		],
	},
	{
		id: "zhipu",
		displayName: "智谱 AI",
		description: "官方免费模型目录模板。先注册智谱开放平台、创建 API Key，再从白名单里选择模型。",
		registrationLabel: "注册 / 登录",
		registrationUrl: "https://bigmodel.cn/login?redirect=%2Fusercenter%2Fproj-mgmt%2Fapikeys",
		apiKeyLabel: "API Key 页面",
		apiKeyUrl: "https://bigmodel.cn/login?redirect=%2Fusercenter%2Fproj-mgmt%2Fapikeys",
		docsLabel: "官方文档",
		docsUrl: "https://docs.bigmodel.cn/cn/guide/start/quick-start",
		defaultBaseUrl: "https://open.bigmodel.cn/api/paas/v4",
		supportsModelListing: true,
		models: [
			{
				id: "glm-4.7-flash",
				displayName: "GLM-4.7-Flash",
				model: "glm-4.7-flash",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费文本模型，适合翻译、问答和通用长文本任务。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "glm-4.6v-flash",
				displayName: "GLM-4.6V-Flash",
				model: "glm-4.6v-flash",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: true,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费视觉理解模型，擅长图像、视频和文件理解。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "glm-4.1v-thinking-flash",
				displayName: "GLM-4.1V-Thinking-Flash",
				model: "glm-4.1v-thinking-flash",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: true,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费视觉推理模型，适合图表、GUI 和网页理解场景。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "glm-4-flash-250414",
				displayName: "GLM-4-Flash-250414",
				model: "glm-4-flash-250414",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费轻量文本模型，适合通用对话、翻译和基础问答。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "glm-4v-flash",
				displayName: "GLM-4V-Flash",
				model: "glm-4v-flash",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: true,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费图像理解模型，适合图像识别、问答和视觉推理。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "cogview-3-flash",
				displayName: "CogView-3-Flash",
				model: "cogview-3-flash",
				modelType: "image_generation",
				protocol: "unsupported",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: [],
				summary: "免费图像生成模型，适合根据文本快速生成图片。",
				selectableInCurrentApp: false,
				disabledReason: "当前 Wabity 没有图像生成链路，不能当普通 LLM 使用。",
			},
			{
				id: "cogvideox-flash",
				displayName: "CogVideoX-Flash",
				model: "cogvideox-flash",
				modelType: "video_generation",
				protocol: "unsupported",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: [],
				summary: "免费视频生成模型，适合根据文本指令生成短视频。",
				selectableInCurrentApp: false,
				disabledReason: "当前 Wabity 没有视频生成链路，不能当普通 LLM 使用。",
			},
		],
	},
	{
		id: "siliconflow",
		displayName: "SiliconFlow",
		description:
			"官方免费语言模型目录模板。先注册 SiliconFlow、创建 API Key，再从白名单里选择模型。",
		registrationLabel: "注册 / 登录",
		registrationUrl: "https://account.siliconflow.cn",
		apiKeyLabel: "API Key 页面",
		apiKeyUrl: "https://cloud.siliconflow.cn/account/ak",
		docsLabel: "官方文档",
		docsUrl: "https://docs.siliconflow.cn/cn/api-reference/chat-completions/chat-completions",
		defaultBaseUrl: "https://api.siliconflow.cn/v1",
		supportsModelListing: true,
		models: [
			{
				id: "qwen3.5-4b-instruct-2507",
				displayName: "Qwen3.5-4B-Instruct-2507",
				model: "Qwen/Qwen3.5-4B-Instruct-2507",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费轻量指令模型，适合低成本翻译、问答和日常文本任务。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "paddleocr-vl-1.5",
				displayName: "PaddleOCR-VL-1.5",
				model: "PaddlePaddle/PaddleOCR-VL-1.5",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: true,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "免费文档理解模型，适合票据、表格和复杂版面 OCR 识别。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "deepseek-r1-distill-qwen-7b",
				displayName: "DeepSeek-R1-Distill-Qwen-7B",
				model: "deepseek-ai/DeepSeek-R1-Distill-Qwen-7B",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费轻量推理模型，适合分析、问答和需要推理的文本任务。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "glm-4.1v-9b-thinking",
				displayName: "GLM-4.1V-9B-Thinking",
				model: "THUDM/GLM-4.1V-9B-Thinking",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: true,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "免费视觉推理模型，适合图表、截图和复杂图像理解。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "paddleocr-vl",
				displayName: "PaddleOCR-VL",
				model: "PaddlePaddle/PaddleOCR-VL",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: true,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "免费 OCR / 文档解析模型，适合表格、票据和富版面内容提取。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "deepseek-ocr",
				displayName: "DeepSeek-OCR",
				model: "deepseek-ai/DeepSeek-OCR",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: true,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "免费 OCR 模型，适合截图、扫描件和文档文字提取。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "qwen3-8b",
				displayName: "Qwen3-8B",
				model: "Qwen/Qwen3-8B",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费通用文本模型，适合对话、翻译和基础问答。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "hunyuan-mt-7b",
				displayName: "Hunyuan-MT-7B",
				model: "tencent/Hunyuan-MT-7B",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation"],
				summary: "免费机器翻译模型，适合中英文和多语种翻译场景。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "deepseek-r1-0528-qwen3-8b",
				displayName: "DeepSeek-R1-0528-Qwen3-8B",
				model: "deepseek-ai/DeepSeek-R1-0528-Qwen3-8B",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "免费推理模型，适合复杂问答和需要多步分析的任务。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "glm-z1-9b-0414",
				displayName: "GLM-Z1-9B-0414",
				model: "THUDM/GLM-Z1-9B-0414",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["rag_answer"],
				summary: "免费推理模型，适合代码解释、复杂问答和长链思考。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "qwen2.5-7b-instruct",
				displayName: "Qwen2.5-7B-Instruct",
				model: "Qwen/Qwen2.5-7B-Instruct",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费通用指令模型，适合日常问答、改写和轻量生成。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "glm-4-9b-0414",
				displayName: "GLM-4-9B-0414",
				model: "THUDM/GLM-4-9B-0414",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费通用文本模型，适合对话、翻译和基础知识问答。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
			{
				id: "internlm2-5-7b-chat",
				displayName: "internlm2_5-7b-chat",
				model: "internlm/internlm2_5-7b-chat",
				modelType: "llm",
				protocol: "chat_completions",
				supportsMultimodal: false,
				supportsStateful: false,
				recommendedFor: ["translation", "rag_answer"],
				summary: "免费聊天模型，适合日常问答和轻量文本生成。",
				selectableInCurrentApp: true,
				disabledReason: null,
			},
		],
	},
];

const browserWorkspaceState: WorkspaceState = {
	rootPath: "/",
	recentRoots: ["/"],
	homePath: null,
	displayHomeAsTilde: false,
};

const browserRagRuntimeStatus: RagRuntimeStatus = {
	phase: "idle",
	scannedFileCount: 0,
	completedFileCount: 0,
	totalFileCount: 0,
	pendingFileCount: 0,
	lastError: null,
	updatedAtMs: 0,
};

const browserBuiltinMcpServerStatus: BuiltinMcpServerStatus = {
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

async function subscribeChannelIfDesktop<T>(
	command: string,
	unsubscribeCommand: string,
	callback: (payload: T) => void,
): Promise<UnlistenFn | null> {
	if (!canUseTauriInvoke()) {
		return null;
	}

	const channel = new Channel<T>();
	channel.onmessage = callback;
	await invokeDesktop(command, { onEvent: channel });
	return () => {
		channel.onmessage = () => {};
		void invokeIfDesktop(unsubscribeCommand, { channelId: channel.id });
	};
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

export async function openDocumentReference(path: string): Promise<void> {
	return invokeIfDesktop("open_document_reference", { path });
}

export async function hideLauncherWindow(): Promise<void> {
	return invokeIfDesktop("hide_launcher_window", {
		windowLabel: await resolveCurrentWindowLabel(),
	});
}

export async function dismissClipboardHistoryPanel(): Promise<void> {
	return invokeIfDesktop("dismiss_clipboard_history_panel");
}

export async function beginTransientWindowInteraction(): Promise<void> {
	return invokeIfDesktop("begin_transient_window_interaction");
}

export async function endTransientWindowInteraction(): Promise<void> {
	return invokeIfDesktop("end_transient_window_interaction");
}

export async function armLauncherBlurAutoHideSuppression(durationMs: number): Promise<void> {
	return invokeIfDesktop("arm_launcher_blur_auto_hide_suppression", {
		durationMs: Math.max(1, Math.ceil(durationMs)),
	});
}

export async function setLauncherBlurAutoHideEnabled(enabled: boolean): Promise<void> {
	return invokeIfDesktop("set_launcher_blur_auto_hide_enabled", { enabled });
}

const lastLauncherWindowSizeByLabel = new Map<string, { width: number; height: number }>();
let currentWindowLabelPromise: Promise<string | null> | null = null;

async function resolveCurrentWindowLabel(): Promise<string | null> {
	if (!isDesktopRuntimeAvailable()) {
		return null;
	}

	if (!currentWindowLabelPromise) {
		currentWindowLabelPromise = Promise.resolve()
			.then(() => getCurrentWindow().label)
			.catch(() => null);
	}

	return currentWindowLabelPromise;
}

export async function resizeLauncherWindow(size: { width: number; height: number }): Promise<void> {
	if (!canUseTauriInvoke()) {
		return;
	}

	const nextWidth = Math.max(1, Math.ceil(size.width));
	const nextHeight = Math.max(1, Math.ceil(size.height));
	const windowLabel = (await resolveCurrentWindowLabel()) ?? "main";
	const lastLauncherWindowSize = lastLauncherWindowSizeByLabel.get(windowLabel) ?? null;

	if (
		lastLauncherWindowSize?.width === nextWidth &&
		lastLauncherWindowSize?.height === nextHeight
	) {
		return;
	}

	await invokeDesktop<void>("resize_launcher_window", {
		width: nextWidth,
		height: nextHeight,
		windowLabel,
	});

	lastLauncherWindowSizeByLabel.set(windowLabel, { width: nextWidth, height: nextHeight });
}

export async function onOcrError(callback: (message: string) => void): Promise<UnlistenFn | null> {
	return listenIfDesktop("ocr-error", callback);
}

export async function onOcrTranslationResult(
	callback: (payload: OcrTranslationResultEvent) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("ocr-translation-result", callback);
}

export async function onOcrTranslationStarted(
	callback: (payload: OcrTranslationStartedEvent) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("ocr-translation-started", callback);
}

export async function onOcrTranslationStream(
	callback: (payload: OcrTranslationStreamEvent) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("ocr-translation-stream", callback);
}

export async function onExecutionProgress(
	callback: (payload: ExecutionProgressEvent) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("execution-progress", callback);
}

export async function onRagRuntimeStatus(
	callback: (payload: RagRuntimeStatus) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("rag-runtime-status", callback);
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

export async function onOpenClipboardHistoryPanel(
	callback: (payload: OpenClipboardHistoryPanelEvent) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("open-clipboard-history-panel", callback);
}

export async function onRevealLauncherMainPanel(
	callback: (payload: RevealLauncherMainPanelEvent) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("reveal-launcher-main-panel", callback);
}

export async function onInsertClipboardHistoryTextIntoLauncher(
	callback: (payload: InsertClipboardHistoryTextIntoLauncherEvent) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("insert-clipboard-history-text-into-launcher", callback);
}

export async function insertClipboardHistoryTextIntoLauncher(text: string): Promise<void> {
	return invokeIfDesktop("insert_clipboard_history_text_into_launcher", { text });
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

export async function getRagRuntimeStatus(): Promise<RagRuntimeStatus> {
	return invokeOrDefault("get_rag_runtime_status", browserRagRuntimeStatus);
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

export async function getClipboardHistory(): Promise<ClipboardHistorySnapshot> {
	return invokeOrDefault("get_clipboard_history", browserClipboardHistorySnapshot);
}

export async function toggleClipboardHistoryEntryPin(
	entryId: string,
): Promise<ClipboardHistorySnapshot> {
	return invokeOrDefault("toggle_clipboard_history_entry_pin", browserClipboardHistorySnapshot, {
		entryId,
	});
}

export async function deleteClipboardHistoryEntry(
	entryId: string,
): Promise<ClipboardHistorySnapshot> {
	return invokeOrDefault("delete_clipboard_history_entry", browserClipboardHistorySnapshot, {
		entryId,
	});
}

export async function pasteClipboardHistoryEntry(
	entryId: string,
): Promise<ClipboardHistorySnapshot> {
	return invokeOrDefault("paste_clipboard_history_entry", browserClipboardHistorySnapshot, {
		entryId,
	});
}

export async function onClipboardHistoryUpdated(
	callback: (snapshot: ClipboardHistorySnapshot) => void,
): Promise<UnlistenFn | null> {
	return listenIfDesktop("clipboard-history-updated", callback);
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
): Promise<UnlistenFn | null> {
	return subscribeChannelIfDesktop(
		"subscribe_acp_session_updates",
		"unsubscribe_acp_session_updates",
		callback,
	);
}

export async function subscribeAcpSessionRemovals(
	callback: (sessionId: string) => void,
): Promise<UnlistenFn | null> {
	return subscribeChannelIfDesktop(
		"subscribe_acp_session_removals",
		"unsubscribe_acp_session_removals",
		callback,
	);
}
