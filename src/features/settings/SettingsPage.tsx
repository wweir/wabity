import { useEffect, useRef, useState } from "react";
import type {
	KeyboardEvent as ReactKeyboardEvent,
	MouseEvent as ReactMouseEvent,
	ReactNode,
} from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
	chooseDirectory,
	getAppSettings,
	getAcpAgents,
	getAcpMcpServers,
	getBuiltinRagMcpServerStatus,
	getPublicSkillCatalog,
	getShortcut,
	getWorkspace,
	hideLauncherWindow,
	listLlmProviderModels,
	onShortcutUpdated,
	scanRagSources,
	setAppSettings,
	setAcpAgents,
	setAcpMcpServers,
	setShortcut,
	defaultRagIgnoreGlobs,
} from "../../lib/tauri/client";
import type {
	AcpAgentConfig,
	AcpMcpServerConfig,
	AppSettings,
	AppearanceSettings,
	BuiltinRagMcpServerStatus,
	GeneralSettings,
	LlmProviderConfig,
	LlmProviderModelEntry,
	LlmSettings,
	OcrSettings,
	PromptsSettings,
	PublicSkillCatalog,
	RagScanResult,
	RagSettings,
	ShortcutConfig,
	SkillTreeNode,
	WorkspaceState,
} from "../../lib/tauri/types";
import { useSettingsWindowFrame } from "./useSettingsWindowFrame";
import { applyAppearanceSettings } from "../../app/appearance";
import "./settings.css";

interface SettingsPageProps {
	onBack: () => void;
	onAppearanceChange?: (appearance: AppearanceSettings) => void;
}

type SettingsSectionId = "general" | "prompts" | "llm" | "rag" | "acp" | "mcp" | "skills" | "about";
type McpPanelMode = "edit" | "create";

interface SettingsQuickLink {
	id: string;
	label: string;
	hint: string;
}

interface SettingsQuickJumpListProps {
	activeBlockId: string | null;
	compact?: boolean;
	links: readonly SettingsQuickLink[];
	onSelect: (blockId: string) => void;
}

const acpAgentOptions = [
	{
		id: "opencode",
		label: "OpenCode",
		command: "opencode acp",
		summary:
			"OpenCode 是开源 AI coding agent，支持终端、桌面端和 IDE；这里预填的是它的 ACP 启动命令。",
		installCommand: "curl -fsSL https://opencode.ai/install | bash",
		installHint:
			"也可以用 `brew install opencode` 或 `npm install -g opencode-ai`；安装后确认 `opencode acp` 可以直接执行。",
		links: [
			{ label: "官网", url: "https://opencode.ai" },
			{ label: "安装文档", url: "https://opencode.ai/docs" },
			{ label: "GitHub", url: "https://github.com/sst/opencode" },
		],
	},
	{
		id: "claude-agent",
		label: "Claude Agent",
		command: "claude-agent-acp",
		summary:
			"Claude Agent ACP 是 Zed 维护的 ACP 适配器，用来把 Claude Agent SDK 暴露给 ACP 客户端。",
		installCommand: "npm install -g @zed-industries/claude-agent-acp",
		installHint:
			"也可以从 GitHub Releases 下载单文件可执行程序；安装后确认 `claude-agent-acp` 可以直接执行。",
		links: [
			{
				label: "README",
				url: "https://github.com/zed-industries/claude-agent-acp#readme",
			},
			{
				label: "Releases",
				url: "https://github.com/zed-industries/claude-agent-acp/releases",
			},
			{
				label: "npm",
				url: "https://www.npmjs.com/package/@zed-industries/claude-agent-acp",
			},
		],
	},
	{
		id: "codex",
		label: "Codex",
		command: "codex-acp",
		summary:
			"Codex ACP 是 Zed 维护的 ACP 适配器，负责把 Codex CLI 暴露成可被 ACP 客户端调用的 Agent。",
		installCommand: "npm install -g @zed-industries/codex-acp",
		installHint:
			"官方 README 主推 GitHub Releases 或 `npx @zed-industries/codex-acp`；这里给出常驻安装命令，目标是装完后能直接执行 `codex-acp`。",
		links: [
			{
				label: "README",
				url: "https://github.com/zed-industries/codex-acp#readme",
			},
			{
				label: "Releases",
				url: "https://github.com/zed-industries/codex-acp/releases",
			},
			{
				label: "npm",
				url: "https://www.npmjs.com/package/@zed-industries/codex-acp",
			},
		],
	},
] as const;

const defaultRagIgnoreGlobPlaceholder = defaultRagIgnoreGlobs.join("\n");

function formatAcpAgentOptionLabel(
	option: (typeof acpAgentOptions)[number],
	alreadyAdded: boolean,
): string {
	return `${option.label} · ${option.command}${alreadyAdded ? " · 已配置" : ""}`;
}

function renderPresetInstallGuide(
	selectedPresetInstallOption: (typeof acpAgentOptions)[number] | null,
	selectedPresetOptionId: string,
) {
	if (selectedPresetInstallOption) {
		return (
			<>
				<span className="settings-acp-preset-kicker">安装指引</span>
				<span className="settings-agent-meta">安装 {selectedPresetInstallOption.label}</span>
				<p className="settings-install-guide-summary">
					{selectedPresetInstallOption.summary} 相关入口：
					{selectedPresetInstallOption.links.map((link, index) => (
						<span key={link.url}>
							{index === 0
								? " "
								: index === selectedPresetInstallOption.links.length - 1
									? " 和 "
									: "、"}
							<button
								className="settings-text-link"
								onClick={() => void openUrl(link.url)}
								type="button"
							>
								{link.label}
							</button>
						</span>
					))}
					。
				</p>
				<code className="settings-install-guide-command">
					{selectedPresetInstallOption.installCommand}
				</code>
				<span className="settings-help-text settings-help-text-tight">
					{selectedPresetInstallOption.installHint}
				</span>
			</>
		);
	}

	if (selectedPresetOptionId === "__custom__") {
		return (
			<>
				<span className="settings-acp-preset-kicker">安装指引</span>
				<span className="settings-agent-meta">自定义 Agent</span>
				<span className="settings-help-text settings-help-text-tight">
					自定义 Agent 不提供预设安装提示。
				</span>
			</>
		);
	}

	return (
		<>
			<span className="settings-acp-preset-kicker">安装指引</span>
			<span className="settings-help-text settings-help-text-tight">
				选择 Agent 后显示对应介绍、安装命令和官方链接。
			</span>
		</>
	);
}

function renderMcpTransportGuide(selectedTransport: McpTransport) {
	const transportMeta = getMcpTransportMeta(selectedTransport);

	return (
		<>
			<span className="settings-acp-preset-kicker">连接说明</span>
			<p className="settings-install-guide-summary">{transportMeta.summary}</p>
			<span className="settings-help-text settings-help-text-tight">
				{transportMeta.fieldsHint}
			</span>
			<code className="settings-install-guide-command">{transportMeta.example}</code>
		</>
	);
}

function renderSkillTreeNode(node: SkillTreeNode) {
	if (node.kind === "file") {
		return (
			<div className="settings-skill-tree-file" key={node.relativePath || node.name}>
				<span className="settings-skill-tree-bullet" aria-hidden="true">
					•
				</span>
				<span>{node.name}</span>
			</div>
		);
	}

	return (
		<details className="settings-skill-tree-directory" key={node.relativePath || node.name} open>
			<summary className="settings-skill-tree-summary">
				<span className="settings-skill-tree-caret" aria-hidden="true">
					▾
				</span>
				<span>{node.name}</span>
			</summary>
			<div className="settings-skill-tree-children">
				{node.children.length > 0 ? (
					node.children.map((child) => renderSkillTreeNode(child))
				) : (
					<span className="settings-help-text settings-help-text-tight">空目录</span>
				)}
			</div>
		</details>
	);
}

function getErrorMessage(error: unknown, fallbackMessage: string) {
	return error instanceof Error ? error.message : fallbackMessage;
}

const validMcpRemoteUrlProtocols = new Set(["http:", "https:"]);

interface AcpAgentDraft {
	id: string;
	name: string;
	command: string;
}

type McpTransport = AcpMcpServerConfig["transport"];
type AcpFieldKey = "name" | "command";
type McpFieldKey = "name" | "command" | "url" | "envText" | "headersText";
type LlmFieldKey = "name" | "baseUrl" | "model";
type LlmModelFieldKey = "model";
type RagFieldKey = "embeddingProviderId" | "sourceDirectories" | "ignoreGlobs";
type LlmProviderKind =
	| "llm_responses_stateless"
	| "llm_responses_stateful"
	| "llm_chat_completions"
	| "embedding";

type FieldIssueMap<FieldKey extends string> = Partial<Record<FieldKey, string>>;

interface AcpMcpServerDraft {
	id: string;
	transport: McpTransport;
	name: string;
	command: string;
	url: string;
	argsText: string;
	envText: string;
	headersText: string;
}

interface AcpDraftValidation {
	totalIssues: number;
	agentIssues: Record<string, string[]>;
	agentFieldIssues: Record<string, FieldIssueMap<AcpFieldKey>>;
}

interface SavedAcpDraftState {
	agents: AcpAgentDraft[];
	defaultAgentId: string | null;
}

interface SavedMcpDraftState {
	servers: AcpMcpServerDraft[];
}

interface LlmDraftValidation {
	totalIssues: number;
	providerIssues: Record<string, string[]>;
	providerFieldIssues: Record<string, FieldIssueMap<LlmFieldKey>>;
}

interface SavedLlmDraftState {
	providers: LlmProviderConfig[];
}

interface RagDraftValidation {
	totalIssues: number;
	issues: string[];
	fieldIssues: FieldIssueMap<RagFieldKey>;
}

interface SavedRagDraftState {
	sourceDirectories: string[];
	ignoreGlobs: string[];
	embeddingProviderId: string | null;
}

const defaultPromptsSettings: PromptsSettings = {
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

interface McpDraftValidation {
	totalIssues: number;
	serverIssues: Record<string, string[]>;
	serverFieldIssues: Record<string, FieldIssueMap<McpFieldKey>>;
}

interface AcpInlineNotice {
	tone: "info" | "warn";
	text: string;
}

interface PendingMcpFocusTarget {
	serverId: string;
	fieldKey: McpFieldKey;
	scrollToForm: boolean;
}

interface AcpIssueFocusTarget {
	agentId: string;
	fieldKey: AcpFieldKey;
}

interface McpIssueFocusTarget {
	serverId: string;
	serverFieldKey: McpFieldKey;
}

interface LlmIssueFocusTarget {
	providerId: string;
	fieldKey: LlmFieldKey;
}

const settingsSections: ReadonlyArray<{
	id: SettingsSectionId;
	label: string;
}> = [
	{ id: "general", label: "通用" },
	{ id: "prompts", label: "AI 功能" },
	{ id: "llm", label: "LLM" },
	{ id: "rag", label: "RAG" },
	{ id: "acp", label: "ACP Agent" },
	{ id: "mcp", label: "MCP" },
	{ id: "skills", label: "Skill" },
	{ id: "about", label: "关于" },
] as const;

const settingsQuickLinks: Readonly<Record<SettingsSectionId, readonly SettingsQuickLink[]>> = {
	general: [
		{ id: "general-shortcuts", label: "快捷键", hint: "启动、截图、翻译" },
		{ id: "general-appearance", label: "外观", hint: "主题与字号" },
		{ id: "general-ocr", label: "OCR", hint: "截图识别" },
	],
	prompts: [
		{ id: "prompts-translation", label: "翻译配置", hint: "模型与提示词" },
		{ id: "prompts-rag-answer", label: "文档问答配置", hint: "模型与提示词" },
	],
	llm: [
		{ id: "llm-catalog", label: "模型列表", hint: "已配置条目" },
		{ id: "llm-editor", label: "当前编辑", hint: "连接信息与能力" },
	],
	rag: [
		{ id: "rag-summary", label: "当前配置", hint: "摘要与约束" },
		{ id: "rag-pipeline", label: "索引流程", hint: "Embedding 与目录" },
		{ id: "rag-scan-result", label: "扫描结果", hint: "最近一次重建" },
	],
	acp: [
		{ id: "acp-catalog", label: "已配置", hint: "Agent 列表" },
		{ id: "acp-presets", label: "快速填充", hint: "预设与安装提示" },
		{ id: "acp-form", label: "当前表单", hint: "名称与命令" },
	],
	mcp: [],
	skills: [],
	about: [{ id: "about-overview", label: "关于 Wabity", hint: "版本与定位" }],
} as const;

function getSettingsTabId(sectionId: SettingsSectionId) {
	return `settings-tab-${sectionId}`;
}

function getSettingsPanelId(sectionId: SettingsSectionId) {
	return `settings-panel-${sectionId}`;
}

function joinDescribedByIds(...ids: Array<string | null | undefined | false>) {
	const joinedIds = ids.filter((id): id is string => Boolean(id)).join(" ");
	return joinedIds || undefined;
}

function buildFieldIssueId(sectionId: string, fieldKey: string, itemId?: string) {
	return itemId
		? `settings-${sectionId}-${itemId}-${fieldKey}-error`
		: `settings-${sectionId}-${fieldKey}-error`;
}

const DISCARD_DRAFT_BUTTON_LABEL = "恢复已保存版本";

function SettingsDraftActionCard({
	title,
	description,
	actions,
}: {
	title: string;
	description: string;
	actions?: ReactNode;
}) {
	return (
		<div className="settings-editor-card settings-editor-card-subtle settings-draft-action-card">
			<div className="settings-editor-card-header">
				<div className="settings-draft-action-copy">
					<span className="settings-section-kicker">草稿状态</span>
					<strong className="settings-draft-action-title">{title}</strong>
					<span className="settings-help-text settings-help-text-tight">{description}</span>
				</div>
				{actions ? <div className="settings-draft-action-actions">{actions}</div> : null}
			</div>
		</div>
	);
}

function SettingsQuickJumpList({
	activeBlockId,
	compact = false,
	links,
	onSelect,
}: SettingsQuickJumpListProps) {
	return (
		<div className={`settings-jump-list ${compact ? "settings-jump-list-compact" : ""}`}>
			{links.map((link) => {
				const isActive = activeBlockId === link.id;
				return (
					<button
						aria-current={isActive ? "location" : undefined}
						className={`settings-jump-button ${compact ? "settings-jump-button-compact" : ""} ${isActive ? "settings-jump-button-active" : ""}`}
						key={link.id}
						onClick={() => onSelect(link.id)}
						type="button"
					>
						<span className="settings-jump-button-label">{link.label}</span>
						{compact ? null : <span className="settings-jump-button-meta">{link.hint}</span>}
					</button>
				);
			})}
		</div>
	);
}

function ShortcutRecorderField({
	isRecording,
	isSaving,
	label,
	onActivate,
	shortcutValue,
	statusId,
	triggerId,
}: {
	isRecording: boolean;
	isSaving: boolean;
	label: string;
	onActivate: () => void;
	shortcutValue: string;
	statusId: string;
	triggerId: string;
}) {
	const statusText = isRecording
		? "正在录制，直接按下目标快捷键，按 Escape 取消。"
		: isSaving
			? "正在保存快捷键。"
			: "按 Enter 或空格开始录制，然后直接按下目标快捷键。";
	const actionLabel = isRecording ? "正在录制" : isSaving ? "保存中" : "开始录制";

	return (
		<div className="settings-item settings-item-stacked">
			<div className="settings-shortcut-field">
				<div className="settings-shortcut-copy">
					<span className="settings-label">{label}</span>
					<span
						aria-live="polite"
						className="settings-help-text settings-help-text-tight"
						id={statusId}
						role="status"
					>
						{statusText}
					</span>
				</div>
				<button
					aria-describedby={statusId}
					aria-pressed={isRecording}
					className={`settings-input settings-shortcut-trigger ${isRecording ? "recording" : ""}`}
					disabled={isSaving}
					id={triggerId}
					onClick={onActivate}
					type="button"
				>
					<span className="settings-shortcut-trigger-value">
						{shortcutValue.trim() || "未设置"}
					</span>
					<span className="settings-shortcut-trigger-action">{actionLabel}</span>
				</button>
			</div>
		</div>
	);
}

const ragSupportedFileExtensions = ["md", "mdx", "txt", "markdown", "rst", "adoc"] as const;
const emptyWorkspaceState: WorkspaceState = {
	rootPath: "",
	recentRoots: [],
	homePath: null,
	displayHomeAsTilde: false,
};

const mcpTransportOptions: ReadonlyArray<{
	transport: McpTransport;
	label: string;
	description: string;
	summary: string;
	fieldsHint: string;
	example: string;
}> = [
	{
		transport: "stdio",
		label: "本地进程",
		description: "通过命令启动 MCP server",
		summary:
			"适合本机已有命令行 MCP server 的场景，Wabity 会把命令、参数和环境变量作为全局 MCP 条目保存。",
		fieldsHint: "需要填写命令；可选填写参数和环境变量。",
		example: "npx -y @modelcontextprotocol/server-filesystem ~/Desktop",
	},
	{
		transport: "http",
		label: "HTTP",
		description: "通过 URL 连接远程 MCP server",
		summary: "适合已经部署好的远程 MCP 服务，保存后会把 URL 和请求头随会话一起交给当前 Agent。",
		fieldsHint: "需要填写 URL；可选填写请求头。",
		example: "https://example.com/mcp",
	},
	{
		transport: "sse",
		label: "SSE",
		description: "通过 SSE 流连接远程 MCP server",
		summary: "适合使用服务端事件流暴露能力的远程 MCP 服务，字段和 HTTP 类似，但连接语义是 SSE。",
		fieldsHint: "需要填写 URL；可选填写请求头。",
		example: "https://example.com/sse",
	},
];

function formatDirectCommand(program: string, args: string[]) {
	return [program, ...args]
		.filter((value) => value.trim().length > 0)
		.map((value) => (/[\s"]/u.test(value) ? JSON.stringify(value) : value))
		.join(" ");
}

function deriveProgramFromCommand(command: string) {
	const normalized = command.trim();
	if (!normalized) {
		return "";
	}

	const [program] = normalized.split(/\s+/, 1);
	return program ?? "";
}

function buildAgentCommand(agent: Pick<AcpAgentConfig, "program" | "args" | "shellCommand">) {
	return agent.shellCommand?.trim() || formatDirectCommand(agent.program, agent.args);
}

function nextDraftId() {
	return `agent-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function nextServerDraftId() {
	return `mcp-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function nextLlmDraftId() {
	return `llm-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function parseTextLines(value: string) {
	return value
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0);
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

function resolveDocumentsDirectoryPath(workspace: WorkspaceState) {
	const homePath = workspace.homePath?.trim() ?? "";
	if (homePath) {
		return joinPathForDisplay(homePath, "Documents");
	}

	return "~/Documents";
}

function resolveRagDirectoryPickerDefaultPath(workspace: WorkspaceState) {
	return resolveDocumentsDirectoryPath(workspace);
}

function formatTextLines(lines: string[]) {
	return lines.join("\n");
}

function parseKeyValueLines(value: string, label: string) {
	return value
		.split("\n")
		.map((line) => line.trim())
		.filter((line) => line.length > 0)
		.map((line, index) => {
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

function createMcpServerDraft(transport: McpTransport = "stdio"): AcpMcpServerDraft {
	return {
		id: nextServerDraftId(),
		transport,
		name: "",
		command: "",
		url: "",
		argsText: "",
		envText: "",
		headersText: "",
	};
}

function createMcpServerDraftFromConfig(server: AcpMcpServerConfig): AcpMcpServerDraft {
	switch (server.transport) {
		case "stdio":
			return {
				id: nextServerDraftId(),
				transport: "stdio",
				name: server.name,
				command: server.command,
				url: "",
				argsText: formatTextLines(server.args),
				envText: formatKeyValueLines(server.env),
				headersText: "",
			};
		case "http":
			return {
				id: nextServerDraftId(),
				transport: "http",
				name: server.name,
				command: "",
				url: server.url,
				argsText: "",
				envText: "",
				headersText: formatKeyValueLines(server.headers),
			};
		case "sse":
			return {
				id: nextServerDraftId(),
				transport: "sse",
				name: server.name,
				command: "",
				url: server.url,
				argsText: "",
				envText: "",
				headersText: formatKeyValueLines(server.headers),
			};
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

function cloneSavedAcpDraftState(state: SavedAcpDraftState): SavedAcpDraftState {
	return {
		defaultAgentId: state.defaultAgentId,
		agents: state.agents.map(cloneAgentDraft),
	};
}

function cloneSavedMcpDraftState(state: SavedMcpDraftState): SavedMcpDraftState {
	return {
		servers: state.servers.map(cloneMcpServerDraft),
	};
}

function cloneLlmProviderDraft(provider: LlmProviderConfig): LlmProviderConfig {
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

function serializeMcpServerDraft(server: AcpMcpServerDraft): AcpMcpServerConfig {
	if (server.transport === "stdio") {
		return {
			transport: "stdio",
			name: server.name.trim(),
			command: server.command.trim(),
			args: parseTextLines(server.argsText),
			env: parseKeyValueLines(server.envText, "MCP env"),
		};
	}

	if (server.transport === "http") {
		return {
			transport: "http",
			name: server.name.trim(),
			url: requireValidMcpRemoteUrl(server.url, "http"),
			headers: parseKeyValueLines(server.headersText, "MCP headers"),
		};
	}

	return {
		transport: "sse",
		name: server.name.trim(),
		url: requireValidMcpRemoteUrl(server.url, "sse"),
		headers: parseKeyValueLines(server.headersText, "MCP headers"),
	};
}

function createAgentDraft(name = "", command = ""): AcpAgentDraft {
	return {
		id: nextDraftId(),
		name,
		command,
	};
}

function createLlmProviderDraft(): LlmProviderConfig {
	return {
		id: nextLlmDraftId(),
		name: "",
		baseUrl: "https://api.openai.com/v1",
		apiKey: "",
		modelType: "llm",
		protocol: "responses",
		model: "",
		modelIdentityHint: null,
		supportsMultimodal: false,
		supportsStateful: false,
	};
}

function providerIsLlmModel(provider: Pick<LlmProviderConfig, "modelType">) {
	return provider.modelType === "llm";
}

function providerIsEmbeddingModel(provider: Pick<LlmProviderConfig, "modelType">) {
	return provider.modelType === "embedding";
}

function providerHasLlmModel(
	provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol">,
) {
	return providerIsLlmModel(provider) && provider.model.trim().length > 0;
}

function providerUsesResponsesProtocol(
	provider: Pick<LlmProviderConfig, "modelType" | "protocol">,
) {
	return providerIsLlmModel(provider) && provider.protocol === "responses";
}

function providerHasResponsesModel(
	provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol">,
) {
	return providerHasLlmModel(provider) && providerUsesResponsesProtocol(provider);
}

function providerHasEmbeddingModel(provider: Pick<LlmProviderConfig, "modelType" | "model">) {
	return providerIsEmbeddingModel(provider) && provider.model.trim().length > 0;
}

function providerCanHandleAiTask(
	provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol">,
) {
	return providerHasLlmModel(provider);
}

function providerCanHandleOcr(
	provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol" | "supportsMultimodal">,
) {
	return providerHasResponsesModel(provider) && provider.supportsMultimodal;
}

function providerCanHandleRagEmbedding(provider: Pick<LlmProviderConfig, "modelType" | "model">) {
	return providerHasEmbeddingModel(provider);
}

function sanitizeLlmProviderDraft(provider: LlmProviderConfig) {
	const sanitized = { ...provider };
	if (!providerUsesResponsesProtocol(sanitized)) {
		sanitized.supportsMultimodal = false;
		sanitized.supportsStateful = false;
	}
	if (sanitized.supportsMultimodal && !providerCanHandleOcr(sanitized)) {
		sanitized.supportsMultimodal = false;
	}
	if (sanitized.supportsStateful && !providerHasResponsesModel(sanitized)) {
		sanitized.supportsStateful = false;
	}
	if (!sanitized.model.trim()) {
		sanitized.modelIdentityHint = null;
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

function reconcileLlmSettings(settings: LlmSettings): LlmSettings {
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

function reconcileOcrSettings(settings: OcrSettings, providers: LlmProviderConfig[]): OcrSettings {
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

function reconcileRagSettings(settings: RagSettings, providers: LlmProviderConfig[]): RagSettings {
	if (!settings.embeddingProviderId) {
		return settings;
	}

	if (
		providers.some(
			(provider) =>
				provider.id === settings.embeddingProviderId && providerCanHandleRagEmbedding(provider),
		)
	) {
		return settings;
	}

	return {
		...settings,
		embeddingProviderId: null,
	};
}

function summarizeLlmProviderProfile(provider: LlmProviderConfig) {
	if (providerHasLlmModel(provider)) {
		const kindLabel = getLlmProviderKindLabel(getLlmProviderKind(provider));
		return provider.supportsMultimodal ? `${kindLabel} · 多模态` : kindLabel;
	}
	if (providerHasEmbeddingModel(provider)) {
		return "Embedding";
	}

	return providerIsEmbeddingModel(provider) ? "Embedding · 未配置模型" : "LLM · 未配置模型";
}

function getLlmProviderKind(
	provider: Pick<LlmProviderConfig, "modelType" | "protocol" | "supportsStateful">,
): LlmProviderKind {
	if (provider.modelType === "embedding") {
		return "embedding";
	}

	if (provider.protocol === "chat_completions") {
		return "llm_chat_completions";
	}

	return provider.supportsStateful ? "llm_responses_stateful" : "llm_responses_stateless";
}

function getLlmProviderKindLabel(kind: LlmProviderKind) {
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

function applyLlmProviderKind(
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

function getLlmProviderUsageBadges(
	provider: Pick<
		LlmProviderConfig,
		"modelType" | "protocol" | "supportsMultimodal" | "supportsStateful"
	>,
) {
	if (providerIsEmbeddingModel(provider)) {
		return ["RAG 索引", "RAG 检索"];
	}

	const providerKind = getLlmProviderKind(provider);
	const badges =
		providerKind === "llm_chat_completions"
			? ["翻译", "RAG 问答", "Chat Completions"]
			: provider.supportsMultimodal
				? ["翻译", "RAG 问答", "OCR", "Responses"]
				: ["翻译", "RAG 问答", "Responses"];
	if (providerKind === "llm_responses_stateful") {
		badges.push("Stateful");
	}
	if (providerKind === "llm_responses_stateless") {
		badges.push("Stateless");
	}
	return badges;
}

function getLlmProviderUsageDescription(
	provider: Pick<
		LlmProviderConfig,
		"modelType" | "protocol" | "supportsMultimodal" | "supportsStateful"
	>,
) {
	if (providerIsEmbeddingModel(provider)) {
		return "Embedding 条目只会出现在 RAG 的 embedding 列表，不会进入翻译 LLM、问答 LLM 或 OCR。";
	}

	const providerKind = getLlmProviderKind(provider);
	if (providerKind === "llm_chat_completions") {
		return "这个条目会走 OpenAI 兼容 chat/completions 协议，当前可供翻译和 RAG 问答复用；继续追问时始终回退到显式历史，不支持 response_id 续链，也不会进入 OCR 列表。";
	}

	const statefulText =
		providerKind === "llm_responses_stateful"
			? "这是 responses 的 stateful 版本，继续追问时会优先复用上一轮 response_id。"
			: "这是 responses 的 stateless 版本，继续追问时不会复用上一轮 response_id，而是回退到显式历史。";
	return provider.supportsMultimodal
		? `这个条目会走 OpenAI 兼容 responses 协议，当前可供翻译、RAG 问答和 OCR 复用。${statefulText}`
		: `这个条目会走 OpenAI 兼容 responses 协议，当前可供翻译和 RAG 问答复用；启用多模态后才会进入 OCR 列表。${statefulText}`;
}

function getLlmProviderModelPlaceholder(provider: Pick<LlmProviderConfig, "modelType">) {
	return provider.modelType === "embedding" ? "text-embedding-3-small" : "gpt-4.1-mini";
}

function getMcpTransportMeta(transport: McpTransport) {
	return (
		mcpTransportOptions.find((option) => option.transport === transport) ?? mcpTransportOptions[0]
	);
}

function parseMcpRemoteUrl(url: string) {
	try {
		return new URL(url.trim());
	} catch {
		return null;
	}
}

function validateMcpRemoteUrl(url: string, transport: Exclude<McpTransport, "stdio">) {
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

function requireValidMcpRemoteUrl(url: string, transport: Exclude<McpTransport, "stdio">) {
	const issue = validateMcpRemoteUrl(url, transport);
	if (issue) {
		throw new Error(issue);
	}

	return url.trim();
}

function getMcpServerDraftTitle(server: AcpMcpServerDraft) {
	return server.name.trim() || "未命名服务";
}

function summarizeMcpRemoteUrl(url: string) {
	const trimmed = url.trim();
	if (!trimmed) {
		return "等待填写 URL";
	}

	const parsed = parseMcpRemoteUrl(trimmed);
	if (!parsed) {
		return trimmed;
	}

	return `${parsed.host}${parsed.pathname === "/" ? "" : parsed.pathname}`;
}

function summarizeMcpServerDraft(server: AcpMcpServerDraft) {
	if (server.transport === "stdio") {
		return server.command.trim() || "等待填写命令";
	}

	return summarizeMcpRemoteUrl(server.url);
}

function buildSavedAcpDraftState(
	agents: AcpAgentDraft[],
	defaultAgentId: string | null,
): SavedAcpDraftState {
	return cloneSavedAcpDraftState({
		agents,
		defaultAgentId,
	});
}

function buildSavedMcpDraftState(servers: AcpMcpServerDraft[]): SavedMcpDraftState {
	return cloneSavedMcpDraftState({
		servers,
	});
}

function buildSavedLlmDraftState(providers: LlmProviderConfig[]): SavedLlmDraftState {
	return cloneSavedLlmDraftState({
		providers,
	});
}

function buildSavedRagDraftState(settings: RagSettings): SavedRagDraftState {
	return cloneSavedRagDraftState({
		sourceDirectories: settings.sourceDirectories,
		ignoreGlobs: settings.ignoreGlobs,
		embeddingProviderId: settings.embeddingProviderId,
	});
}

function buildAcpDraftSnapshot(agents: AcpAgentDraft[]) {
	return JSON.stringify({
		agents: agents.map((agent) => ({
			id: agent.id,
			name: agent.name,
			command: agent.command,
		})),
	});
}

function buildMcpDraftSnapshot(servers: AcpMcpServerDraft[]) {
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
	});
}

function buildLlmDraftSnapshot(settings: Pick<LlmSettings, "providers">) {
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
			supportsMultimodal: provider.supportsMultimodal,
			supportsStateful: provider.supportsStateful,
		})),
	});
}

function buildPromptsDraftSnapshot(
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

function buildTranslationTaskDraftSnapshot(
	settings: Pick<PromptsSettings, "translationPrompt">,
	llmSettings: Pick<LlmSettings, "translationProviderId">,
) {
	return JSON.stringify({
		translationPrompt: settings.translationPrompt,
		translationProviderId: llmSettings.translationProviderId,
	});
}

function buildQuestionAnswerTaskDraftSnapshot(
	settings: Pick<PromptsSettings, "ragAnswerSystemPrompt">,
	llmSettings: Pick<LlmSettings, "questionAnswerProviderId">,
) {
	return JSON.stringify({
		ragAnswerSystemPrompt: settings.ragAnswerSystemPrompt,
		questionAnswerProviderId: llmSettings.questionAnswerProviderId,
	});
}

function buildRagDraftSnapshot(settings: RagSettings) {
	return JSON.stringify({
		sourceDirectories: settings.sourceDirectories,
		ignoreGlobs: settings.ignoreGlobs,
		embeddingProviderId: settings.embeddingProviderId,
	});
}

function findFirstAcpIssue(agents: AcpAgentDraft[]): AcpIssueFocusTarget | null {
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

function findFirstMcpIssue(servers: AcpMcpServerDraft[]): McpIssueFocusTarget | null {
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

function findFirstLlmIssue(settings: LlmSettings): LlmIssueFocusTarget | null {
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
	}

	return null;
}

function validateAcpAgents(agents: AcpAgentDraft[]): AcpDraftValidation {
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

function validateLlmSettings(settings: LlmSettings): LlmDraftValidation {
	const providerIssues: Record<string, string[]> = {};
	const providerFieldIssues: Record<string, FieldIssueMap<LlmFieldKey>> = {};
	let totalIssues = 0;

	settings.providers.forEach((provider, providerIndex) => {
		const currentProviderIssues: string[] = [];
		const currentProviderFieldIssues: FieldIssueMap<LlmFieldKey> = {};
		if (!provider.name.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个 LLM 条目缺少名称。`);
			currentProviderFieldIssues.name = "请输入条目名称。";
		}
		if (!provider.baseUrl.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个 LLM 条目缺少 Base URL。`);
			currentProviderFieldIssues.baseUrl = "请输入 Base URL。";
		}
		if (!provider.model.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个 LLM 条目缺少模型名。`);
			currentProviderFieldIssues.model = "请输入模型名。";
		}
		if (provider.supportsMultimodal && !providerHasResponsesModel(provider)) {
			currentProviderIssues.push(
				`第 ${providerIndex + 1} 个 LLM 条目只有 responses 协议才能开启多模态。`,
			);
		}
		if (provider.supportsStateful && !providerHasResponsesModel(provider)) {
			currentProviderIssues.push(
				`第 ${providerIndex + 1} 个 LLM 条目只有 responses 协议才能开启 stateful 请求。`,
			);
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

function validateRagSettings(settings: RagSettings, llmSettings: LlmSettings): RagDraftValidation {
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

function selectExistingIdOrFirst<T extends { id: string }>(items: T[], selectedId: string | null) {
	if (items.length === 0) {
		return null;
	}

	if (!selectedId || !items.some((item) => item.id === selectedId)) {
		return items[0]?.id ?? null;
	}

	return selectedId;
}

function validateMcpServers(servers: AcpMcpServerDraft[]): McpDraftValidation {
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

export function SettingsPage({ onBack, onAppearanceChange }: SettingsPageProps) {
	const [activeSection, setActiveSection] = useState<SettingsSectionId>("general");
	const [generalSettings, setGeneralSettings] = useState<GeneralSettings>({
		autoStart: false,
		showInDock: true,
		language: "zh-CN",
	});
	const [promptsSettings, setPromptsSettings] = useState<PromptsSettings>(defaultPromptsSettings);

	const [shortcutSettings, setShortcutSettings] = useState<ShortcutConfig>({
		toggle_launcher: "Alt+Space",
		ocr_capture: "Alt+R",
		ocr_translate: "Alt+D",
	});
	const [llmSettings, setLlmSettings] = useState<LlmSettings>({
		providers: [],
		translationProviderId: null,
		questionAnswerProviderId: null,
	});
	const [selectedLlmProviderId, setSelectedLlmProviderId] = useState<string | null>(null);
	const [savedLlmSnapshot, setSavedLlmSnapshot] = useState(() =>
		buildLlmDraftSnapshot({
			providers: [],
		}),
	);
	const [, setSavedLlmState] = useState<SavedLlmDraftState>(() => buildSavedLlmDraftState([]));
	const [llmModelOptions, setLlmModelOptions] = useState<Record<string, LlmProviderModelEntry[]>>(
		{},
	);
	const [llmModelErrors, setLlmModelErrors] = useState<Record<string, string>>({});
	const [loadingLlmModelProviderId, setLoadingLlmModelProviderId] = useState<string | null>(null);
	const [openLlmModelPickerId, setOpenLlmModelPickerId] = useState<string | null>(null);
	const [ragSettings, setRagSettings] = useState<RagSettings>({
		sourceDirectories: [],
		ignoreGlobs: [...defaultRagIgnoreGlobs],
		embeddingProviderId: null,
	});
	const [savedRagSnapshot, setSavedRagSnapshot] = useState(() =>
		buildRagDraftSnapshot({
			sourceDirectories: [],
			ignoreGlobs: [...defaultRagIgnoreGlobs],
			embeddingProviderId: null,
		}),
	);
	const [, setSavedRagState] = useState<SavedRagDraftState>(() =>
		buildSavedRagDraftState({
			sourceDirectories: [],
			ignoreGlobs: [...defaultRagIgnoreGlobs],
			embeddingProviderId: null,
		}),
	);
	const [ragScanResult, setRagScanResult] = useState<RagScanResult | null>(null);
	const [acpAgents, setAcpAgentsState] = useState<AcpAgentDraft[]>([]);
	const [defaultAgentId, setDefaultAgentId] = useState<string | null>(null);
	const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
	const [savedAcpSnapshot, setSavedAcpSnapshot] = useState(() => buildAcpDraftSnapshot([]));
	const [savedAcpState, setSavedAcpState] = useState<SavedAcpDraftState>(() =>
		buildSavedAcpDraftState([], null),
	);
	const [acpNotice, setAcpNotice] = useState<AcpInlineNotice | null>(null);
	const [selectedPresetOptionId, setSelectedPresetOptionId] = useState("__custom__");
	const [selectedMcpTransport, setSelectedMcpTransport] = useState<McpTransport>("stdio");
	const [mcpServers, setMcpServersState] = useState<AcpMcpServerDraft[]>([]);
	const [selectedMcpServerId, setSelectedMcpServerId] = useState<string | null>(null);
	const [savedMcpSnapshot, setSavedMcpSnapshot] = useState(() => buildMcpDraftSnapshot([]));
	const [savedMcpState, setSavedMcpState] = useState<SavedMcpDraftState>(() =>
		buildSavedMcpDraftState([]),
	);
	const [mcpNotice, setMcpNotice] = useState<AcpInlineNotice | null>(null);
	const [builtinRagMcpServerStatus, setBuiltinRagMcpServerStatus] =
		useState<BuiltinRagMcpServerStatus | null>(null);
	const [skillCatalog, setSkillCatalog] = useState<PublicSkillCatalog>({
		rootPath: "~/.agents/skills",
		exists: false,
		skills: [],
	});
	const [workspaceContext, setWorkspaceContext] = useState<WorkspaceState>(emptyWorkspaceState);
	const [selectedSkillId, setSelectedSkillId] = useState<string | null>(null);
	const [skillError, setSkillError] = useState<string | null>(null);

	const [appearanceSettings, setAppearanceSettings] = useState<AppearanceSettings>({
		theme: "auto",
		fontSize: "medium",
	});
	const [ocrSettings, setOcrSettings] = useState<OcrSettings>({
		provider: "system",
		llmProviderId: null,
	});
	const [persistedAppSettings, setPersistedAppSettings] = useState<AppSettings>({
		general: {
			autoStart: false,
			showInDock: true,
			language: "zh-CN",
		},
		appearance: {
			theme: "auto",
			fontSize: "medium",
		},
		prompts: defaultPromptsSettings,
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
	});

	// Track which shortcut is being edited
	const [editingShortcut, setEditingShortcut] = useState<keyof ShortcutConfig | null>(null);
	const [savingShortcutKey, setSavingShortcutKey] = useState<keyof ShortcutConfig | null>(null);
	const [savingSettings, setSavingSettings] = useState(false);
	const [savingTranslationConfig, setSavingTranslationConfig] = useState(false);
	const [savingQuestionAnswerConfig, setSavingQuestionAnswerConfig] = useState(false);
	const [savingLlm, setSavingLlm] = useState(false);
	const [savingOcr, setSavingOcr] = useState(false);
	const [savingRag, setSavingRag] = useState(false);
	const [scanningRag, setScanningRag] = useState(false);
	const [savingMcp, setSavingMcp] = useState(false);
	const [mcpPanelMode, setMcpPanelMode] = useState<McpPanelMode>("create");
	const [settingsError, setSettingsError] = useState<string | null>(null);
	const acpFieldRefs = useRef<Record<string, HTMLInputElement | HTMLTextAreaElement | null>>({});
	const mcpListOptionRefs = useRef<Record<string, HTMLButtonElement | null>>({});
	const pendingMcpFocusRef = useRef<PendingMcpFocusTarget | null>(null);
	const llmFieldRefs = useRef<Record<string, HTMLInputElement | null>>({});
	const ragFieldRefs = useRef<Record<RagFieldKey, HTMLSelectElement | HTMLTextAreaElement | null>>({
		embeddingProviderId: null,
		sourceDirectories: null,
		ignoreGlobs: null,
	});
	const llmModelMenuRef = useRef<HTMLDivElement | null>(null);
	const sectionTabRefs = useRef<Record<SettingsSectionId, HTMLButtonElement | null>>({
		general: null,
		prompts: null,
		llm: null,
		rag: null,
		acp: null,
		mcp: null,
		skills: null,
		about: null,
	});
	const sectionBlockRefs = useRef<Record<string, HTMLElement | null>>({});
	const contentRef = useRef<HTMLDivElement | null>(null);
	const shellRef = useRef<HTMLElement | null>(null);
	const frameSize = useSettingsWindowFrame(shellRef);
	const showLlmOcrFields = ocrSettings.provider === "llm_ocr";
	const ragSourceDirectoryPlaceholder = resolveDocumentsDirectoryPath(workspaceContext);
	const ragDirectoryPickerDefaultPath = resolveRagDirectoryPickerDefaultPath(workspaceContext);
	const activeQuickLinks = settingsQuickLinks[activeSection];
	const [activeSectionBlockId, setActiveSectionBlockId] = useState<string | null>(
		settingsQuickLinks.general[0]?.id ?? null,
	);

	function handleSectionTabKeyDown(
		event: ReactKeyboardEvent<HTMLButtonElement>,
		sectionId: SettingsSectionId,
	) {
		const currentIndex = settingsSections.findIndex((section) => section.id === sectionId);
		if (currentIndex < 0) {
			return;
		}

		let nextIndex: number | null = null;
		switch (event.key) {
			case "ArrowRight":
			case "ArrowDown":
				nextIndex = (currentIndex + 1) % settingsSections.length;
				break;
			case "ArrowLeft":
			case "ArrowUp":
				nextIndex = (currentIndex - 1 + settingsSections.length) % settingsSections.length;
				break;
			case "Home":
				nextIndex = 0;
				break;
			case "End":
				nextIndex = settingsSections.length - 1;
				break;
			default:
				return;
		}

		event.preventDefault();
		const nextSectionId = settingsSections[nextIndex]?.id;
		if (!nextSectionId) {
			return;
		}

		setActiveSection(nextSectionId);
		sectionTabRefs.current[nextSectionId]?.focus();
	}

	function bindSectionBlockRef(blockId: string) {
		return (element: HTMLElement | null) => {
			sectionBlockRefs.current[blockId] = element;
		};
	}

	function scrollToSectionBlock(blockId: string) {
		const target = sectionBlockRefs.current[blockId];
		if (!target) {
			return;
		}

		const prefersReducedMotion =
			typeof window !== "undefined" &&
			typeof window.matchMedia === "function" &&
			window.matchMedia("(prefers-reduced-motion: reduce)").matches;

		setActiveSectionBlockId(blockId);

		target.scrollIntoView({
			behavior: prefersReducedMotion ? "auto" : "smooth",
			block: "start",
			inline: "nearest",
		});
	}

	function handleViewSkill(skillId: string) {
		setSelectedSkillId(skillId);
		scrollToSectionBlock("skills-detail");
	}

	function handleSelectSection(sectionId: SettingsSectionId) {
		setActiveSection(sectionId);

		const firstBlockId = settingsQuickLinks[sectionId][0]?.id ?? null;
		setActiveSectionBlockId(firstBlockId);

		const root = contentRef.current;
		if (!root) {
			return;
		}

		root.scrollTo({
			top: 0,
		});
	}

	function applyAgentDrafts(nextAgents: AcpAgentDraft[], nextSelectedAgentId: string | null) {
		setAcpAgentsState(nextAgents);
		setSelectedAgentId(nextSelectedAgentId);
	}

	function applyMcpServerDrafts(
		nextServers: AcpMcpServerDraft[],
		nextSelectedServerId: string | null,
	) {
		setMcpServersState(nextServers);
		setSelectedMcpServerId(nextSelectedServerId);
	}

	function bindMcpListOptionRef(serverId: string) {
		return (node: HTMLButtonElement | null) => {
			mcpListOptionRefs.current[serverId] = node;
		};
	}

	function selectMcpServer(serverId: string, options?: { focusOption?: boolean }) {
		setMcpPanelMode("edit");
		setSelectedMcpServerId(serverId);
		if (options?.focusOption) {
			window.requestAnimationFrame(() => {
				mcpListOptionRefs.current[serverId]?.focus();
			});
		}
	}

	function queueMcpFieldFocus(
		serverId: string,
		fieldKey: McpFieldKey,
		options?: { scrollToForm?: boolean },
	) {
		setMcpPanelMode("edit");
		pendingMcpFocusRef.current = {
			serverId,
			fieldKey,
			scrollToForm: options?.scrollToForm ?? true,
		};
		setSelectedMcpServerId(serverId);
	}

	function handleEnterMcpCreateMode() {
		setMcpPanelMode("create");
	}

	function handleReturnToCurrentMcp() {
		if (selectedMcpServerId) {
			setMcpPanelMode("edit");
		}
	}

	function handleMcpCatalogKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>, serverId: string) {
		const currentIndex = regularMcpServers.findIndex((server) => server.id === serverId);
		if (currentIndex < 0) {
			return;
		}

		let nextIndex: number | null = null;
		switch (event.key) {
			case "ArrowDown":
			case "ArrowRight":
				nextIndex = (currentIndex + 1) % regularMcpServers.length;
				break;
			case "ArrowUp":
			case "ArrowLeft":
				nextIndex = (currentIndex - 1 + regularMcpServers.length) % regularMcpServers.length;
				break;
			case "Home":
				nextIndex = 0;
				break;
			case "End":
				nextIndex = regularMcpServers.length - 1;
				break;
			default:
				return;
		}

		event.preventDefault();
		const nextServerId = regularMcpServers[nextIndex]?.id;
		if (!nextServerId) {
			return;
		}

		selectMcpServer(nextServerId, { focusOption: true });
	}

	function clearLlmModelCatalog(providerId: string) {
		setLlmModelOptions((current) => {
			if (!(providerId in current)) {
				return current;
			}

			const next = { ...current };
			delete next[providerId];
			return next;
		});
		setLlmModelErrors((current) => {
			if (!(providerId in current)) {
				return current;
			}

			const next = { ...current };
			delete next[providerId];
			return next;
		});
		setLoadingLlmModelProviderId((current) => (current === providerId ? null : current));
		setOpenLlmModelPickerId((current) => (current?.startsWith(`${providerId}:`) ? null : current));
	}

	function buildLlmModelPickerId(providerId: string, fieldKey: LlmModelFieldKey) {
		return `${providerId}:${fieldKey}`;
	}

	function findLlmModelOption(providerId: string, model: string) {
		return (llmModelOptions[providerId] ?? []).find((option) => option.id === model) ?? null;
	}

	async function handleFetchLlmProviderModels(
		provider: LlmProviderConfig,
		options?: { openField?: LlmModelFieldKey },
	) {
		setSettingsError(null);
		setLoadingLlmModelProviderId(provider.id);
		setLlmModelErrors((current) => {
			const next = { ...current };
			delete next[provider.id];
			return next;
		});

		try {
			const models = await listLlmProviderModels(provider);
			setLlmModelOptions((current) => ({
				...current,
				[provider.id]: models,
			}));
			const matchedOption = models.find((option) => option.id === provider.model) ?? null;
			if (matchedOption) {
				setLlmSettings((current) =>
					reconcileLlmSettings({
						...current,
						providers: current.providers.map((candidate) =>
							candidate.id === provider.id
								? {
										...candidate,
										modelIdentityHint: matchedOption.identityHint,
									}
								: candidate,
						),
					}),
				);
			}
			if (options?.openField && models.length > 0) {
				setOpenLlmModelPickerId(buildLlmModelPickerId(provider.id, options.openField));
			}
		} catch (error: unknown) {
			setLlmModelErrors((current) => ({
				...current,
				[provider.id]: getErrorMessage(error, "模型列表拉取失败"),
			}));
			setLlmModelOptions((current) => {
				const next = { ...current };
				delete next[provider.id];
				return next;
			});
		} finally {
			setLoadingLlmModelProviderId((current) => (current === provider.id ? null : current));
		}
	}

	async function handleToggleLlmModelMenu(provider: LlmProviderConfig, fieldKey: LlmModelFieldKey) {
		if (loadingLlmModelProviderId === provider.id || !provider.baseUrl.trim()) {
			return;
		}

		const models = llmModelOptions[provider.id] ?? [];
		if (models.length === 0) {
			await handleFetchLlmProviderModels(provider, { openField: fieldKey });
			return;
		}

		const pickerId = buildLlmModelPickerId(provider.id, fieldKey);
		setOpenLlmModelPickerId((current) => (current === pickerId ? null : pickerId));
	}

	function handleLlmModelOptionClick(
		providerId: string,
		fieldKey: LlmModelFieldKey,
		option: LlmProviderModelEntry,
	) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) =>
					provider.id === providerId
						? {
								...provider,
								[fieldKey]: option.id,
								modelIdentityHint: option.identityHint,
							}
						: provider,
				),
			}),
		);
		setOpenLlmModelPickerId(null);
		setSettingsError(null);
	}

	function handleLlmModelInputKeyDown(
		event: ReactKeyboardEvent<HTMLInputElement>,
		provider: LlmProviderConfig,
		fieldKey: LlmModelFieldKey,
	) {
		if (event.key === "ArrowDown") {
			event.preventDefault();
			void handleToggleLlmModelMenu(provider, fieldKey);
			return;
		}

		if (event.key === "Escape") {
			setOpenLlmModelPickerId((current) =>
				current === buildLlmModelPickerId(provider.id, fieldKey) ? null : current,
			);
		}
	}

	// Load initial shortcut config
	useEffect(() => {
		void getAppSettings().then((settings) => {
			setGeneralSettings(settings.general);
			setAppearanceSettings(settings.appearance);
			applyAppearanceSettings(settings.appearance);
			onAppearanceChange?.(settings.appearance);
			setPromptsSettings(settings.prompts);
			setLlmSettings(settings.llm);
			setSelectedLlmProviderId(settings.llm.providers[0]?.id ?? null);
			setSavedLlmSnapshot(buildLlmDraftSnapshot(settings.llm));
			setSavedLlmState(buildSavedLlmDraftState(settings.llm.providers));
			setLlmModelOptions({});
			setLlmModelErrors({});
			setLoadingLlmModelProviderId(null);
			setOpenLlmModelPickerId(null);
			setOcrSettings(settings.ocr);
			setRagSettings(settings.rag);
			setSavedRagSnapshot(buildRagDraftSnapshot(settings.rag));
			setSavedRagState(buildSavedRagDraftState(settings.rag));
			setRagScanResult(null);
			setPersistedAppSettings(settings);
		});
		void getShortcut().then((config) => {
			setShortcutSettings(config);
		});
		void getAcpAgents().then((catalog) => {
			const draftAgents = catalog.agents.map((agent) => ({
				id: agent.id,
				name: agent.name,
				command: buildAgentCommand(agent),
			}));
			const nextDefaultAgentId = catalog.defaultAgentId ?? catalog.agents[0]?.id ?? null;
			applyAgentDrafts(draftAgents, draftAgents[0]?.id ?? null);
			setDefaultAgentId(nextDefaultAgentId);
			setSavedAcpSnapshot(buildAcpDraftSnapshot(draftAgents));
			setSavedAcpState(buildSavedAcpDraftState(draftAgents, nextDefaultAgentId));
		});
		void getAcpMcpServers().then((catalog) => {
			const draftServers = catalog.servers.map(createMcpServerDraftFromConfig);
			applyMcpServerDrafts(draftServers, draftServers[0]?.id ?? null);
			setMcpPanelMode(draftServers.length > 0 ? "edit" : "create");
			setSavedMcpSnapshot(buildMcpDraftSnapshot(draftServers));
			setSavedMcpState(buildSavedMcpDraftState(draftServers));
		});
		void getBuiltinRagMcpServerStatus()
			.then((status) => {
				setBuiltinRagMcpServerStatus(status);
			})
			.catch((error: unknown) => {
				setBuiltinRagMcpServerStatus({
					server: {
						transport: "http",
						name: "Wabity RAG Query",
						url: "http://127.0.0.1:43189/internal/mcp/rag",
						headers: [],
					},
					running: false,
					lastError: getErrorMessage(error, "内置 RAG MCP server 状态读取失败"),
				});
			});
		void getPublicSkillCatalog()
			.then((catalog) => {
				setSkillError(null);
				setSkillCatalog(catalog);
				setSelectedSkillId(catalog.skills[0]?.id ?? null);
			})
			.catch((error: unknown) => {
				setSkillError(getErrorMessage(error, "公共 skill 读取失败"));
			});
		void getWorkspace()
			.then((workspace) => {
				setWorkspaceContext(workspace);
			})
			.catch(() => {
				setWorkspaceContext(emptyWorkspaceState);
			});

		// Listen for shortcut updates from backend
		const unlistenPromise = onShortcutUpdated((config) => {
			setShortcutSettings(config);
		});

		return () => {
			void unlistenPromise.then((unlisten) => unlisten?.());
		};
	}, [onAppearanceChange]);

	// Handle keydown when recording shortcut
	useEffect(() => {
		if (!editingShortcut) return;

		const handleKeyDown = (e: KeyboardEvent) => {
			e.preventDefault();
			e.stopPropagation();

			if (e.key === "Escape") {
				setEditingShortcut(null);
				return;
			}

			if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) {
				return;
			}

			const modifiers: string[] = [];
			if (e.metaKey) modifiers.push("Cmd");
			if (e.ctrlKey) modifiers.push("Ctrl");
			if (e.altKey) modifiers.push("Alt");
			if (e.shiftKey) modifiers.push("Shift");

			// Get the key, handle special keys
			let key = e.key;
			if (key === " ") {
				key = "Space";
			} else if (key.length === 1) {
				// Single character, uppercase for letters
				key = key.toUpperCase();
			} else {
				// Special keys like Enter, Escape, etc.
				key = key.charAt(0).toUpperCase() + key.slice(1);
			}

			// Build shortcut string
			const shortcut = [...modifiers, key].join("+");

			// Stop editing
			setEditingShortcut(null);
			setSavingShortcutKey(editingShortcut);
			setSettingsError(null);

			void setShortcut(editingShortcut, shortcut)
				.then(() => {
					setShortcutSettings((prev) => ({
						...prev,
						[editingShortcut]: shortcut,
					}));
				})
				.catch((error: unknown) => {
					setSettingsError(getErrorMessage(error, "快捷键保存失败"));
				})
				.finally(() => {
					setSavingShortcutKey((current) => (current === editingShortcut ? null : current));
				});
		};

		const handleKeyUp = (e: KeyboardEvent) => {
			if (e.key === "Escape") {
				setEditingShortcut(null);
			}
		};

		window.addEventListener("keydown", handleKeyDown, true);
		window.addEventListener("keyup", handleKeyUp, true);

		return () => {
			window.removeEventListener("keydown", handleKeyDown, true);
			window.removeEventListener("keyup", handleKeyUp, true);
		};
	}, [editingShortcut]);

	useEffect(() => {
		const nextSelectedLlmProviderId = selectExistingIdOrFirst(
			llmSettings.providers,
			selectedLlmProviderId,
		);
		if (nextSelectedLlmProviderId !== selectedLlmProviderId) {
			setSelectedLlmProviderId(nextSelectedLlmProviderId);
		}
	}, [llmSettings.providers, selectedLlmProviderId]);

	useEffect(() => {
		if (
			openLlmModelPickerId !== null &&
			!openLlmModelPickerId.startsWith(`${selectedLlmProviderId ?? ""}:`)
		) {
			setOpenLlmModelPickerId(null);
		}
	}, [selectedLlmProviderId, openLlmModelPickerId]);

	useEffect(() => {
		if (
			openLlmModelPickerId !== null &&
			!llmSettings.providers.some((provider) => openLlmModelPickerId.startsWith(`${provider.id}:`))
		) {
			setOpenLlmModelPickerId(null);
		}
	}, [llmSettings.providers, openLlmModelPickerId]);

	useEffect(() => {
		function handlePointerDown(event: MouseEvent) {
			const target = event.target;
			if (!(target instanceof Node)) {
				return;
			}
			if (llmModelMenuRef.current?.contains(target)) {
				return;
			}
			setOpenLlmModelPickerId(null);
		}

		document.addEventListener("mousedown", handlePointerDown);
		return () => {
			document.removeEventListener("mousedown", handlePointerDown);
		};
	}, []);

	useEffect(() => {
		setOcrSettings((current) => reconcileOcrSettings(current, llmSettings.providers));
	}, [llmSettings.providers, ocrSettings.llmProviderId, ocrSettings.provider]);

	useEffect(() => {
		setRagSettings((current) => reconcileRagSettings(current, llmSettings.providers));
	}, [llmSettings.providers, ragSettings.embeddingProviderId]);

	useEffect(() => {
		const nextSelectedAgentId = selectExistingIdOrFirst(acpAgents, selectedAgentId);
		if (nextSelectedAgentId !== selectedAgentId) {
			setSelectedAgentId(nextSelectedAgentId);
		}
	}, [acpAgents, selectedAgentId]);

	useEffect(() => {
		const nextSelectedMcpServerId = selectExistingIdOrFirst(mcpServers, selectedMcpServerId);
		if (nextSelectedMcpServerId !== selectedMcpServerId) {
			setSelectedMcpServerId(nextSelectedMcpServerId);
		}
	}, [mcpServers, selectedMcpServerId]);

	useEffect(() => {
		const pendingTarget = pendingMcpFocusRef.current;
		if (!pendingTarget || pendingTarget.serverId !== selectedMcpServerId) {
			return;
		}

		if (pendingTarget.scrollToForm) {
			scrollToSectionBlock("mcp-form");
		}

		const refKey = buildAcpFieldRefKey("server", pendingTarget.serverId, pendingTarget.fieldKey);
		const focusTimer = window.setTimeout(
			() => {
				acpFieldRefs.current[refKey]?.focus();
			},
			pendingTarget.scrollToForm ? 180 : 0,
		);
		pendingMcpFocusRef.current = null;

		return () => {
			window.clearTimeout(focusTimer);
		};
	}, [selectedMcpServerId, mcpServers]);

	useEffect(() => {
		const nextSelectedSkillId = selectExistingIdOrFirst(skillCatalog.skills, selectedSkillId);
		if (nextSelectedSkillId !== selectedSkillId) {
			setSelectedSkillId(nextSelectedSkillId);
		}
	}, [selectedSkillId, skillCatalog.skills]);

	useEffect(() => {
		setActiveSectionBlockId(settingsQuickLinks[activeSection][0]?.id ?? null);
	}, [activeSection]);

	useEffect(() => {
		const root = contentRef.current;
		if (!root) {
			return;
		}

		const observedBlocks = activeQuickLinks
			.map((link) => sectionBlockRefs.current[link.id])
			.filter((element): element is HTMLElement => element instanceof HTMLElement);

		if (observedBlocks.length === 0) {
			return;
		}

		const observer = new IntersectionObserver(
			(entries) => {
				const candidate = entries
					.filter((entry) => entry.isIntersecting)
					.sort(
						(left, right) =>
							right.intersectionRatio - left.intersectionRatio ||
							left.boundingClientRect.top - right.boundingClientRect.top,
					)[0];

				if (!(candidate?.target instanceof HTMLElement)) {
					return;
				}

				const nextBlockId = candidate.target.id;
				setActiveSectionBlockId((current) => (current === nextBlockId ? current : nextBlockId));
			},
			{
				root,
				rootMargin: "-12% 0px -58% 0px",
				threshold: [0.2, 0.4, 0.65],
			},
		);

		observedBlocks.forEach((block) => observer.observe(block));
		return () => {
			observer.disconnect();
		};
	}, [activeQuickLinks]);

	const handleShortcutClick = (key: keyof ShortcutConfig) => {
		setEditingShortcut(key);
	};

	const isRecording = (key: keyof ShortcutConfig) => editingShortcut === key;
	const [savingAgent, setSavingAgent] = useState(false);
	const llmValidation = validateLlmSettings(llmSettings);
	const ragValidation = validateRagSettings(ragSettings, llmSettings);
	const acpValidation = validateAcpAgents(acpAgents);
	const mcpValidation = validateMcpServers(mcpServers);
	const promptsDraftSnapshot = buildPromptsDraftSnapshot(promptsSettings, llmSettings);
	const persistedPromptsDraftSnapshot = buildPromptsDraftSnapshot(
		persistedAppSettings.prompts,
		persistedAppSettings.llm,
	);
	const promptsHaveUnsavedChanges = promptsDraftSnapshot !== persistedPromptsDraftSnapshot;
	const translationTaskDraftSnapshot = buildTranslationTaskDraftSnapshot(
		promptsSettings,
		llmSettings,
	);
	const persistedTranslationTaskDraftSnapshot = buildTranslationTaskDraftSnapshot(
		persistedAppSettings.prompts,
		persistedAppSettings.llm,
	);
	const translationHasUnsavedChanges =
		translationTaskDraftSnapshot !== persistedTranslationTaskDraftSnapshot;
	const questionAnswerTaskDraftSnapshot = buildQuestionAnswerTaskDraftSnapshot(
		promptsSettings,
		llmSettings,
	);
	const persistedQuestionAnswerTaskDraftSnapshot = buildQuestionAnswerTaskDraftSnapshot(
		persistedAppSettings.prompts,
		persistedAppSettings.llm,
	);
	const questionAnswerHasUnsavedChanges =
		questionAnswerTaskDraftSnapshot !== persistedQuestionAnswerTaskDraftSnapshot;
	const savingAiTaskConfig = savingTranslationConfig || savingQuestionAnswerConfig;
	const llmDraftSnapshot = buildLlmDraftSnapshot(llmSettings);
	const llmHasUnsavedChanges = llmDraftSnapshot !== savedLlmSnapshot;
	const ragDraftSnapshot = buildRagDraftSnapshot(ragSettings);
	const ragHasUnsavedChanges = ragDraftSnapshot !== savedRagSnapshot;
	const acpDraftSnapshot = buildAcpDraftSnapshot(acpAgents);
	const acpHasUnsavedChanges = acpDraftSnapshot !== savedAcpSnapshot;
	const mcpDraftSnapshot = buildMcpDraftSnapshot(mcpServers);
	const mcpHasUnsavedChanges = mcpDraftSnapshot !== savedMcpSnapshot;
	const selectedLlmProvider =
		llmSettings.providers.find((provider) => provider.id === selectedLlmProviderId) ?? null;
	const selectedLlmProviderKind = selectedLlmProvider
		? getLlmProviderKind(selectedLlmProvider)
		: null;
	const selectedLlmProviderModels = selectedLlmProvider
		? (llmModelOptions[selectedLlmProvider.id] ?? [])
		: [];
	const selectedLlmProviderModelsError = selectedLlmProvider
		? (llmModelErrors[selectedLlmProvider.id] ?? null)
		: null;
	const isLoadingSelectedLlmProviderModels =
		selectedLlmProvider !== null && loadingLlmModelProviderId === selectedLlmProvider.id;
	const eligibleOcrProviders = llmSettings.providers.filter((provider) =>
		providerCanHandleOcr(provider),
	);
	const eligibleAiTaskProviders = llmSettings.providers.filter((provider) =>
		providerCanHandleAiTask(provider),
	);
	const selectedTranslationProvider =
		eligibleAiTaskProviders.find((provider) => provider.id === llmSettings.translationProviderId) ??
		null;
	const selectedQuestionAnswerProvider =
		eligibleAiTaskProviders.find(
			(provider) => provider.id === llmSettings.questionAnswerProviderId,
		) ?? null;
	const eligibleRagEmbeddingProviders = llmSettings.providers.filter((provider) =>
		providerCanHandleRagEmbedding(provider),
	);
	const selectedRagEmbeddingProvider =
		eligibleRagEmbeddingProviders.find(
			(provider) => provider.id === ragSettings.embeddingProviderId,
		) ?? null;
	const selectedRagEmbeddingProviderLabel =
		selectedRagEmbeddingProvider?.name ||
		selectedRagEmbeddingProvider?.model ||
		selectedRagEmbeddingProvider?.baseUrl ||
		"未选择 Embedding";
	const ragSourceDirectoryCount = ragSettings.sourceDirectories.length;
	const ragIgnoreGlobCount = ragSettings.ignoreGlobs.length;
	const ragSupportedExtensionsLabel = ragSupportedFileExtensions
		.map((extension) => `.${extension}`)
		.join("、");
	const ragScanSummaryItems = ragScanResult
		? [
				{ label: "数据库", value: ragScanResult.databasePath },
				{ label: "目录数", value: String(ragScanResult.sourceCount) },
				{ label: "扫描文件", value: String(ragScanResult.scannedFileCount) },
				{ label: "已索引文件", value: String(ragScanResult.indexedFileCount) },
				{ label: "跳过文件", value: String(ragScanResult.skippedFileCount) },
				{ label: "向量块", value: String(ragScanResult.chunkCount) },
			]
		: [];
	const selectedLlmProviderUsedByPersistedRag =
		selectedLlmProvider !== null &&
		persistedAppSettings.rag.embeddingProviderId === selectedLlmProvider.id;
	const selectedLlmProviderUsedByRagDraft =
		selectedLlmProvider !== null && ragSettings.embeddingProviderId === selectedLlmProvider.id;
	const selectedLlmProviderTriggersRagReindex =
		selectedLlmProvider !== null &&
		providerCanHandleRagEmbedding(selectedLlmProvider) &&
		(selectedLlmProviderUsedByPersistedRag || selectedLlmProviderUsedByRagDraft);
	const selectedAgent = acpAgents.find((agent) => agent.id === selectedAgentId) ?? null;
	const selectedMcpServer = mcpServers.find((server) => server.id === selectedMcpServerId) ?? null;
	const sectionSummaryText: Record<SettingsSectionId, string> = {
		general: "即时生效",
		prompts: promptsHaveUnsavedChanges ? "有草稿" : "已同步",
		llm: llmHasUnsavedChanges ? "有草稿" : `${llmSettings.providers.length} 条`,
		rag: ragHasUnsavedChanges ? "有草稿" : `目录 ${ragSourceDirectoryCount}`,
		acp: acpHasUnsavedChanges ? "有草稿" : `${acpAgents.length} 个 Agent`,
		mcp: mcpHasUnsavedChanges ? "有草稿" : `${mcpServers.length} 个服务`,
		skills: skillCatalog.exists ? `${skillCatalog.skills.length} 个 skill` : "只读",
		about: "只读",
	};
	const selectedPresetInstallOption =
		acpAgentOptions.find((option) => option.id === selectedPresetOptionId) ?? null;
	const selectedLlmIssueCount = selectedLlmProvider
		? (llmValidation.providerIssues[selectedLlmProvider.id]?.length ?? 0)
		: 0;
	const selectedAgentIssueCount = selectedAgent
		? (acpValidation.agentIssues[selectedAgent.id]?.length ?? 0)
		: 0;
	const selectedMcpIssueCount = selectedMcpServer
		? (mcpValidation.serverIssues[selectedMcpServer.id]?.length ?? 0)
		: 0;
	const builtinRagMcpServerHttpUrl =
		builtinRagMcpServerStatus?.server.transport === "http"
			? builtinRagMcpServerStatus.server.url.trim()
			: null;
	const builtinRagMcpServer =
		builtinRagMcpServerHttpUrl !== null
			? (mcpServers.find(
					(server) =>
						server.transport === "http" && server.url.trim() === builtinRagMcpServerHttpUrl,
				) ?? null)
			: null;
	const builtinRagMcpConfigured = builtinRagMcpServer !== null;
	const regularMcpServers =
		builtinRagMcpServerHttpUrl === null
			? mcpServers
			: mcpServers.filter(
					(server) =>
						!(server.transport === "http" && server.url.trim() === builtinRagMcpServerHttpUrl),
				);
	const builtinRagMcpIssueCount = builtinRagMcpServer
		? (mcpValidation.serverIssues[builtinRagMcpServer.id]?.length ?? 0)
		: 0;
	const builtinRagMcpTransportMeta =
		builtinRagMcpServerStatus !== null
			? getMcpTransportMeta(builtinRagMcpServerStatus.server.transport)
			: getMcpTransportMeta("http");
	const builtinRagMcpToggleDisabled =
		!builtinRagMcpConfigured &&
		(!builtinRagMcpServerStatus?.running || builtinRagMcpServerStatus.server.transport !== "http");
	const selectedSkill = skillCatalog.skills.find((skill) => skill.id === selectedSkillId) ?? null;
	const selectedLlmFieldIssues = selectedLlmProvider
		? (llmValidation.providerFieldIssues[selectedLlmProvider.id] ?? {})
		: {};
	const selectedAgentFieldIssues = selectedAgent
		? (acpValidation.agentFieldIssues[selectedAgent.id] ?? {})
		: {};
	const selectedMcpFieldIssues = selectedMcpServer
		? (mcpValidation.serverFieldIssues[selectedMcpServer.id] ?? {})
		: {};
	const isMcpCreateMode =
		mcpPanelMode === "create" || (!selectedMcpServer && mcpServers.length === 0);
	const isMcpEditMode = !isMcpCreateMode && selectedMcpServer !== null;

	function buildAcpFieldRefKey(
		scope: "agent" | "server",
		id: string,
		field: McpFieldKey | AcpFieldKey,
	) {
		return `${scope}:${id}:${field}`;
	}

	function bindAcpFieldRef(
		scope: "agent" | "server",
		id: string,
		field: McpFieldKey | AcpFieldKey,
	) {
		const refKey = buildAcpFieldRefKey(scope, id, field);
		return (node: HTMLInputElement | HTMLTextAreaElement | null) => {
			acpFieldRefs.current[refKey] = node;
		};
	}

	function focusAcpField(scope: "agent" | "server", id: string, field: McpFieldKey | AcpFieldKey) {
		const refKey = buildAcpFieldRefKey(scope, id, field);
		window.requestAnimationFrame(() => {
			acpFieldRefs.current[refKey]?.focus();
		});
	}

	function buildLlmFieldRefKey(id: string, field: LlmFieldKey) {
		return `${id}:${field}`;
	}

	function bindLlmFieldRef(id: string, field: LlmFieldKey) {
		const refKey = buildLlmFieldRefKey(id, field);
		return (node: HTMLInputElement | null) => {
			llmFieldRefs.current[refKey] = node;
		};
	}

	function focusLlmField(id: string, field: LlmFieldKey) {
		const refKey = buildLlmFieldRefKey(id, field);
		window.requestAnimationFrame(() => {
			llmFieldRefs.current[refKey]?.focus();
		});
	}

	function bindRagFieldRef(field: RagFieldKey) {
		return (node: HTMLSelectElement | HTMLTextAreaElement | null) => {
			ragFieldRefs.current[field] = node;
		};
	}

	function focusRagField(field: RagFieldKey) {
		window.requestAnimationFrame(() => {
			ragFieldRefs.current[field]?.focus();
		});
	}

	function locateFirstLlmIssue() {
		const nextIssue = findFirstLlmIssue(llmSettings);
		if (!nextIssue) {
			return;
		}

		setActiveSection("llm");
		setSelectedLlmProviderId(nextIssue.providerId);
		focusLlmField(nextIssue.providerId, nextIssue.fieldKey);
	}

	function locateFirstAcpIssue() {
		const nextIssue = findFirstAcpIssue(acpAgents);
		if (!nextIssue) {
			return;
		}

		setActiveSection("acp");
		setSelectedAgentId(nextIssue.agentId);
		focusAcpField("agent", nextIssue.agentId, nextIssue.fieldKey);
	}

	function locateFirstMcpIssue() {
		const nextIssue = findFirstMcpIssue(mcpServers);
		if (!nextIssue) {
			return;
		}

		setActiveSection("mcp");
		queueMcpFieldFocus(nextIssue.serverId, nextIssue.serverFieldKey);
	}

	function locateFirstRagIssue() {
		const ragFieldOrder: RagFieldKey[] = [
			"embeddingProviderId",
			"sourceDirectories",
			"ignoreGlobs",
		];
		const nextField = ragFieldOrder.find((fieldKey) => ragValidation.fieldIssues[fieldKey]);
		if (!nextField) {
			return;
		}

		setActiveSection("rag");
		setSettingsError(
			ragValidation.fieldIssues[nextField] ?? ragValidation.issues[0] ?? "RAG 配置不合法",
		);
		focusRagField(nextField);
	}

	async function handleSaveAgent() {
		if (acpValidation.totalIssues > 0) {
			locateFirstAcpIssue();
			return;
		}

		setSavingAgent(true);
		setSettingsError(null);
		setAcpNotice(null);
		try {
			const preservedDefaultAgentId =
				defaultAgentId && acpAgents.some((agent) => agent.id === defaultAgentId)
					? defaultAgentId
					: null;
			const normalizedAgents = acpAgents.map((agent) => ({
				id: agent.id,
				name: agent.name.trim(),
				program: deriveProgramFromCommand(agent.command),
				args: [],
				shellCommand: agent.command.trim() || null,
			}));
			const nextCatalog = await setAcpAgents(normalizedAgents, preservedDefaultAgentId);
			const nextDraftAgents = nextCatalog.agents.map((agent) => ({
				id: agent.id,
				name: agent.name,
				command: buildAgentCommand(agent),
			}));
			const nextDefaultAgentId = nextCatalog.defaultAgentId ?? nextCatalog.agents[0]?.id ?? null;
			setDefaultAgentId(nextDefaultAgentId);
			applyAgentDrafts(
				nextDraftAgents,
				nextDraftAgents.some((agent) => agent.id === selectedAgentId)
					? selectedAgentId
					: (nextDraftAgents[0]?.id ?? null),
			);
			setSavedAcpSnapshot(buildAcpDraftSnapshot(nextDraftAgents));
			setSavedAcpState(buildSavedAcpDraftState(nextDraftAgents, nextDefaultAgentId));
		} catch (error: unknown) {
			setSettingsError(getErrorMessage(error, "ACP 配置保存失败"));
		} finally {
			setSavingAgent(false);
		}
	}

	async function handleSaveMcp() {
		if (mcpValidation.totalIssues > 0) {
			locateFirstMcpIssue();
			return;
		}

		setSavingMcp(true);
		setSettingsError(null);
		setMcpNotice(null);
		try {
			const nextCatalog = await setAcpMcpServers(mcpServers.map(serializeMcpServerDraft));
			const nextDraftServers = nextCatalog.servers.map(createMcpServerDraftFromConfig);
			const nextSelectedServerId = nextDraftServers.some(
				(server) => server.id === selectedMcpServerId,
			)
				? selectedMcpServerId
				: (nextDraftServers[0]?.id ?? null);
			applyMcpServerDrafts(nextDraftServers, nextSelectedServerId);
			setMcpPanelMode(nextSelectedServerId ? "edit" : "create");
			setSavedMcpSnapshot(buildMcpDraftSnapshot(nextDraftServers));
			setSavedMcpState(buildSavedMcpDraftState(nextDraftServers));
		} catch (error: unknown) {
			setSettingsError(getErrorMessage(error, "MCP 配置保存失败"));
		} finally {
			setSavingMcp(false);
		}
	}

	function handleDiscardTranslationDraft() {
		setPromptsSettings((current) => ({
			...current,
			translationPrompt: persistedAppSettings.prompts.translationPrompt,
		}));
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				translationProviderId: persistedAppSettings.llm.translationProviderId,
			}),
		);
		setSettingsError(null);
	}

	function handleDiscardQuestionAnswerDraft() {
		setPromptsSettings((current) => ({
			...current,
			ragAnswerSystemPrompt: persistedAppSettings.prompts.ragAnswerSystemPrompt,
		}));
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				questionAnswerProviderId: persistedAppSettings.llm.questionAnswerProviderId,
			}),
		);
		setSettingsError(null);
	}

	function handleDiscardLlmDraft() {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: persistedAppSettings.llm.providers.map(cloneLlmProviderDraft),
			}),
		);
		setSelectedLlmProviderId(
			persistedAppSettings.llm.providers.some((provider) => provider.id === selectedLlmProviderId)
				? selectedLlmProviderId
				: (persistedAppSettings.llm.providers[0]?.id ?? null),
		);
		setLlmModelOptions({});
		setLlmModelErrors({});
		setLoadingLlmModelProviderId(null);
		setOpenLlmModelPickerId(null);
		setSettingsError(null);
	}

	function handleDiscardRagDraft() {
		setRagSettings({
			sourceDirectories: [...persistedAppSettings.rag.sourceDirectories],
			ignoreGlobs: [...persistedAppSettings.rag.ignoreGlobs],
			embeddingProviderId: persistedAppSettings.rag.embeddingProviderId,
		});
		setSettingsError(null);
	}

	function handleDiscardAcpDraft() {
		const restored = cloneSavedAcpDraftState(savedAcpState);
		setAcpNotice(null);
		setDefaultAgentId(restored.defaultAgentId);
		applyAgentDrafts(
			restored.agents,
			restored.agents.some((agent) => agent.id === selectedAgentId)
				? selectedAgentId
				: (restored.agents[0]?.id ?? null),
		);
		setSettingsError(null);
	}

	function handleDiscardMcpDraft() {
		const restored = cloneSavedMcpDraftState(savedMcpState);
		setMcpNotice(null);
		const nextSelectedServerId = restored.servers.some(
			(server) => server.id === selectedMcpServerId,
		)
			? selectedMcpServerId
			: (restored.servers[0]?.id ?? null);
		applyMcpServerDrafts(restored.servers, nextSelectedServerId);
		setMcpPanelMode(nextSelectedServerId ? "edit" : "create");
		setSettingsError(null);
	}

	async function saveAppSettings(nextGeneral: GeneralSettings, nextAppearance: AppearanceSettings) {
		await persistAppSettings(
			{
				general: nextGeneral,
				appearance: nextAppearance,
				prompts: persistedAppSettings.prompts,
				llm: persistedAppSettings.llm,
				ocr: persistedAppSettings.ocr,
				rag: persistedAppSettings.rag,
			},
			setSavingSettings,
			"设置保存失败",
			{
				adoptPromptKeys: [],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: [],
				adoptProviderDependencies: false,
				adoptOcr: false,
				adoptRag: false,
			},
		);
	}

	async function handleSaveTranslationConfig() {
		await persistAppSettings(
			{
				general: persistedAppSettings.general,
				appearance: persistedAppSettings.appearance,
				prompts: {
					translationPrompt: promptsSettings.translationPrompt,
					ragAnswerSystemPrompt: persistedAppSettings.prompts.ragAnswerSystemPrompt,
				},
				llm: {
					...persistedAppSettings.llm,
					translationProviderId: llmSettings.translationProviderId,
				},
				ocr: persistedAppSettings.ocr,
				rag: persistedAppSettings.rag,
			},
			setSavingTranslationConfig,
			"翻译配置保存失败",
			{
				adoptPromptKeys: ["translationPrompt"],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: ["translationProviderId"],
				adoptProviderDependencies: false,
				adoptOcr: false,
				adoptRag: false,
			},
		);
	}

	async function handleSaveQuestionAnswerConfig() {
		await persistAppSettings(
			{
				general: persistedAppSettings.general,
				appearance: persistedAppSettings.appearance,
				prompts: {
					translationPrompt: persistedAppSettings.prompts.translationPrompt,
					ragAnswerSystemPrompt: promptsSettings.ragAnswerSystemPrompt,
				},
				llm: {
					...persistedAppSettings.llm,
					questionAnswerProviderId: llmSettings.questionAnswerProviderId,
				},
				ocr: persistedAppSettings.ocr,
				rag: persistedAppSettings.rag,
			},
			setSavingQuestionAnswerConfig,
			"文档问答配置保存失败",
			{
				adoptPromptKeys: ["ragAnswerSystemPrompt"],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: ["questionAnswerProviderId"],
				adoptProviderDependencies: false,
				adoptOcr: false,
				adoptRag: false,
			},
		);
	}

	async function handleSaveLlm() {
		if (llmValidation.totalIssues > 0) {
			locateFirstLlmIssue();
			return;
		}

		await persistAppSettings(
			{
				general: persistedAppSettings.general,
				appearance: persistedAppSettings.appearance,
				prompts: persistedAppSettings.prompts,
				llm: {
					providers: llmSettings.providers,
					translationProviderId: persistedAppSettings.llm.translationProviderId,
					questionAnswerProviderId: persistedAppSettings.llm.questionAnswerProviderId,
				},
				ocr: persistedAppSettings.ocr,
				rag: persistedAppSettings.rag,
			},
			setSavingLlm,
			"LLM 配置保存失败",
			{
				adoptPromptKeys: [],
				adoptLlmProviders: true,
				adoptLlmRouteKeys: [],
				adoptProviderDependencies: true,
				adoptOcr: false,
				adoptRag: false,
			},
		);
	}

	async function handleSaveOcr() {
		await persistAppSettings(
			{
				general: persistedAppSettings.general,
				appearance: persistedAppSettings.appearance,
				prompts: persistedAppSettings.prompts,
				llm: persistedAppSettings.llm,
				ocr: ocrSettings,
				rag: persistedAppSettings.rag,
			},
			setSavingOcr,
			"OCR 配置保存失败",
			{
				adoptPromptKeys: [],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: [],
				adoptProviderDependencies: false,
				adoptOcr: true,
				adoptRag: false,
			},
		);
	}

	async function handleSaveRag() {
		if (ragValidation.totalIssues > 0) {
			locateFirstRagIssue();
			return;
		}

		await persistAppSettings(
			{
				general: persistedAppSettings.general,
				appearance: persistedAppSettings.appearance,
				prompts: persistedAppSettings.prompts,
				llm: persistedAppSettings.llm,
				ocr: persistedAppSettings.ocr,
				rag: ragSettings,
			},
			setSavingRag,
			"RAG 配置保存失败",
			{
				adoptPromptKeys: [],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: [],
				adoptProviderDependencies: false,
				adoptOcr: false,
				adoptRag: true,
			},
		);
	}

	async function persistAppSettings(
		nextSettings: AppSettings,
		setSaving: (value: boolean) => void,
		fallbackMessage: string,
		options: {
			adoptPromptKeys: Array<keyof PromptsSettings>;
			adoptLlmProviders: boolean;
			adoptLlmRouteKeys: Array<"translationProviderId" | "questionAnswerProviderId">;
			adoptProviderDependencies: boolean;
			adoptOcr: boolean;
			adoptRag: boolean;
		},
	) {
		setSaving(true);
		setSettingsError(null);
		try {
			const saved = await setAppSettings(nextSettings);
			setGeneralSettings(saved.general);
			setAppearanceSettings(saved.appearance);
			applyAppearanceSettings(saved.appearance);
			onAppearanceChange?.(saved.appearance);
			if (options.adoptPromptKeys.length > 0) {
				setPromptsSettings((current) => ({
					...current,
					...Object.fromEntries(options.adoptPromptKeys.map((key) => [key, saved.prompts[key]])),
				}));
			}
			if (
				options.adoptLlmProviders ||
				options.adoptLlmRouteKeys.length > 0 ||
				options.adoptProviderDependencies
			) {
				setLlmSettings((current) =>
					reconcileLlmSettings({
						providers: options.adoptLlmProviders ? saved.llm.providers : current.providers,
						translationProviderId:
							options.adoptProviderDependencies ||
							options.adoptLlmRouteKeys.includes("translationProviderId")
								? saved.llm.translationProviderId
								: current.translationProviderId,
						questionAnswerProviderId:
							options.adoptProviderDependencies ||
							options.adoptLlmRouteKeys.includes("questionAnswerProviderId")
								? saved.llm.questionAnswerProviderId
								: current.questionAnswerProviderId,
					}),
				);
			}
			if (options.adoptProviderDependencies) {
				setOcrSettings((current) =>
					reconcileOcrSettings(
						{
							...current,
							llmProviderId: saved.ocr.llmProviderId,
						},
						saved.llm.providers,
					),
				);
				setRagSettings((current) =>
					reconcileRagSettings(
						{
							...current,
							embeddingProviderId: saved.rag.embeddingProviderId,
						},
						saved.llm.providers,
					),
				);
			}
			if (options.adoptLlmProviders) {
				setSelectedLlmProviderId(
					saved.llm.providers.some((provider) => provider.id === selectedLlmProviderId)
						? selectedLlmProviderId
						: (saved.llm.providers[0]?.id ?? null),
				);
				setSavedLlmSnapshot(buildLlmDraftSnapshot(saved.llm));
				setSavedLlmState(buildSavedLlmDraftState(saved.llm.providers));
			}
			if (options.adoptOcr) {
				setOcrSettings(saved.ocr);
			}
			if (options.adoptRag) {
				setRagSettings(saved.rag);
				setSavedRagSnapshot(buildRagDraftSnapshot(saved.rag));
				setSavedRagState(buildSavedRagDraftState(saved.rag));
			}
			setPersistedAppSettings(saved);
		} catch (error: unknown) {
			setSettingsError(getErrorMessage(error, fallbackMessage));
		} finally {
			setSaving(false);
		}
	}

	function handleHeaderDragStart(event: ReactMouseEvent<HTMLDivElement>) {
		if (event.button !== 0) {
			return;
		}

		event.preventDefault();

		void getCurrentWindow()
			.startDragging()
			.catch((dragError: unknown) => {
				console.warn("failed to start settings window dragging", dragError);
			});
	}

	function handleAgentFieldChange(agentId: string, key: "name" | "command", value: string) {
		setAcpNotice(null);
		setAcpAgentsState((current) =>
			current.map((agent) => (agent.id === agentId ? { ...agent, [key]: value } : agent)),
		);
	}

	function handleAddLlmProvider() {
		const nextProvider = createLlmProviderDraft();
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: [...current.providers, nextProvider],
			}),
		);
		setSelectedLlmProviderId(nextProvider.id);
		setSettingsError(null);
	}

	function handleRemoveLlmProvider(providerId: string) {
		const nextSelectedProviderId =
			selectedLlmProviderId === providerId
				? (llmSettings.providers.find((provider) => provider.id !== providerId)?.id ?? null)
				: selectedLlmProviderId;
		setLlmSettings((current) => {
			return reconcileLlmSettings({
				...current,
				providers: current.providers.filter((provider) => provider.id !== providerId),
			});
		});
		setSelectedLlmProviderId(nextSelectedProviderId);
		clearLlmModelCatalog(providerId);
		setSettingsError(null);
	}

	function handleLlmProviderFieldChange<K extends keyof LlmProviderConfig>(
		providerId: string,
		key: K,
		value: LlmProviderConfig[K],
	) {
		if (key === "baseUrl" || key === "apiKey") {
			clearLlmModelCatalog(providerId);
		}

		setLlmSettings((current) => {
			const nextSettings = reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					const nextProvider = { ...provider, [key]: value };
					if (key === "baseUrl" || key === "apiKey") {
						nextProvider.modelIdentityHint = null;
					} else if (key === "model") {
						nextProvider.modelIdentityHint =
							findLlmModelOption(providerId, String(value))?.identityHint ?? null;
					}

					return nextProvider;
				}),
			});
			return nextSettings;
		});
		setSettingsError(null);
	}

	function handleLlmProviderKindChange(providerId: string, kind: LlmProviderKind) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) =>
					provider.id === providerId ? applyLlmProviderKind(provider, kind) : provider,
				),
			}),
		);
		setSettingsError(null);
	}

	function handleLlmRouteProviderChange(
		key: "translationProviderId" | "questionAnswerProviderId",
		providerId: string | null,
	) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				[key]: providerId,
			}),
		);
		setSettingsError(null);
	}

	async function handleAppendRagSourceDirectory() {
		const selectedPath = await chooseDirectory(ragDirectoryPickerDefaultPath);
		if (!selectedPath) {
			return;
		}

		setRagSettings((current) => ({
			...current,
			sourceDirectories: current.sourceDirectories.includes(selectedPath)
				? current.sourceDirectories
				: [...current.sourceDirectories, selectedPath],
		}));
		setSettingsError(null);
	}

	async function handleScanRag() {
		if (ragValidation.totalIssues > 0) {
			locateFirstRagIssue();
			return;
		}

		setScanningRag(true);
		setSettingsError(null);
		try {
			const result = await scanRagSources(ragSettings, llmSettings);
			setRagScanResult(result);
		} catch (error: unknown) {
			setSettingsError(getErrorMessage(error, "RAG 扫描失败"));
		} finally {
			setScanningRag(false);
		}
	}

	function handleAddMcpServer(transport: McpTransport) {
		setMcpNotice(null);
		const nextServer = createMcpServerDraft(transport);
		const transportMeta = getMcpTransportMeta(transport);
		setMcpNotice({
			tone: "info",
			text: `已新建 ${transportMeta.label} 服务，继续填写当前表单即可。`,
		});
		applyMcpServerDrafts([...mcpServers, nextServer], nextServer.id);
		setMcpPanelMode("edit");
		queueMcpFieldFocus(nextServer.id, "name");
	}

	function handleRemoveMcpServer(serverId: string) {
		setMcpNotice(null);
		const nextServers = mcpServers.filter((server) => server.id !== serverId);
		const nextSelectedServerId =
			selectedMcpServerId === serverId ? (nextServers[0]?.id ?? null) : selectedMcpServerId;
		applyMcpServerDrafts(nextServers, nextSelectedServerId);
		setMcpPanelMode(nextSelectedServerId ? "edit" : "create");
	}

	function handleMcpServerFieldChange(
		serverId: string,
		key: keyof Omit<AcpMcpServerDraft, "id">,
		value: string,
	) {
		setMcpNotice(null);
		setMcpServersState((current) =>
			current.map((server) => (server.id === serverId ? { ...server, [key]: value } : server)),
		);
	}

	function handleApplyMcpTransportSelection() {
		handleAddMcpServer(selectedMcpTransport);
	}

	function handleAddBuiltinRagMcpServer() {
		const status = builtinRagMcpServerStatus;
		if (!status || status.server.transport !== "http") {
			setMcpNotice({
				tone: "warn",
				text: "内置 RAG MCP server 信息不可用，暂时不能直接填充。",
			});
			return;
		}
		if (!status.running) {
			setMcpNotice({
				tone: "warn",
				text: status.lastError || "内置 RAG MCP server 还没运行成功，先修复运行状态。",
			});
			return;
		}

		const existingServer = mcpServers.find((server) => {
			if (server.transport !== "http" || status.server.transport !== "http") {
				return false;
			}

			return server.url.trim() === status.server.url.trim();
		});
		if (existingServer) {
			setMcpNotice({
				tone: "warn",
				text: "Wabity RAG Query 已存在，不需要重复添加。",
			});
			queueMcpFieldFocus(existingServer.id, "name");
			return;
		}

		const nextServer = createMcpServerDraftFromConfig(status.server);
		setMcpNotice({
			tone: "info",
			text: "已加入内置 RAG Query，继续检查后保存即可。",
		});
		applyMcpServerDrafts([...mcpServers, nextServer], nextServer.id);
		setMcpPanelMode("edit");
		queueMcpFieldFocus(nextServer.id, "name");
	}

	function handleRemoveBuiltinRagMcpServer() {
		if (!builtinRagMcpServer) {
			setMcpNotice({
				tone: "warn",
				text: "内置 RAG Query 还没加入草稿，不需要移除。",
			});
			return;
		}

		setMcpNotice({
			tone: "info",
			text: "已从 MCP 草稿移除内置 RAG Query。",
		});
		const nextServers = mcpServers.filter((server) => server.id !== builtinRagMcpServer.id);
		const nextSelectedServerId =
			selectedMcpServerId === builtinRagMcpServer.id
				? (nextServers[0]?.id ?? null)
				: selectedMcpServerId;
		applyMcpServerDrafts(nextServers, nextSelectedServerId);
		setMcpPanelMode(nextSelectedServerId ? "edit" : "create");
	}

	function handleToggleBuiltinRagMcpServer(enabled: boolean) {
		if (enabled) {
			handleAddBuiltinRagMcpServer();
			return;
		}

		handleRemoveBuiltinRagMcpServer();
	}

	function renderMcpCreateCard() {
		return (
			<div
				className="settings-editor-card settings-editor-card-subtle settings-mcp-create-card"
				id="mcp-create"
				ref={bindSectionBlockRef("mcp-create")}
			>
				<div className="settings-editor-card-header">
					<div className="settings-acp-detail-copy">
						<strong className="settings-agent-mcp-title">新建服务</strong>
						<span className="settings-agent-meta">只在需要另一种连接方式时新增</span>
					</div>
					{selectedMcpServer ? (
						<button
							className="settings-agent-secondary settings-button-compact"
							onClick={handleReturnToCurrentMcp}
							type="button"
						>
							返回当前服务
						</button>
					) : null}
				</div>
				<div className="settings-acp-preset-panel settings-mcp-create-panel">
					<div className="settings-mcp-create-copy">
						<span className="settings-acp-preset-kicker">空白</span>
						<strong className="settings-acp-preset-label">创建一个新服务</strong>
					</div>
					<label className="settings-label settings-label-stacked">
						<span className="settings-acp-preset-label">连接类型</span>
						<select
							className="settings-select"
							onChange={(event) => setSelectedMcpTransport(event.target.value as McpTransport)}
							value={selectedMcpTransport}
						>
							{mcpTransportOptions.map((option) => (
								<option key={option.transport} value={option.transport}>
									{option.label} · {option.description}
								</option>
							))}
						</select>
					</label>
					<div className="settings-mcp-transport-guide">
						{renderMcpTransportGuide(selectedMcpTransport)}
					</div>
					<button
						className="settings-button settings-button-compact settings-acp-preset-action"
						onClick={handleApplyMcpTransportSelection}
						type="button"
					>
						新建 {getMcpTransportMeta(selectedMcpTransport).label}
					</button>
				</div>
			</div>
		);
	}

	function handleAddPresetAgent(option: (typeof acpAgentOptions)[number]) {
		const existingAgent = acpAgents.find((agent) => agent.command.trim() === option.command);
		if (existingAgent) {
			setAcpNotice({
				tone: "warn",
				text: `${option.label} 已存在。相同启动命令不需要重复添加。`,
			});
			setSelectedAgentId(existingAgent.id);
			return;
		}

		const nextAgent = createAgentDraft(option.label, option.command);
		const nextAgents = [...acpAgents, nextAgent];
		setAcpNotice({
			tone: "info",
			text: `${option.label} 已加入 ACP Agent 草稿，还没写回 config.toml。`,
		});
		applyAgentDrafts(nextAgents, nextAgent.id);
	}

	function handleAddCustomAgent() {
		const nextAgent = createAgentDraft("ACP Agent", "");
		const nextAgents = [...acpAgents, nextAgent];
		setAcpNotice({
			tone: "info",
			text: "已创建新的自定义 ACP Agent 草稿。先补全名称和启动命令，再保存。",
		});
		applyAgentDrafts(nextAgents, nextAgent.id);
	}

	function handleApplyPresetSelection() {
		if (!selectedPresetOptionId) {
			return;
		}

		if (selectedPresetOptionId === "__custom__") {
			handleAddCustomAgent();
			setSelectedPresetOptionId("__custom__");
			return;
		}

		const option = acpAgentOptions.find((item) => item.id === selectedPresetOptionId);
		if (!option) {
			return;
		}

		handleAddPresetAgent(option);
		setSelectedPresetOptionId("__custom__");
	}

	function handleRemoveAgent(agentId: string) {
		setAcpNotice(null);
		const nextAgents = acpAgents.filter((agent) => agent.id !== agentId);
		applyAgentDrafts(
			nextAgents,
			selectedAgentId === agentId ? (nextAgents[0]?.id ?? null) : selectedAgentId,
		);
	}

	return (
		<main className="settings-shell" ref={shellRef}>
			<section
				className="settings-frame"
				style={{
					width: `${frameSize.width}px`,
					maxWidth: `${frameSize.width}px`,
					height: `${frameSize.height}px`,
					maxHeight: `${frameSize.height}px`,
				}}
			>
				<header className="settings-header">
					<button className="settings-back-button" onClick={onBack} type="button">
						← 返回
					</button>
					<div
						aria-hidden="true"
						className="settings-header-drag-zone"
						onMouseDown={handleHeaderDragStart}
					/>
					<h1 className="settings-title">设置</h1>
					<div
						aria-hidden="true"
						className="settings-header-drag-zone"
						onMouseDown={handleHeaderDragStart}
					/>
					<button
						className="settings-close-button"
						onClick={() => void hideLauncherWindow()}
						type="button"
					>
						关闭
					</button>
				</header>

				<div className="settings-content">
					<div className="settings-layout">
						<aside className="settings-sidebar">
							<div className="settings-sidebar-inner">
								<nav
									aria-label="设置分组"
									className="settings-nav settings-nav-rail"
									role="tablist"
								>
									{settingsSections.map((section) => (
										<button
											aria-controls={getSettingsPanelId(section.id)}
											aria-selected={activeSection === section.id}
											className={`settings-nav-button ${activeSection === section.id ? "settings-nav-button-active" : ""}`}
											id={getSettingsTabId(section.id)}
											key={section.id}
											onKeyDown={(event) => handleSectionTabKeyDown(event, section.id)}
											onClick={() => handleSelectSection(section.id)}
											ref={(element) => {
												sectionTabRefs.current[section.id] = element;
											}}
											role="tab"
											tabIndex={activeSection === section.id ? 0 : -1}
											type="button"
										>
											<span className="settings-nav-button-label">{section.label}</span>
											<span className="settings-nav-button-meta">
												{sectionSummaryText[section.id]}
											</span>
										</button>
									))}
								</nav>
							</div>
						</aside>

						<div className="settings-main" ref={contentRef}>
							{settingsError ? (
								<div className="settings-banner settings-banner-error">{settingsError}</div>
							) : null}
							<div className="settings-main-header">
								<div className="settings-main-header-copy">
									<span className="settings-section-kicker">当前分组</span>
									<h2 className="settings-main-title">
										{settingsSections.find((section) => section.id === activeSection)?.label}
									</h2>
									<span className="settings-help-text settings-help-text-tight">
										{sectionSummaryText[activeSection]}
									</span>
								</div>
							</div>
							{activeQuickLinks.length > 0 ? (
								<div className="settings-main-jump-bar">
									<SettingsQuickJumpList
										activeBlockId={activeSectionBlockId}
										compact
										links={activeQuickLinks}
										onSelect={scrollToSectionBlock}
									/>
								</div>
							) : null}

							{activeSection === "general" ? (
								<section
									aria-labelledby={getSettingsTabId("general")}
									className="settings-section"
									id={getSettingsPanelId("general")}
									role="tabpanel"
									tabIndex={0}
								>
									<h2 className="settings-section-title">通用</h2>
									<div className="settings-item">
										<label className="settings-label">
											<span>开机自启动</span>
											<input
												checked={generalSettings.autoStart}
												className="settings-toggle"
												onChange={(e) =>
													void saveAppSettings(
														{ ...generalSettings, autoStart: e.target.checked },
														appearanceSettings,
													)
												}
												disabled={savingSettings}
												type="checkbox"
											/>
										</label>
									</div>
									<div className="settings-item">
										<label className="settings-label">
											<span>在 Dock 中显示</span>
											<input
												checked={generalSettings.showInDock}
												className="settings-toggle"
												onChange={(e) =>
													void saveAppSettings(
														{ ...generalSettings, showInDock: e.target.checked },
														appearanceSettings,
													)
												}
												disabled={savingSettings}
												type="checkbox"
											/>
										</label>
									</div>
									<div className="settings-item">
										<label className="settings-label">
											<span>语言</span>
											<select
												className="settings-select"
												value={generalSettings.language}
												onChange={(e) =>
													void saveAppSettings(
														{ ...generalSettings, language: e.target.value },
														appearanceSettings,
													)
												}
												disabled={savingSettings}
											>
												<option value="zh-CN">简体中文</option>
												<option value="en-US">English</option>
											</select>
										</label>
									</div>

									<div
										className="settings-editor-card settings-editor-card-subtle"
										id="general-shortcuts"
										ref={bindSectionBlockRef("general-shortcuts")}
									>
										<div className="settings-editor-card-header">
											<div className="settings-acp-detail-copy">
												<span className="settings-section-kicker">Shortcuts</span>
												<h3 className="settings-subsection-title">快捷键</h3>
												<span className="settings-help-text settings-help-text-tight">
													快捷键配置并入通用页，但仍然通过独立命令保存，避免和其它设置字段互相覆盖。
												</span>
											</div>
										</div>
										<ShortcutRecorderField
											isRecording={isRecording("toggle_launcher")}
											isSaving={savingShortcutKey === "toggle_launcher"}
											label="打开启动器"
											onActivate={() => handleShortcutClick("toggle_launcher")}
											shortcutValue={shortcutSettings.toggle_launcher}
											statusId="shortcut-toggle-launcher-status"
											triggerId="shortcut-toggle-launcher-trigger"
										/>
										<ShortcutRecorderField
											isRecording={isRecording("ocr_capture")}
											isSaving={savingShortcutKey === "ocr_capture"}
											label="截图 OCR"
											onActivate={() => handleShortcutClick("ocr_capture")}
											shortcutValue={shortcutSettings.ocr_capture}
											statusId="shortcut-ocr-capture-status"
											triggerId="shortcut-ocr-capture-trigger"
										/>
										<ShortcutRecorderField
											isRecording={isRecording("ocr_translate")}
											isSaving={savingShortcutKey === "ocr_translate"}
											label="优先翻译选中文本，否则 OCR"
											onActivate={() => handleShortcutClick("ocr_translate")}
											shortcutValue={shortcutSettings.ocr_translate}
											statusId="shortcut-ocr-translate-status"
											triggerId="shortcut-ocr-translate-trigger"
										/>
									</div>

									<div
										className="settings-editor-card settings-editor-card-subtle"
										id="general-appearance"
										ref={bindSectionBlockRef("general-appearance")}
									>
										<div className="settings-editor-card-header">
											<div className="settings-acp-detail-copy">
												<span className="settings-section-kicker">Appearance</span>
												<h3 className="settings-subsection-title">外观</h3>
												<span className="settings-help-text settings-help-text-tight">
													外观配置并入通用页，继续使用即时保存，不额外维护草稿。
												</span>
											</div>
										</div>
										<div className="settings-item">
											<label className="settings-label">
												<span>主题</span>
												<select
													className="settings-select"
													value={appearanceSettings.theme}
													onChange={(e) =>
														void saveAppSettings(generalSettings, {
															...appearanceSettings,
															theme: e.target.value,
														})
													}
													disabled={savingSettings}
												>
													<option value="auto">跟随系统</option>
													<option value="light">浅色</option>
													<option value="dark">深色</option>
												</select>
											</label>
										</div>
										<div className="settings-item">
											<label className="settings-label">
												<span>字体大小</span>
												<select
													className="settings-select"
													value={appearanceSettings.fontSize}
													onChange={(e) =>
														void saveAppSettings(generalSettings, {
															...appearanceSettings,
															fontSize: e.target.value,
														})
													}
													disabled={savingSettings}
												>
													<option value="small">小</option>
													<option value="medium">中</option>
													<option value="large">大</option>
												</select>
											</label>
										</div>
									</div>

									<div
										className="settings-editor-card settings-editor-card-subtle"
										id="general-ocr"
										ref={bindSectionBlockRef("general-ocr")}
									>
										<div className="settings-editor-card-header">
											<div className="settings-acp-detail-copy">
												<span className="settings-section-kicker">OCR</span>
												<h3 className="settings-subsection-title">截图识别</h3>
												<span className="settings-help-text settings-help-text-tight">
													OCR 配置移到了通用页。截图流程仍只在 macOS 可用；远程 OCR 会复用普通 LLM
													模型，并要求显式启用多模态。
												</span>
											</div>
											<button
												className="settings-button settings-button-compact"
												disabled={savingOcr}
												onClick={() => void handleSaveOcr()}
												type="button"
											>
												{savingOcr ? "保存中..." : "保存 OCR 配置"}
											</button>
										</div>

										<div className="settings-item">
											<label className="settings-label">
												<span>识别 Provider</span>
												<select
													className="settings-select"
													value={ocrSettings.provider}
													onChange={(event) =>
														setOcrSettings((current) => ({
															...current,
															provider: event.target.value as OcrSettings["provider"],
															llmProviderId:
																event.target.value === "llm_ocr"
																	? (current.llmProviderId ?? eligibleOcrProviders[0]?.id ?? null)
																	: current.llmProviderId,
														}))
													}
													disabled={savingOcr}
												>
													<option value="system">系统 OCR</option>
													<option value="llm_ocr">大模型 OCR</option>
													<option value="disabled">禁用</option>
												</select>
											</label>
										</div>

										{showLlmOcrFields ? (
											<div className="settings-item settings-item-stacked">
												<label
													className="settings-label settings-label-stacked"
													htmlFor="general-ocr-llm-provider"
												>
													<span>LLM 条目</span>
												</label>
												<select
													className="settings-select"
													disabled={savingOcr}
													id="general-ocr-llm-provider"
													onChange={(event) =>
														setOcrSettings((current) => ({
															...current,
															llmProviderId: event.target.value || null,
														}))
													}
													value={ocrSettings.llmProviderId ?? ""}
												>
													<option value="">选择一个启用了多模态的条目</option>
													{eligibleOcrProviders.map((provider) => (
														<option key={provider.id} value={provider.id}>
															{provider.name || provider.model || provider.baseUrl}
															{` · ${summarizeLlmProviderProfile(provider)}`}
														</option>
													))}
												</select>
												<span className="settings-help-text">
													OCR 只接受普通 LLM 类型且显式启用多模态的条目。
												</span>
												{eligibleOcrProviders.length === 0 ? (
													<span className="settings-help-text settings-help-text-tight">
														当前没有可用的 OCR 条目。先在 LLM 页面配置普通 LLM 模型，并开启多模态。
													</span>
												) : null}
											</div>
										) : null}
									</div>
								</section>
							) : null}

							{activeSection === "prompts" ? (
								<section
									aria-labelledby={getSettingsTabId("prompts")}
									className="settings-section"
									id={getSettingsPanelId("prompts")}
									role="tabpanel"
									tabIndex={0}
								>
									<h2 className="settings-section-title">AI 功能</h2>
									<div className="settings-acp-form-layout">
										<div className="settings-acp-detail" id="prompts-editor">
											<div
												className="settings-editor-card"
												id="prompts-translation"
												ref={bindSectionBlockRef("prompts-translation")}
											>
												<div className="settings-editor-card-header">
													<div className="settings-acp-detail-copy">
														<span className="settings-section-kicker">Translation</span>
														<h3 className="settings-subsection-title">翻译配置</h3>
														<span className="settings-help-text settings-help-text-tight">
															slash command `/translate`、`/fy`、`/tr`
															会使用这里指定的模型和系统提示词。未显式指定目标语言时，默认规则会在简体中文和英文之间互译。
														</span>
													</div>
													<div className="settings-editor-card-actions">
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingAiTaskConfig}
															onClick={() =>
																setPromptsSettings((current) => ({
																	...current,
																	translationPrompt: defaultPromptsSettings.translationPrompt,
																}))
															}
															type="button"
														>
															恢复默认
														</button>
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingAiTaskConfig || !translationHasUnsavedChanges}
															onClick={handleDiscardTranslationDraft}
															type="button"
														>
															{DISCARD_DRAFT_BUTTON_LABEL}
														</button>
														<button
															className="settings-button"
															disabled={savingAiTaskConfig || !translationHasUnsavedChanges}
															onClick={() => void handleSaveTranslationConfig()}
															type="button"
														>
															{savingTranslationConfig ? "保存中..." : "保存翻译配置"}
														</button>
													</div>
												</div>
												<div className="settings-item settings-item-stacked">
													<label
														className="settings-label settings-label-stacked"
														htmlFor="translation-provider"
													>
														<span>翻译模型</span>
													</label>
													<select
														className="settings-select"
														disabled={savingAiTaskConfig}
														id="translation-provider"
														onChange={(event) =>
															handleLlmRouteProviderChange(
																"translationProviderId",
																event.target.value || null,
															)
														}
														value={llmSettings.translationProviderId ?? ""}
													>
														<option value="">请选择一个普通 LLM 条目</option>
														{eligibleAiTaskProviders.map((provider) => (
															<option key={provider.id} value={provider.id}>
																{provider.name || provider.model || provider.baseUrl}
																{` · ${summarizeLlmProviderProfile(provider)}`}
															</option>
														))}
													</select>
													<span className="settings-help-text settings-help-text-tight">
														当前用于翻译链路的条目：
														{selectedTranslationProvider
															? `${selectedTranslationProvider.name || selectedTranslationProvider.model || selectedTranslationProvider.baseUrl} · ${summarizeLlmProviderProfile(selectedTranslationProvider)}`
															: "未配置"}
													</span>
													{eligibleAiTaskProviders.length === 0 ? (
														<span className="settings-help-text settings-help-text-tight">
															当前没有可用的翻译模型。先在 LLM 页面配置一个普通 LLM
															模型，再回到这里选择翻译模型。
														</span>
													) : null}
												</div>
												<div className="settings-item settings-item-stacked">
													<label
														className="settings-label settings-label-stacked"
														htmlFor="translation-prompt"
													>
														<span>系统提示词</span>
													</label>
													<textarea
														className="settings-textarea"
														disabled={savingAiTaskConfig}
														id="translation-prompt"
														onChange={(event) =>
															setPromptsSettings((current) => ({
																...current,
																translationPrompt: event.target.value,
															}))
														}
														placeholder="留空并保存时会恢复内置默认提示词"
														rows={10}
														value={promptsSettings.translationPrompt}
													/>
													<span className="settings-help-text">
														默认提示词要求模型只输出译文，并保留格式、Markdown、代码块、占位符和链接。这里只改翻译阶段，不影响
														RAG 问答。
													</span>
												</div>
											</div>

											<div
												className="settings-editor-card"
												id="prompts-rag-answer"
												ref={bindSectionBlockRef("prompts-rag-answer")}
											>
												<div className="settings-editor-card-header">
													<div className="settings-acp-detail-copy">
														<span className="settings-section-kicker">RAG Answer</span>
														<h3 className="settings-subsection-title">文档问答配置</h3>
														<span className="settings-help-text settings-help-text-tight">
															这里只控制检索后的回答阶段使用哪个模型，以及回答阶段遵循的系统提示词；不影响向量化、召回数量和
															Embedding 条目选择。
														</span>
													</div>
													<div className="settings-editor-card-actions">
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingAiTaskConfig}
															onClick={() =>
																setPromptsSettings((current) => ({
																	...current,
																	ragAnswerSystemPrompt:
																		defaultPromptsSettings.ragAnswerSystemPrompt,
																}))
															}
															type="button"
														>
															恢复默认
														</button>
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingAiTaskConfig || !questionAnswerHasUnsavedChanges}
															onClick={handleDiscardQuestionAnswerDraft}
															type="button"
														>
															{DISCARD_DRAFT_BUTTON_LABEL}
														</button>
														<button
															className="settings-button"
															disabled={savingAiTaskConfig || !questionAnswerHasUnsavedChanges}
															onClick={() => void handleSaveQuestionAnswerConfig()}
															type="button"
														>
															{savingQuestionAnswerConfig ? "保存中..." : "保存文档问答配置"}
														</button>
													</div>
												</div>
												<div className="settings-item settings-item-stacked">
													<label
														className="settings-label settings-label-stacked"
														htmlFor="question-answer-provider"
													>
														<span>问答模型</span>
													</label>
													<select
														className="settings-select"
														disabled={savingAiTaskConfig}
														id="question-answer-provider"
														onChange={(event) =>
															handleLlmRouteProviderChange(
																"questionAnswerProviderId",
																event.target.value || null,
															)
														}
														value={llmSettings.questionAnswerProviderId ?? ""}
													>
														<option value="">请选择一个普通 LLM 条目</option>
														{eligibleAiTaskProviders.map((provider) => (
															<option key={provider.id} value={provider.id}>
																{provider.name || provider.model || provider.baseUrl}
																{` · ${summarizeLlmProviderProfile(provider)}`}
															</option>
														))}
													</select>
													<span className="settings-help-text settings-help-text-tight">
														当前用于回答阶段的条目：
														{selectedQuestionAnswerProvider
															? `${selectedQuestionAnswerProvider.name || selectedQuestionAnswerProvider.model || selectedQuestionAnswerProvider.baseUrl} · ${summarizeLlmProviderProfile(selectedQuestionAnswerProvider)}`
															: "未配置"}
													</span>
													<span className="settings-help-text settings-help-text-tight">
														文档检索仍然使用 RAG 页面配置的 Embedding 条目；这里只决定回答阶段。
													</span>
													{eligibleAiTaskProviders.length === 0 ? (
														<span className="settings-help-text settings-help-text-tight">
															当前没有可用的普通 LLM 条目。先在 LLM 页面配置至少一个非 Embedding
															模型。
														</span>
													) : null}
												</div>
												<div className="settings-item settings-item-stacked">
													<label
														className="settings-label settings-label-stacked"
														htmlFor="rag-answer-system-prompt"
													>
														<span>系统提示词</span>
													</label>
													<textarea
														className="settings-textarea"
														disabled={savingAiTaskConfig}
														id="rag-answer-system-prompt"
														onChange={(event) =>
															setPromptsSettings((current) => ({
																...current,
																ragAnswerSystemPrompt: event.target.value,
															}))
														}
														placeholder="留空并保存时会恢复内置默认提示词"
														rows={10}
														value={promptsSettings.ragAnswerSystemPrompt}
													/>
													<span className="settings-help-text">
														默认提示词会强制模型先用工具取证，再区分事实与推断；证据不足、冲突或缺失时，必须直接说明，而不是猜。
													</span>
												</div>
											</div>
										</div>
									</div>
								</section>
							) : null}

							{activeSection === "llm" ? (
								<section
									aria-labelledby={getSettingsTabId("llm")}
									className="settings-section"
									id={getSettingsPanelId("llm")}
									role="tabpanel"
									tabIndex={0}
								>
									<h2 className="settings-section-title">LLM</h2>
									<div className="settings-acp-form-layout settings-llm-layout">
										<aside
											className="settings-acp-sidebar settings-acp-sidebar-secondary settings-llm-sidebar"
											id="llm-catalog"
											ref={bindSectionBlockRef("llm-catalog")}
										>
											<div className="settings-acp-sidebar-header">
												<div className="settings-acp-sidebar-copy">
													<span className="settings-section-kicker">已配置</span>
													<h3 className="settings-subsection-title">LLM 条目</h3>
													<span className="settings-help-text settings-help-text-tight">
														每个条目只配置一个模型。右侧按“配置类型、连接信息、模型名”顺序编辑。
													</span>
												</div>
												<button
													className="settings-button settings-button-compact"
													onClick={handleAddLlmProvider}
													type="button"
												>
													新增
												</button>
											</div>

											{llmSettings.providers.length > 0 ? (
												<div className="settings-llm-provider-grid">
													{llmSettings.providers.map((provider) => {
														const issueCount =
															llmValidation.providerIssues[provider.id]?.length ?? 0;
														const isSelected = provider.id === selectedLlmProviderId;
														return (
															<article
																className={`settings-llm-provider-card ${isSelected ? "settings-llm-provider-card-selected" : ""} ${issueCount > 0 ? "settings-llm-provider-card-invalid" : ""}`}
																key={provider.id}
															>
																<button
																	aria-pressed={isSelected}
																	className="settings-llm-provider-card-main"
																	onClick={() => setSelectedLlmProviderId(provider.id)}
																	type="button"
																>
																	<div className="settings-llm-provider-card-header">
																		<span className="settings-section-kicker">模型条目</span>
																		<div className="settings-llm-provider-card-badges">
																			{llmSettings.translationProviderId === provider.id ? (
																				<span className="settings-status-chip settings-status-chip-strong">
																					翻译
																				</span>
																			) : null}
																			{llmSettings.questionAnswerProviderId === provider.id ? (
																				<span className="settings-status-chip settings-status-chip-strong">
																					问答
																				</span>
																			) : null}
																			{providerCanHandleOcr(provider) ? (
																				<span className="settings-status-chip">多模态</span>
																			) : null}
																			{issueCount > 0 ? (
																				<span className="settings-llm-provider-card-issue-badge">
																					{issueCount} 个问题
																				</span>
																			) : null}
																		</div>
																	</div>

																	<div className="settings-agent-title-wrap">
																		<strong className="settings-agent-name">
																			{provider.name.trim() || "未命名模型条目"}
																		</strong>
																		<span className="settings-agent-meta">
																			{provider.baseUrl.trim() || "未设置 Base URL"}
																		</span>
																	</div>

																	<div className="settings-llm-provider-card-model-grid">
																		<div
																			className={`settings-llm-provider-card-model settings-llm-provider-card-model-${provider.modelType}`}
																		>
																			<span className="settings-llm-provider-card-model-label">
																				{getLlmProviderKindLabel(getLlmProviderKind(provider))}
																			</span>
																			<span className="settings-agent-command-preview">
																				{provider.model.trim() || "未配置模型"}
																			</span>
																		</div>
																	</div>

																	<span className="settings-agent-meta">
																		{summarizeLlmProviderProfile(provider)}
																	</span>
																</button>
															</article>
														);
													})}
												</div>
											) : (
												<div className="settings-empty-panel settings-empty-panel-subtle">
													<strong className="settings-empty-title">还没有 LLM 条目</strong>
													<span className="settings-help-text settings-help-text-tight">
														先添加至少一个条目。OCR、翻译和 RAG 问答会引用 LLM 类型条目；RAG
														索引会引用 Embedding 类型条目。
													</span>
												</div>
											)}
										</aside>

										<div
											className="settings-acp-detail settings-llm-detail"
											id="llm-editor"
											ref={bindSectionBlockRef("llm-editor")}
										>
											<SettingsDraftActionCard
												actions={
													<>
														{llmValidation.totalIssues > 0 ? (
															<button
																className="settings-button settings-agent-secondary"
																onClick={locateFirstLlmIssue}
																type="button"
															>
																定位问题
															</button>
														) : null}
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingLlm || !llmHasUnsavedChanges}
															onClick={handleDiscardLlmDraft}
															type="button"
														>
															{DISCARD_DRAFT_BUTTON_LABEL}
														</button>
														<button
															className="settings-button"
															disabled={savingLlm || !llmHasUnsavedChanges}
															onClick={() => void handleSaveLlm()}
															type="button"
														>
															{savingLlm ? "保存中..." : "保存 LLM 配置"}
														</button>
													</>
												}
												description={
													llmHasUnsavedChanges
														? "当前 LLM 草稿尚未写回配置。"
														: "LLM 配置已同步到本地配置文件。"
												}
												title={llmHasUnsavedChanges ? "有未保存的 LLM 草稿" : "LLM 配置已同步"}
											/>
											{selectedLlmProvider ? (
												<>
													<div
														className={`settings-llm-editor-hero settings-llm-editor-hero-${selectedLlmProvider.modelType}`}
													>
														<div className="settings-llm-editor-hero-copy">
															<span className="settings-section-kicker">编辑</span>
															<h3 className="settings-subsection-title">
																{selectedLlmProvider.name.trim() || "LLM 条目"}
															</h3>
															<div className="settings-llm-editor-hero-meta">
																<span>
																	{selectedLlmProvider.baseUrl.trim() || "未设置 Base URL"}
																</span>
																<span>{selectedLlmProvider.model.trim() || "未配置模型"}</span>
															</div>
															<div className="settings-llm-editor-hero-badges">
																<span
																	className={`settings-llm-hero-type settings-llm-hero-type-${selectedLlmProvider.modelType}`}
																>
																	{getLlmProviderKindLabel(selectedLlmProviderKind ?? "embedding")}
																</span>
																{llmSettings.translationProviderId === selectedLlmProvider.id ? (
																	<span className="settings-status-chip settings-status-chip-strong">
																		翻译 LLM
																	</span>
																) : null}
																{llmSettings.questionAnswerProviderId === selectedLlmProvider.id ? (
																	<span className="settings-status-chip settings-status-chip-strong">
																		问答 LLM
																	</span>
																) : null}
																{providerCanHandleOcr(selectedLlmProvider) ? (
																	<span className="settings-status-chip">多模态</span>
																) : null}
															</div>
														</div>
														<button
															className="settings-button settings-agent-remove-inline"
															onClick={() => handleRemoveLlmProvider(selectedLlmProvider.id)}
															type="button"
														>
															删除
														</button>
													</div>

													<div className="settings-llm-editor-grid">
														<div className="settings-llm-editor-main">
															<div className="settings-llm-editor-panel">
																<div className="settings-item settings-item-stacked">
																	<label
																		className="settings-label settings-label-stacked"
																		htmlFor="llm-provider-kind"
																	>
																		<span>配置类型</span>
																	</label>
																	<select
																		className="settings-select"
																		disabled={savingLlm}
																		id="llm-provider-kind"
																		onChange={(event) =>
																			handleLlmProviderKindChange(
																				selectedLlmProvider.id,
																				event.target.value as LlmProviderKind,
																			)
																		}
																		value={selectedLlmProviderKind ?? "embedding"}
																	>
																		<option value="llm_responses_stateless">
																			LLM · responses stateless
																		</option>
																		<option value="llm_responses_stateful">
																			LLM · responses stateful
																		</option>
																		<option value="llm_chat_completions">
																			LLM · chat/completions
																		</option>
																		<option value="embedding">Embedding</option>
																	</select>
																</div>

																<div className="settings-item settings-item-stacked">
																	<label
																		className="settings-label settings-label-stacked"
																		htmlFor="llm-provider-name"
																	>
																		<span>名称</span>
																	</label>
																	<input
																		aria-describedby={joinDescribedByIds(
																			selectedLlmFieldIssues.name
																				? buildFieldIssueId("llm", "name", selectedLlmProvider.id)
																				: undefined,
																		)}
																		aria-invalid={selectedLlmFieldIssues.name ? true : undefined}
																		className="settings-input settings-input-wide"
																		disabled={savingLlm}
																		id="llm-provider-name"
																		onChange={(event) =>
																			handleLlmProviderFieldChange(
																				selectedLlmProvider.id,
																				"name",
																				event.target.value,
																			)
																		}
																		ref={bindLlmFieldRef(selectedLlmProvider.id, "name")}
																		type="text"
																		value={selectedLlmProvider.name}
																	/>
																	{selectedLlmFieldIssues.name ? (
																		<span
																			className="settings-field-error"
																			id={buildFieldIssueId("llm", "name", selectedLlmProvider.id)}
																		>
																			{selectedLlmFieldIssues.name}
																		</span>
																	) : null}
																</div>

																<div className="settings-item settings-item-stacked">
																	<label
																		className="settings-label settings-label-stacked"
																		htmlFor="llm-provider-base-url"
																	>
																		<span>API Base URL</span>
																	</label>
																	<input
																		aria-describedby={joinDescribedByIds(
																			selectedLlmFieldIssues.baseUrl
																				? buildFieldIssueId(
																						"llm",
																						"base-url",
																						selectedLlmProvider.id,
																					)
																				: undefined,
																		)}
																		aria-invalid={selectedLlmFieldIssues.baseUrl ? true : undefined}
																		className="settings-input settings-input-wide settings-input-mono"
																		disabled={savingLlm}
																		id="llm-provider-base-url"
																		onChange={(event) =>
																			handleLlmProviderFieldChange(
																				selectedLlmProvider.id,
																				"baseUrl",
																				event.target.value,
																			)
																		}
																		placeholder="https://api.openai.com/v1"
																		ref={bindLlmFieldRef(selectedLlmProvider.id, "baseUrl")}
																		type="text"
																		value={selectedLlmProvider.baseUrl}
																	/>
																	{selectedLlmFieldIssues.baseUrl ? (
																		<span
																			className="settings-field-error"
																			id={buildFieldIssueId(
																				"llm",
																				"base-url",
																				selectedLlmProvider.id,
																			)}
																		>
																			{selectedLlmFieldIssues.baseUrl}
																		</span>
																	) : null}
																</div>

																<div className="settings-item settings-item-stacked">
																	<label
																		className="settings-label settings-label-stacked"
																		htmlFor="llm-provider-api-key"
																	>
																		<span>API Key</span>
																	</label>
																	<input
																		autoComplete="off"
																		className="settings-input settings-input-wide settings-input-mono"
																		disabled={savingLlm}
																		id="llm-provider-api-key"
																		onChange={(event) =>
																			handleLlmProviderFieldChange(
																				selectedLlmProvider.id,
																				"apiKey",
																				event.target.value,
																			)
																		}
																		placeholder="sk-..."
																		type="password"
																		value={selectedLlmProvider.apiKey}
																	/>
																</div>

																<div
																	className="settings-item settings-item-stacked"
																	ref={llmModelMenuRef}
																>
																	<label
																		className="settings-label settings-label-stacked"
																		htmlFor="llm-provider-model"
																	>
																		<span>模型名</span>
																	</label>
																	<div className="settings-llm-model-picker">
																		<div className="settings-llm-model-input-row">
																			<input
																				aria-describedby={joinDescribedByIds(
																					selectedLlmFieldIssues.model
																						? buildFieldIssueId(
																								"llm",
																								"model",
																								selectedLlmProvider.id,
																							)
																						: undefined,
																					"llm-provider-model-help",
																				)}
																				aria-invalid={
																					selectedLlmFieldIssues.model ? true : undefined
																				}
																				aria-controls={
																					openLlmModelPickerId ===
																					buildLlmModelPickerId(selectedLlmProvider.id, "model")
																						? "llm-provider-model-menu"
																						: undefined
																				}
																				aria-expanded={
																					openLlmModelPickerId ===
																					buildLlmModelPickerId(selectedLlmProvider.id, "model")
																				}
																				aria-haspopup="listbox"
																				className="settings-input settings-input-wide settings-input-mono"
																				disabled={savingLlm}
																				id="llm-provider-model"
																				onChange={(event) =>
																					handleLlmProviderFieldChange(
																						selectedLlmProvider.id,
																						"model",
																						event.target.value,
																					)
																				}
																				onKeyDown={(event) =>
																					handleLlmModelInputKeyDown(
																						event,
																						selectedLlmProvider,
																						"model",
																					)
																				}
																				placeholder={getLlmProviderModelPlaceholder(
																					selectedLlmProvider,
																				)}
																				ref={bindLlmFieldRef(selectedLlmProvider.id, "model")}
																				type="text"
																				value={selectedLlmProvider.model}
																			/>
																			<button
																				aria-expanded={
																					openLlmModelPickerId ===
																					buildLlmModelPickerId(selectedLlmProvider.id, "model")
																				}
																				aria-haspopup="listbox"
																				aria-label={
																					selectedLlmProviderModels.length > 0
																						? "展开模型列表"
																						: "通过 /models 拉取模型列表"
																				}
																				className="settings-button settings-llm-model-toggle"
																				disabled={savingLlm || !selectedLlmProvider.baseUrl.trim()}
																				onClick={() =>
																					void handleToggleLlmModelMenu(
																						selectedLlmProvider,
																						"model",
																					)
																				}
																				type="button"
																			>
																				{isLoadingSelectedLlmProviderModels ? "..." : "▾"}
																			</button>
																		</div>
																		{openLlmModelPickerId ===
																		buildLlmModelPickerId(selectedLlmProvider.id, "model") ? (
																			<div
																				className="settings-combobox-panel settings-llm-model-panel"
																				id="llm-provider-model-menu"
																				role="listbox"
																			>
																				<div className="settings-llm-model-panel-header">
																					<span className="settings-combobox-option-meta">
																						已拉取 {selectedLlmProviderModels.length} 个模型
																					</span>
																					<button
																						className="settings-button settings-llm-model-refresh"
																						disabled={
																							savingLlm || isLoadingSelectedLlmProviderModels
																						}
																						onClick={() =>
																							void handleFetchLlmProviderModels(
																								selectedLlmProvider,
																								{
																									openField: "model",
																								},
																							)
																						}
																						type="button"
																					>
																						刷新
																					</button>
																				</div>
																				{selectedLlmProviderModels.map((model) => (
																					<button
																						aria-selected={selectedLlmProvider.model === model.id}
																						className={`settings-combobox-option ${
																							selectedLlmProvider.model === model.id
																								? "settings-combobox-option-active"
																								: ""
																						}`}
																						key={model.id}
																						onClick={() =>
																							handleLlmModelOptionClick(
																								selectedLlmProvider.id,
																								"model",
																								model,
																							)
																						}
																						role="option"
																						type="button"
																					>
																						<span className="settings-combobox-option-label">
																							{model.id}
																						</span>
																						<span className="settings-combobox-option-meta">
																							{selectedLlmProvider.model === model.id
																								? "当前已选"
																								: "点击填入输入框"}
																						</span>
																					</button>
																				))}
																			</div>
																		) : null}
																	</div>
																	<span
																		className="settings-help-text settings-help-text-tight"
																		id="llm-provider-model-help"
																	>
																		{isLoadingSelectedLlmProviderModels
																			? "正在从当前 Base URL 拉取模型列表。"
																			: selectedLlmProviderModelsError
																				? selectedLlmProviderModelsError
																				: selectedLlmProviderModels.length > 0
																					? `已拉取 ${selectedLlmProviderModels.length} 个模型。右侧下拉按钮可直接选，输入框仍可手填列表里没有的模型。`
																					: "先填写 Base URL 和 API Key，再点右侧下拉按钮请求 /models；如果服务不要求鉴权，API Key 可以留空。"}
																	</span>
																	{selectedLlmFieldIssues.model ? (
																		<span
																			className="settings-field-error"
																			id={buildFieldIssueId("llm", "model", selectedLlmProvider.id)}
																		>
																			{selectedLlmFieldIssues.model}
																		</span>
																	) : null}
																</div>
															</div>
														</div>
														<div className="settings-llm-editor-side">
															<div className="settings-llm-usage-card">
																<div className="settings-acp-detail-copy">
																	<span className="settings-section-kicker">用途</span>
																	<strong className="settings-agent-name">
																		{providerIsEmbeddingModel(selectedLlmProvider)
																			? "RAG Embedding"
																			: "通用 LLM"}
																	</strong>
																	<span className="settings-agent-meta">
																		{getLlmProviderUsageDescription(selectedLlmProvider)}
																	</span>
																</div>
																<div className="settings-llm-usage-badges">
																	{getLlmProviderUsageBadges(selectedLlmProvider).map((badge) => (
																		<span className="settings-status-chip" key={badge}>
																			{badge}
																		</span>
																	))}
																</div>
															</div>

															{providerIsLlmModel(selectedLlmProvider) ? (
																<div className="settings-llm-capability-panel">
																	<div className="settings-acp-detail-copy">
																		<span className="settings-section-kicker">能力</span>
																		<strong className="settings-agent-name">
																			{getLlmProviderKindLabel(
																				selectedLlmProviderKind ?? "embedding",
																			)}
																		</strong>
																		<span className="settings-agent-meta">
																			{selectedLlmProviderKind === "llm_responses_stateful"
																				? "当前是 responses 的 stateful 页面；续问时会优先复用上一轮 response_id。"
																				: selectedLlmProviderKind === "llm_responses_stateless"
																					? "当前是 responses 的 stateless 页面；续问时固定回退到显式历史。"
																					: "chat/completions 页面不支持 response_id 续链，可用于翻译和问答，但不会进入 OCR 列表。"}
																		</span>
																	</div>
																	{selectedLlmProviderKind === "llm_chat_completions" ? (
																		<div className="settings-acp-detail-copy">
																			<span className="settings-agent-meta">
																				这个页面没有额外能力开关。翻译和问答都会固定走
																				chat/completions；继续追问时回退到显式历史。
																			</span>
																		</div>
																	) : (
																		<div className="settings-llm-capability-grid">
																			<label className="settings-llm-capability-card">
																				<div className="settings-llm-capability-copy">
																					<strong>多模态</strong>
																					<span className="settings-agent-meta">
																						启用后条目才会进入 OCR 可选列表。
																					</span>
																				</div>
																				<input
																					checked={selectedLlmProvider.supportsMultimodal}
																					className="settings-toggle"
																					disabled={
																						savingLlm ||
																						!providerHasResponsesModel(selectedLlmProvider)
																					}
																					onChange={(event) =>
																						handleLlmProviderFieldChange(
																							selectedLlmProvider.id,
																							"supportsMultimodal",
																							event.target.checked,
																						)
																					}
																					type="checkbox"
																				/>
																			</label>
																		</div>
																	)}
																</div>
															) : (
																<div className="settings-llm-capability-panel settings-llm-capability-panel-passive">
																	<div className="settings-acp-detail-copy">
																		<span className="settings-section-kicker">能力</span>
																		<strong className="settings-agent-name">Embedding 约束</strong>
																		<span className="settings-agent-meta">
																			Embedding 条目不会出现在翻译 LLM、问答 LLM 或 OCR
																			列表，也不会启用多模态或 stateful 续链。
																		</span>
																	</div>
																</div>
															)}

															{selectedLlmProviderTriggersRagReindex ? (
																<div className="settings-validation-box settings-banner-warn">
																	{selectedLlmProviderUsedByPersistedRag
																		? "这个条目当前正被 RAG 用作 embedding。修改 Base URL 或模型并保存 LLM 配置后，现有文档向量会按新的建索引目标重新生成。"
																		: "当前 RAG 草稿引用了这个 embedding 条目。后续保存并应用该 RAG 配置时，如果这里的 Base URL 或模型发生变化，会按新的建索引目标重新生成文档向量。"}
																</div>
															) : null}
														</div>
													</div>

													{selectedLlmIssueCount > 0 ? (
														<div className="settings-validation-box settings-banner-error">
															<ul className="settings-issue-list">
																{(llmValidation.providerIssues[selectedLlmProvider.id] ?? []).map(
																	(issue) => (
																		<li key={issue}>{issue}</li>
																	),
																)}
															</ul>
														</div>
													) : null}
												</>
											) : (
												<div className="settings-empty-panel">
													<strong className="settings-empty-title">没有可编辑的 LLM 条目</strong>
													<span className="settings-help-text settings-help-text-tight">
														左侧新增一个条目后，先选配置类型，再补全名称、Base URL 和模型名。
													</span>
												</div>
											)}
										</div>
									</div>
								</section>
							) : null}

							{activeSection === "rag" ? (
								<section
									aria-labelledby={getSettingsTabId("rag")}
									className="settings-section"
									id={getSettingsPanelId("rag")}
									role="tabpanel"
									tabIndex={0}
								>
									<h2 className="settings-section-title">RAG</h2>
									<div className="settings-acp-form-layout settings-rag-layout">
										<aside
											className="settings-acp-sidebar settings-acp-sidebar-secondary settings-rag-sidebar"
											id="rag-summary"
											ref={bindSectionBlockRef("rag-summary")}
										>
											<div className="settings-acp-sidebar-header">
												<div className="settings-acp-sidebar-copy">
													<span className="settings-section-kicker">当前配置</span>
													<h3 className="settings-subsection-title">RAG Index</h3>
													<span className="settings-help-text settings-help-text-tight">
														RAG 只关心三个输入：Embedding 条目、扫描目录和忽略规则。
													</span>
												</div>
											</div>

											<div className="settings-rag-sidebar-stack">
												<div className="settings-rag-summary-card settings-rag-summary-card-primary">
													<div className="settings-rag-summary-header">
														<span className="settings-section-kicker">Embedding</span>
														<span
															className={`settings-status-chip ${ragHasUnsavedChanges ? "settings-status-chip-strong" : ""}`}
														>
															{ragHasUnsavedChanges ? "未保存" : "已同步"}
														</span>
													</div>
													<strong className="settings-agent-name">
														{selectedRagEmbeddingProviderLabel}
													</strong>
													<span className="settings-agent-meta">
														{selectedRagEmbeddingProvider
															? `${selectedRagEmbeddingProvider.model} · ${selectedRagEmbeddingProvider.baseUrl}`
															: eligibleRagEmbeddingProviders.length > 0
																? "右侧选择一个 Embedding 条目后，RAG 才能建立索引。"
																: "当前没有可用于 RAG 的 Embedding 条目，先去 LLM 页面新增一个。"}
													</span>
													<div className="settings-rag-summary-metric-grid">
														<div className="settings-rag-summary-metric">
															<span className="settings-rag-summary-value">
																{ragSourceDirectoryCount}
															</span>
															<span className="settings-agent-meta">扫描目录</span>
														</div>
														<div className="settings-rag-summary-metric">
															<span className="settings-rag-summary-value">
																{ragIgnoreGlobCount}
															</span>
															<span className="settings-agent-meta">忽略规则</span>
														</div>
														<div className="settings-rag-summary-metric">
															<span className="settings-rag-summary-value">
																{ragSupportedFileExtensions.length}
															</span>
															<span className="settings-agent-meta">支持后缀</span>
														</div>
														<div className="settings-rag-summary-metric">
															<span className="settings-rag-summary-value">
																{ragValidation.totalIssues}
															</span>
															<span className="settings-agent-meta">校验问题</span>
														</div>
													</div>
												</div>

												<div className="settings-rag-summary-card settings-rag-summary-card-muted">
													<div className="settings-acp-detail-copy">
														<span className="settings-section-kicker">支持范围</span>
														<strong className="settings-agent-name">文本后缀</strong>
														<span className="settings-agent-meta">
															只有这些后缀、可读、UTF-8、且不超过 50 MB
															的文本文件才会进入切分和向量化。
														</span>
													</div>
													<div className="settings-rag-chip-group">
														{ragSupportedFileExtensions.map((extension) => (
															<span className="settings-status-chip" key={extension}>
																.{extension}
															</span>
														))}
													</div>
												</div>
											</div>
										</aside>

										<div
											className="settings-acp-detail settings-rag-detail"
											id="rag-pipeline"
											ref={bindSectionBlockRef("rag-pipeline")}
										>
											<SettingsDraftActionCard
												actions={
													<>
														{ragValidation.totalIssues > 0 ? (
															<button
																className="settings-button settings-agent-secondary"
																onClick={locateFirstRagIssue}
																type="button"
															>
																定位问题
															</button>
														) : null}
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingRag || scanningRag || !ragHasUnsavedChanges}
															onClick={handleDiscardRagDraft}
															type="button"
														>
															{DISCARD_DRAFT_BUTTON_LABEL}
														</button>
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingRag || scanningRag}
															onClick={() => void handleScanRag()}
															type="button"
														>
															{scanningRag ? "扫描中..." : "立即重建索引"}
														</button>
														<button
															className="settings-button"
															disabled={savingRag || scanningRag || !ragHasUnsavedChanges}
															onClick={() => void handleSaveRag()}
															type="button"
														>
															{savingRag ? "保存中..." : "保存 RAG 配置"}
														</button>
													</>
												}
												description={
													ragHasUnsavedChanges
														? "当前 RAG 草稿尚未写回配置。"
														: "RAG 配置已同步到本地配置文件。"
												}
												title={ragHasUnsavedChanges ? "有未保存的 RAG 草稿" : "RAG 配置已同步"}
											/>
											<div className="settings-rag-hero">
												<div className="settings-rag-hero-copy">
													<span className="settings-section-kicker">Index Pipeline</span>
													<h3 className="settings-subsection-title">LanceDB 文档索引</h3>
													<span className="settings-help-text settings-help-text-tight">
														保存后会按当前配置启动目录监听。初次扫描和后续变更都会重新切分文本、调用
														embedding 模型，并用新向量覆盖旧索引。
													</span>
													<div className="settings-rag-hero-meta">
														<span>{`Embedding：${selectedRagEmbeddingProviderLabel}`}</span>
														<span>{`目录：${ragSourceDirectoryCount}`}</span>
														<span>{`忽略规则：${ragIgnoreGlobCount}`}</span>
													</div>
												</div>
												<div className="settings-rag-hero-badges">
													<span className="settings-status-chip">目录监听</span>
													<span className="settings-status-chip">结构切分</span>
													<span className="settings-status-chip">向量覆盖</span>
												</div>
											</div>

											<div className="settings-rag-editor-grid">
												<div className="settings-rag-editor-main">
													<div className="settings-rag-editor-panel">
														<div className="settings-editor-card-header">
															<div className="settings-acp-detail-copy">
																<span className="settings-section-kicker">模型</span>
																<strong className="settings-agent-name">Embedding 条目</strong>
																<span className="settings-agent-meta">
																	RAG 只接受 Embedding 类型条目。没有可选项时，先去 LLM 页面新增一个
																	Embedding 模型。
																</span>
															</div>
														</div>
														<div className="settings-item settings-item-stacked">
															<label
																className="settings-label settings-label-stacked"
																htmlFor="rag-embedding-provider"
															>
																<span>Embedding 条目</span>
															</label>
															<select
																aria-describedby={joinDescribedByIds(
																	ragValidation.fieldIssues.embeddingProviderId
																		? buildFieldIssueId("rag", "embedding-provider")
																		: undefined,
																	"rag-embedding-provider-help",
																)}
																aria-invalid={
																	ragValidation.fieldIssues.embeddingProviderId ? true : undefined
																}
																className="settings-select"
																disabled={savingRag || scanningRag}
																id="rag-embedding-provider"
																onChange={(event) =>
																	setRagSettings((current) => ({
																		...current,
																		embeddingProviderId: event.target.value || null,
																	}))
																}
																ref={bindRagFieldRef("embeddingProviderId")}
																value={ragSettings.embeddingProviderId ?? ""}
															>
																<option value="">选择一个配置了 embedding 模型的条目</option>
																{eligibleRagEmbeddingProviders.map((provider) => (
																	<option key={provider.id} value={provider.id}>
																		{provider.name || provider.model || provider.baseUrl}
																		{` · ${provider.model}`}
																	</option>
																))}
															</select>
															<span
																className="settings-help-text settings-help-text-tight"
																id="rag-embedding-provider-help"
															>
																当前选择会决定索引时调用哪个 embedding 模型；如果 Embedding 条目、
																扫描目录或忽略规则发生变化，保存 RAG 配置会触发自动重建。
															</span>
															{ragValidation.fieldIssues.embeddingProviderId ? (
																<span
																	className="settings-field-error"
																	id={buildFieldIssueId("rag", "embedding-provider")}
																>
																	{ragValidation.fieldIssues.embeddingProviderId}
																</span>
															) : null}
														</div>
													</div>

													<div className="settings-rag-editor-panel">
														<div className="settings-editor-card-header">
															<div className="settings-acp-detail-copy">
																<span className="settings-section-kicker">范围</span>
																<strong className="settings-agent-name">扫描目录与忽略规则</strong>
																<span className="settings-agent-meta">
																	Markdown 类文件按文档结构切分，其他文本文件走通用语义切分。
																</span>
															</div>
														</div>

														<div className="settings-item settings-item-stacked">
															<label
																className="settings-label settings-label-stacked"
																htmlFor="rag-source-dirs"
															>
																<span>扫描目录</span>
															</label>
															<textarea
																aria-describedby={joinDescribedByIds(
																	ragValidation.fieldIssues.sourceDirectories
																		? buildFieldIssueId("rag", "source-directories")
																		: undefined,
																	"rag-source-dirs-help",
																	"rag-source-dirs-extension-help",
																)}
																aria-invalid={
																	ragValidation.fieldIssues.sourceDirectories ? true : undefined
																}
																className="settings-textarea settings-input settings-input-mono"
																disabled={savingRag || scanningRag}
																id="rag-source-dirs"
																onChange={(event) =>
																	setRagSettings((current) => ({
																		...current,
																		sourceDirectories: parseTextLines(event.target.value),
																	}))
																}
																placeholder={ragSourceDirectoryPlaceholder}
																ref={bindRagFieldRef("sourceDirectories")}
																rows={6}
																value={formatTextLines(ragSettings.sourceDirectories)}
															/>
															<div className="settings-inline-actions">
																<button
																	className="settings-button settings-agent-secondary"
																	disabled={savingRag || scanningRag}
																	onClick={() => void handleAppendRagSourceDirectory()}
																	type="button"
																>
																	选择目录追加
																</button>
																<span
																	className="settings-help-text settings-help-text-tight"
																	id="rag-source-dirs-help"
																>
																	每行一个目录。保存后会监听这些目录内的文件变化；目录选择器默认从
																	`~/Documents` 打开。
																</span>
															</div>
															<span
																className="settings-help-text settings-help-text-tight"
																id="rag-source-dirs-extension-help"
															>
																当前允许向量化的后缀：{ragSupportedExtensionsLabel}。
															</span>
															{ragValidation.fieldIssues.sourceDirectories ? (
																<span
																	className="settings-field-error"
																	id={buildFieldIssueId("rag", "source-directories")}
																>
																	{ragValidation.fieldIssues.sourceDirectories}
																</span>
															) : null}
														</div>

														<div className="settings-item settings-item-stacked">
															<label
																className="settings-label settings-label-stacked"
																htmlFor="rag-ignore-globs"
															>
																<span>忽略通配符</span>
															</label>
															<textarea
																aria-describedby={joinDescribedByIds(
																	ragValidation.fieldIssues.ignoreGlobs
																		? buildFieldIssueId("rag", "ignore-globs")
																		: undefined,
																	"rag-ignore-globs-help",
																)}
																aria-invalid={
																	ragValidation.fieldIssues.ignoreGlobs ? true : undefined
																}
																className="settings-textarea settings-input settings-input-mono"
																disabled={savingRag || scanningRag}
																id="rag-ignore-globs"
																onChange={(event) =>
																	setRagSettings((current) => ({
																		...current,
																		ignoreGlobs: parseTextLines(event.target.value),
																	}))
																}
																placeholder={defaultRagIgnoreGlobPlaceholder}
																ref={bindRagFieldRef("ignoreGlobs")}
																rows={5}
																value={formatTextLines(ragSettings.ignoreGlobs)}
															/>
															<span
																className="settings-help-text settings-help-text-tight"
																id="rag-ignore-globs-help"
															>
																每行一个 glob。命中后文件不会被切分、向量化或写入 LanceDB。
															</span>
															{ragValidation.fieldIssues.ignoreGlobs ? (
																<span
																	className="settings-field-error"
																	id={buildFieldIssueId("rag", "ignore-globs")}
																>
																	{ragValidation.fieldIssues.ignoreGlobs}
																</span>
															) : null}
														</div>
													</div>
												</div>

												<div className="settings-rag-editor-side">
													<div className="settings-rag-usage-card">
														<div className="settings-acp-detail-copy">
															<span className="settings-section-kicker">流程</span>
															<strong className="settings-agent-name">索引阶段</strong>
															<span className="settings-agent-meta">
																先过滤文件，再切分文本，最后调用 embedding 模型写入 LanceDB。
															</span>
														</div>
														<div className="settings-rag-chip-group">
															<span className="settings-status-chip">过滤后缀</span>
															<span className="settings-status-chip">切分文本</span>
															<span className="settings-status-chip">生成向量</span>
															<span className="settings-status-chip">持久化索引</span>
														</div>
													</div>

													<div className="settings-rag-capability-panel">
														<div className="settings-acp-detail-copy">
															<span className="settings-section-kicker">边界</span>
															<strong className="settings-agent-name">数据与重建规则</strong>
														</div>
														<div className="settings-rag-capability-grid">
															<div className="settings-rag-capability-card">
																<strong>隐私边界</strong>
																<span className="settings-agent-meta">
																	RAG 建索引时会把切分后的文档内容发送给当前 embedding
																	模型。涉及隐私或敏感数据时，优先选本机部署的
																	Ollama，或其它你明确信任的模型服务。
																</span>
															</div>
															<div className="settings-rag-capability-card">
																<strong>自动重建</strong>
																<span className="settings-agent-meta">
																	RAG 元数据会记录每个文件最近一次建索引所用的 embedding
																	模型。当前实际使用的 Embedding
																	条目、扫描目录或忽略规则变化时，保存配置会自动重建；其它场景需要手动点击“立即重建索引”。
																</span>
															</div>
														</div>
													</div>

													<div
														className="settings-rag-scan-card"
														id="rag-scan-result"
														ref={bindSectionBlockRef("rag-scan-result")}
													>
														<div className="settings-acp-detail-copy">
															<span className="settings-section-kicker">扫描结果</span>
															<strong className="settings-agent-name">最近一次手动重建</strong>
															<span className="settings-agent-meta">
																{ragScanResult
																	? "这里只展示当前窗口内最近一次手动触发的扫描结果。"
																	: "还没有手动重建结果。完成一次“立即重建索引”后，这里会显示统计数据。"}
															</span>
														</div>
														{ragScanResult ? (
															<div className="settings-rag-scan-grid">
																{ragScanSummaryItems.map((item) => (
																	<div className="settings-rag-scan-metric" key={item.label}>
																		<span className="settings-rag-scan-label">{item.label}</span>
																		<span
																			className={`settings-rag-scan-value ${item.label === "数据库" ? "settings-rag-scan-value-path" : ""}`}
																		>
																			{item.value}
																		</span>
																	</div>
																))}
															</div>
														) : (
															<div className="settings-empty-panel settings-empty-panel-subtle">
																<strong className="settings-empty-title">没有扫描统计</strong>
																<span className="settings-help-text settings-help-text-tight">
																	保存配置不会自动跑一次全量扫描。需要时手动点击“立即重建索引”。
																</span>
															</div>
														)}
													</div>
												</div>
											</div>

											{ragValidation.totalIssues > 0 ? (
												<div className="settings-validation-box settings-banner-error">
													<ul className="settings-issue-list">
														{ragValidation.issues.map((issue) => (
															<li key={issue}>{issue}</li>
														))}
													</ul>
												</div>
											) : null}
										</div>
									</div>
								</section>
							) : null}

							{activeSection === "acp" ? (
								<section
									aria-labelledby={getSettingsTabId("acp")}
									className="settings-section"
									id={getSettingsPanelId("acp")}
									role="tabpanel"
									tabIndex={0}
								>
									<h2 className="settings-section-title">ACP Agent</h2>
									{acpNotice ? (
										<div className={`settings-banner settings-banner-${acpNotice.tone}`}>
											{acpNotice.text}
										</div>
									) : null}

									<div className="settings-acp-form-layout">
										<aside
											className="settings-acp-sidebar settings-acp-sidebar-secondary"
											id="acp-catalog"
											ref={bindSectionBlockRef("acp-catalog")}
										>
											<div className="settings-acp-sidebar-header">
												<div className="settings-acp-sidebar-copy">
													<span className="settings-section-kicker">已配置</span>
												</div>
												<button
													className="settings-button settings-button-compact"
													onClick={handleAddCustomAgent}
													type="button"
												>
													+ 新建
												</button>
											</div>
											<p className="settings-help-text settings-help-text-tight">
												ACP Agent 就是一条启动命令配置。预设只负责填表；实际创建 Session 用哪个
												Agent，看 launcher 顶部选择。
											</p>

											{acpAgents.length === 0 ? (
												<div className="settings-empty-panel">
													<strong className="settings-empty-title">还没有 Agent</strong>
													<span className="settings-help-text settings-help-text-tight">
														先新建一个，或者从预设填表。
													</span>
												</div>
											) : (
												<div className="settings-agent-list settings-agent-list-master settings-agent-list-compact">
													{acpAgents.map((agent) => {
														const issueCount = acpValidation.agentIssues[agent.id]?.length ?? 0;
														const isSelected = selectedAgentId === agent.id;
														return (
															<article
																className={`settings-agent-list-item ${isSelected ? "settings-agent-list-item-selected" : ""} ${
																	issueCount > 0 ? "settings-agent-list-item-invalid" : ""
																}`}
																key={agent.id}
															>
																<button
																	className="settings-agent-list-item-main"
																	onClick={() => setSelectedAgentId(agent.id)}
																	type="button"
																>
																	<div className="settings-agent-title-row">
																		<strong className="settings-agent-name">
																			{agent.name.trim() || "未命名 Agent"}
																		</strong>
																		<span className="settings-agent-meta">{issueCount} 个问题</span>
																	</div>
																	<span className="settings-agent-command-preview">
																		{agent.command.trim() || "还没有启动命令"}
																	</span>
																</button>
															</article>
														);
													})}
												</div>
											)}
										</aside>

										<div className="settings-acp-detail">
											<SettingsDraftActionCard
												actions={
													<>
														{acpValidation.totalIssues > 0 ? (
															<button
																className="settings-button settings-agent-secondary"
																onClick={locateFirstAcpIssue}
																type="button"
															>
																定位问题
															</button>
														) : null}
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingAgent || !acpHasUnsavedChanges}
															onClick={handleDiscardAcpDraft}
															type="button"
														>
															{DISCARD_DRAFT_BUTTON_LABEL}
														</button>
														<button
															className="settings-button"
															disabled={savingAgent || !acpHasUnsavedChanges}
															onClick={() => void handleSaveAgent()}
															type="button"
														>
															{savingAgent ? "保存中..." : "保存 ACP Agent"}
														</button>
													</>
												}
												description={
													acpValidation.totalIssues > 0
														? "先修复校验问题，再写回配置。"
														: acpHasUnsavedChanges
															? "当前 ACP Agent 草稿尚未写回配置。"
															: "ACP Agent 配置已与本地 config.toml 同步。"
												}
												title={
													acpValidation.totalIssues > 0
														? `先修复 ${acpValidation.totalIssues} 个问题`
														: acpHasUnsavedChanges
															? "有未保存的 ACP Agent 草稿"
															: "ACP Agent 配置已同步"
												}
											/>
											{selectedAgent ? (
												<>
													<div className="settings-acp-detail-header">
														<div className="settings-acp-detail-copy">
															<span className="settings-section-kicker">当前表单</span>
															<h3 className="settings-subsection-title">
																{selectedAgent.name.trim() || "未命名 Agent"}
															</h3>
														</div>
														<button
															className="settings-agent-remove settings-agent-remove-inline"
															onClick={() => handleRemoveAgent(selectedAgent.id)}
															type="button"
														>
															删除 Agent
														</button>
													</div>

													{(acpValidation.agentIssues[selectedAgent.id]?.length ?? 0) > 0 ? (
														<ul className="settings-issue-list">
															{(acpValidation.agentIssues[selectedAgent.id] ?? []).map((issue) => (
																<li key={issue}>{issue}</li>
															))}
														</ul>
													) : null}

													<div
														className="settings-editor-card"
														id="acp-presets"
														ref={bindSectionBlockRef("acp-presets")}
													>
														<div className="settings-editor-card-header">
															<strong className="settings-agent-mcp-title">选择 Agent</strong>
															<span className="settings-agent-meta">
																下拉项只负责填充表单默认值
															</span>
														</div>
														<div className="settings-acp-preset-layout">
															<div className="settings-acp-preset-controls">
																<div className="settings-acp-preset-panel">
																	<span className="settings-acp-preset-kicker">快速填充</span>
																	<label className="settings-label settings-label-stacked">
																		<span className="settings-acp-preset-label">Agent 模板</span>
																		<select
																			aria-label="选择 Agent"
																			className="settings-select"
																			onChange={(event) =>
																				setSelectedPresetOptionId(event.target.value)
																			}
																			value={selectedPresetOptionId}
																		>
																			<option value="__custom__">自定义 · 空白表单</option>
																			{acpAgentOptions.map((option) => {
																				const alreadyAdded = acpAgents.some(
																					(agent) => agent.command.trim() === option.command,
																				);
																				return (
																					<option key={option.id} value={option.id}>
																						{formatAcpAgentOptionLabel(option, alreadyAdded)}
																					</option>
																				);
																			})}
																		</select>
																	</label>
																	<span className="settings-help-text settings-help-text-tight">
																		只会把名称和启动命令填到下面表单，不会直接保存。
																	</span>
																	<button
																		className="settings-button settings-button-compact settings-acp-preset-action"
																		disabled={!selectedPresetOptionId}
																		onClick={handleApplyPresetSelection}
																		type="button"
																	>
																		填入表单
																	</button>
																</div>
															</div>
															<div className="settings-install-guide settings-install-guide-card">
																{renderPresetInstallGuide(
																	selectedPresetInstallOption,
																	selectedPresetOptionId,
																)}
															</div>
														</div>
													</div>

													<div
														className="settings-editor-card"
														id="acp-form"
														ref={bindSectionBlockRef("acp-form")}
													>
														<div className="settings-editor-card-header">
															<strong className="settings-agent-mcp-title">基础信息</strong>
															<span className="settings-agent-meta">
																{selectedAgentIssueCount > 0
																	? `${selectedAgentIssueCount} 个待处理问题`
																	: "基础信息完整"}
															</span>
														</div>
														<div className="settings-agent-fields">
															<label className="settings-label settings-label-stacked">
																<span>显示名称</span>
																<input
																	aria-describedby={joinDescribedByIds(
																		selectedAgentFieldIssues.name
																			? buildFieldIssueId("acp", "name", selectedAgent.id)
																			: undefined,
																	)}
																	aria-invalid={selectedAgentFieldIssues.name ? true : undefined}
																	className="settings-input settings-input-wide"
																	onChange={(event) =>
																		handleAgentFieldChange(
																			selectedAgent.id,
																			"name",
																			event.target.value,
																		)
																	}
																	placeholder="例如 Codex"
																	ref={bindAcpFieldRef("agent", selectedAgent.id, "name")}
																	type="text"
																	value={selectedAgent.name}
																/>
																{selectedAgentFieldIssues.name ? (
																	<span
																		className="settings-field-error"
																		id={buildFieldIssueId("acp", "name", selectedAgent.id)}
																	>
																		{selectedAgentFieldIssues.name}
																	</span>
																) : null}
															</label>
															<label className="settings-label settings-label-stacked">
																<span>启动命令</span>
																<input
																	aria-describedby={joinDescribedByIds(
																		selectedAgentFieldIssues.command
																			? buildFieldIssueId("acp", "command", selectedAgent.id)
																			: undefined,
																		`settings-acp-${selectedAgent.id}-command-help`,
																	)}
																	aria-invalid={selectedAgentFieldIssues.command ? true : undefined}
																	className="settings-input settings-input-wide settings-input-mono"
																	onChange={(event) =>
																		handleAgentFieldChange(
																			selectedAgent.id,
																			"command",
																			event.target.value,
																		)
																	}
																	placeholder="输入单行 shell 命令，例如 codex-acp"
																	ref={bindAcpFieldRef("agent", selectedAgent.id, "command")}
																	type="text"
																	value={selectedAgent.command}
																/>
																<span
																	className="settings-help-text settings-help-text-tight"
																	id={`settings-acp-${selectedAgent.id}-command-help`}
																>
																	这里只填写 Agent 启动命令；所有 Agent 仍共用同一份全局 MCP 配置。
																</span>
																{selectedAgentFieldIssues.command ? (
																	<span
																		className="settings-field-error"
																		id={buildFieldIssueId("acp", "command", selectedAgent.id)}
																	>
																		{selectedAgentFieldIssues.command}
																	</span>
																) : null}
															</label>
														</div>
													</div>
												</>
											) : (
												<>
													<div
														className="settings-editor-card"
														id="acp-presets"
														ref={bindSectionBlockRef("acp-presets")}
													>
														<div className="settings-editor-card-header">
															<strong className="settings-agent-mcp-title">选择 Agent</strong>
															<span className="settings-agent-meta">
																下拉项只负责填充表单默认值
															</span>
														</div>
														<div className="settings-acp-preset-layout">
															<div className="settings-acp-preset-controls">
																<div className="settings-acp-preset-panel">
																	<span className="settings-acp-preset-kicker">快速填充</span>
																	<label className="settings-label settings-label-stacked">
																		<span className="settings-acp-preset-label">Agent 模板</span>
																		<select
																			aria-label="选择 Agent"
																			className="settings-select"
																			onChange={(event) =>
																				setSelectedPresetOptionId(event.target.value)
																			}
																			value={selectedPresetOptionId}
																		>
																			<option value="__custom__">自定义 · 空白表单</option>
																			{acpAgentOptions.map((option) => (
																				<option key={option.id} value={option.id}>
																					{formatAcpAgentOptionLabel(option, false)}
																				</option>
																			))}
																		</select>
																	</label>
																	<span className="settings-help-text settings-help-text-tight">
																		只会把名称和启动命令填到下面表单，不会直接保存。
																	</span>
																	<button
																		className="settings-button settings-button-compact settings-acp-preset-action"
																		disabled={!selectedPresetOptionId}
																		onClick={handleApplyPresetSelection}
																		type="button"
																	>
																		填入表单
																	</button>
																</div>
															</div>
															<div className="settings-install-guide settings-install-guide-card">
																{renderPresetInstallGuide(
																	selectedPresetInstallOption,
																	selectedPresetOptionId,
																)}
															</div>
														</div>
													</div>
													<div
														className="settings-empty-panel"
														id="acp-form"
														ref={bindSectionBlockRef("acp-form")}
													>
														<strong className="settings-empty-title">
															先创建一个 ACP Agent 草稿
														</strong>
														<span className="settings-help-text settings-help-text-tight">
															先选择一个 Agent 填入默认值，或者直接新建一个空白 Agent 表单。
														</span>
														<div className="settings-acp-empty-actions">
															<button
																className="settings-button settings-button-compact"
																onClick={handleAddCustomAgent}
																type="button"
															>
																+ 新建 Agent
															</button>
														</div>
													</div>
												</>
											)}
										</div>
									</div>
								</section>
							) : null}

							{activeSection === "mcp" ? (
								<section
									aria-labelledby={getSettingsTabId("mcp")}
									className="settings-section"
									id={getSettingsPanelId("mcp")}
									role="tabpanel"
									tabIndex={0}
								>
									<h2 className="settings-section-title">MCP</h2>
									{mcpNotice ? (
										<div className={`settings-banner settings-banner-${mcpNotice.tone}`}>
											{mcpNotice.text}
										</div>
									) : null}

									<div className="settings-mcp-layout">
										<div
											className="settings-editor-card settings-editor-card-subtle settings-mcp-catalog-panel"
											id="mcp-catalog"
											ref={bindSectionBlockRef("mcp-catalog")}
										>
											<div className="settings-editor-card-header">
												<div className="settings-acp-sidebar-copy">
													<span className="settings-section-kicker">已配置</span>
													<h3 className="settings-subsection-title">服务目录</h3>
												</div>
											</div>
											<p className="settings-help-text settings-help-text-tight">
												所有 Agent 共用这一份 MCP 目录。先在上面选服务，下面只处理当前任务。
											</p>
											{mcpServers.length === 0 ? (
												<div className="settings-empty-panel settings-empty-panel-subtle">
													<strong className="settings-empty-title">还没有服务</strong>
													<span className="settings-help-text settings-help-text-tight">
														先新建一个服务，再补全连接参数。
													</span>
												</div>
											) : null}
											<div className="settings-catalog-grid settings-catalog-grid-3 settings-mcp-catalog-grid">
												<article
													className={`settings-mcp-list-item settings-mcp-builtin-card ${
														builtinRagMcpConfigured &&
														selectedMcpServerId === builtinRagMcpServer?.id
															? "settings-mcp-list-item-selected"
															: ""
													} ${builtinRagMcpIssueCount > 0 ? "settings-mcp-list-item-invalid" : ""}`}
												>
													<div className="settings-mcp-builtin-card-header">
														<div className="settings-agent-title-row">
															<strong className="settings-agent-name">内置 MCP</strong>
														</div>
														<label className="settings-mcp-builtin-toggle">
															<span className="sr-only">启用内置 MCP</span>
															<input
																checked={builtinRagMcpConfigured}
																className="settings-toggle"
																disabled={builtinRagMcpToggleDisabled}
																onChange={(event) =>
																	handleToggleBuiltinRagMcpServer(event.target.checked)
																}
																type="checkbox"
															/>
														</label>
													</div>
													<div className="settings-mcp-list-item-badges">
														<span className="settings-status-chip">内置</span>
														<span className="settings-mcp-badge">
															{builtinRagMcpTransportMeta.label}
														</span>
														<span
															className={`settings-status-chip ${
																builtinRagMcpServerStatus?.running
																	? "settings-status-chip-success"
																	: "settings-status-chip-warn"
															}`}
														>
															{builtinRagMcpServerStatus?.running ? "运行中" : "未运行"}
														</span>
														{builtinRagMcpIssueCount > 0 ? (
															<span className="settings-agent-meta">
																{builtinRagMcpIssueCount} 个问题
															</span>
														) : null}
													</div>
													<span className="settings-agent-command-preview">
														{builtinRagMcpServerStatus?.server.transport === "http"
															? builtinRagMcpServerStatus.server.url
															: "http://127.0.0.1:43189/internal/mcp/rag"}
													</span>
													<span className="settings-help-text settings-help-text-tight">
														{builtinRagMcpConfigured
															? "已加入当前 MCP 草稿，保存后当前 Agent 就能直接调用。"
															: builtinRagMcpServerStatus?.running
																? "开启后会把内置 RAG Query 写入当前草稿。"
																: builtinRagMcpServerStatus?.lastError ||
																	"桌面端启动后会自动暴露这个本地地址。"}
													</span>
													{builtinRagMcpConfigured && builtinRagMcpServer ? (
														<button
															className="settings-agent-secondary settings-button-compact"
															onClick={() => selectMcpServer(builtinRagMcpServer.id)}
															type="button"
														>
															编辑当前服务
														</button>
													) : null}
												</article>
												<div
													aria-label="MCP 服务目录"
													className="settings-mcp-catalog-group"
													role="radiogroup"
												>
													{regularMcpServers.map((server) => {
														const transportMeta = getMcpTransportMeta(server.transport);
														const issueCount = mcpValidation.serverIssues[server.id]?.length ?? 0;
														const isSelected = isMcpEditMode && selectedMcpServerId === server.id;
														const isFocusable =
															isSelected ||
															(!isMcpEditMode && regularMcpServers[0]?.id === server.id);
														return (
															<article
																className={`settings-mcp-list-item ${isSelected ? "settings-mcp-list-item-selected" : ""} ${
																	issueCount > 0 ? "settings-mcp-list-item-invalid" : ""
																}`}
																key={server.id}
															>
																<button
																	aria-checked={isSelected}
																	className="settings-mcp-list-item-main"
																	id={`settings-mcp-option-${server.id}`}
																	onClick={() => selectMcpServer(server.id)}
																	onKeyDown={(event) => handleMcpCatalogKeyDown(event, server.id)}
																	ref={bindMcpListOptionRef(server.id)}
																	role="radio"
																	tabIndex={isFocusable ? 0 : -1}
																	type="button"
																>
																	<div className="settings-agent-title-row">
																		<strong className="settings-agent-name">
																			{getMcpServerDraftTitle(server)}
																		</strong>
																	</div>
																	<div className="settings-mcp-list-item-badges">
																		<span className="settings-mcp-badge">
																			{transportMeta.label}
																		</span>
																		{issueCount > 0 ? (
																			<span className="settings-agent-meta">
																				{issueCount} 个问题
																			</span>
																		) : null}
																	</div>
																	<span className="settings-agent-command-preview">
																		{summarizeMcpServerDraft(server)}
																	</span>
																</button>
															</article>
														);
													})}
												</div>
												<article
													className={`settings-mcp-list-item settings-mcp-list-item-create ${
														isMcpCreateMode ? "settings-mcp-list-item-selected" : ""
													}`}
												>
													<button
														className="settings-mcp-list-item-main settings-mcp-create-trigger"
														onClick={handleEnterMcpCreateMode}
														type="button"
													>
														<div className="settings-agent-title-row">
															<strong className="settings-agent-name">新建服务</strong>
														</div>
														<div className="settings-mcp-list-item-badges">
															<span className="settings-mcp-badge">新建</span>
														</div>
														<span className="settings-agent-command-preview">
															只在需要另一种 transport 时新增，创建后会直接切到当前服务。
														</span>
													</button>
												</article>
											</div>
										</div>

										<div className="settings-acp-detail settings-mcp-detail">
											<SettingsDraftActionCard
												actions={
													<>
														{mcpValidation.totalIssues > 0 ? (
															<button
																className="settings-button settings-agent-secondary"
																onClick={locateFirstMcpIssue}
																type="button"
															>
																定位问题
															</button>
														) : null}
														<button
															className="settings-button settings-agent-secondary"
															disabled={savingMcp || !mcpHasUnsavedChanges}
															onClick={handleDiscardMcpDraft}
															type="button"
														>
															{DISCARD_DRAFT_BUTTON_LABEL}
														</button>
														<button
															className="settings-button"
															disabled={savingMcp || !mcpHasUnsavedChanges}
															onClick={() => void handleSaveMcp()}
															type="button"
														>
															{savingMcp ? "保存中..." : "保存 MCP 配置"}
														</button>
													</>
												}
												description={
													mcpValidation.totalIssues > 0
														? "先修复校验问题，再写回配置。"
														: mcpHasUnsavedChanges
															? "当前 MCP 草稿尚未写回配置。"
															: "MCP 配置已与本地 config.toml 同步。"
												}
												title={
													mcpValidation.totalIssues > 0
														? `先修复 ${mcpValidation.totalIssues} 个问题`
														: mcpHasUnsavedChanges
															? "有未保存的 MCP 草稿"
															: "MCP 配置已同步"
												}
											/>
											{isMcpEditMode ? (
												<>
													<div className="settings-acp-detail-header settings-mcp-detail-header">
														<div className="settings-acp-detail-copy">
															<span className="settings-section-kicker">当前服务</span>
															<h3 className="settings-subsection-title">
																{getMcpServerDraftTitle(selectedMcpServer)}
															</h3>
															<span className="settings-help-text settings-help-text-tight">
																目录和表单始终指向同一条服务。
															</span>
														</div>
														<button
															className="settings-agent-remove settings-agent-remove-inline"
															onClick={() => handleRemoveMcpServer(selectedMcpServer.id)}
															type="button"
														>
															删除服务
														</button>
													</div>

													<div
														className="settings-editor-card"
														id="mcp-form"
														ref={bindSectionBlockRef("mcp-form")}
													>
														<div className="settings-editor-card-header">
															<strong className="settings-agent-mcp-title">基础信息</strong>
															<span className="settings-agent-meta">
																{selectedMcpIssueCount > 0
																	? `${selectedMcpIssueCount} 个问题待处理`
																	: "名称会显示在左侧目录里"}
															</span>
														</div>

														<div className="settings-agent-fields">
															<label className="settings-label settings-label-stacked">
																<span>名称</span>
																<input
																	aria-describedby={joinDescribedByIds(
																		selectedMcpFieldIssues.name
																			? buildFieldIssueId("mcp", "name", selectedMcpServer.id)
																			: undefined,
																		`settings-mcp-${selectedMcpServer.id}-name-help`,
																	)}
																	aria-invalid={selectedMcpFieldIssues.name ? true : undefined}
																	className="settings-input settings-input-wide"
																	onChange={(event) =>
																		handleMcpServerFieldChange(
																			selectedMcpServer.id,
																			"name",
																			event.target.value,
																		)
																	}
																	placeholder="例如 filesystem"
																	ref={bindAcpFieldRef("server", selectedMcpServer.id, "name")}
																	type="text"
																	value={selectedMcpServer.name}
																/>
																<span
																	className="settings-help-text settings-help-text-tight"
																	id={`settings-mcp-${selectedMcpServer.id}-name-help`}
																>
																	用能力或数据源命名，后续切换服务时更容易辨认。
																</span>
																{selectedMcpFieldIssues.name ? (
																	<span
																		className="settings-field-error"
																		id={buildFieldIssueId("mcp", "name", selectedMcpServer.id)}
																	>
																		{selectedMcpFieldIssues.name}
																	</span>
																) : null}
															</label>
															<div className="settings-item settings-item-stacked">
																<span className="settings-label">连接类型</span>
																<div className="settings-mcp-transport-summary">
																	<span className="settings-mcp-badge">
																		{getMcpTransportMeta(selectedMcpServer.transport).label}
																	</span>
																	<span className="settings-help-text settings-help-text-tight">
																		连接类型在创建时决定。需要换 transport 时，直接新建一条更清楚。
																	</span>
																</div>
															</div>
														</div>
													</div>

													<div className="settings-editor-card">
														<div className="settings-editor-card-header">
															<strong className="settings-agent-mcp-title">连接配置</strong>
															<span className="settings-agent-meta">
																{getMcpTransportMeta(selectedMcpServer.transport).description}
															</span>
														</div>
														<div className="settings-agent-fields">
															{selectedMcpServer.transport === "stdio" ? (
																<>
																	<label className="settings-label settings-label-stacked">
																		<span>命令</span>
																		<input
																			aria-describedby={joinDescribedByIds(
																				selectedMcpFieldIssues.command
																					? buildFieldIssueId(
																							"mcp",
																							"command",
																							selectedMcpServer.id,
																						)
																					: undefined,
																			)}
																			aria-invalid={
																				selectedMcpFieldIssues.command ? true : undefined
																			}
																			className="settings-input settings-input-wide settings-input-mono"
																			onChange={(event) =>
																				handleMcpServerFieldChange(
																					selectedMcpServer.id,
																					"command",
																					event.target.value,
																				)
																			}
																			placeholder="例如 npx"
																			ref={bindAcpFieldRef(
																				"server",
																				selectedMcpServer.id,
																				"command",
																			)}
																			type="text"
																			value={selectedMcpServer.command}
																		/>
																		{selectedMcpFieldIssues.command ? (
																			<span
																				className="settings-field-error"
																				id={buildFieldIssueId(
																					"mcp",
																					"command",
																					selectedMcpServer.id,
																				)}
																			>
																				{selectedMcpFieldIssues.command}
																			</span>
																		) : null}
																	</label>
																	<label className="settings-label settings-label-stacked">
																		<span>参数</span>
																		<textarea
																			className="settings-textarea settings-input-mono"
																			onChange={(event) =>
																				handleMcpServerFieldChange(
																					selectedMcpServer.id,
																					"argsText",
																					event.target.value,
																				)
																			}
																			placeholder="每行一个参数"
																			rows={3}
																			value={selectedMcpServer.argsText}
																		/>
																	</label>
																	<label className="settings-label settings-label-stacked">
																		<span>环境变量</span>
																		<textarea
																			aria-describedby={joinDescribedByIds(
																				selectedMcpFieldIssues.envText
																					? buildFieldIssueId(
																							"mcp",
																							"env-text",
																							selectedMcpServer.id,
																						)
																					: undefined,
																			)}
																			aria-invalid={
																				selectedMcpFieldIssues.envText ? true : undefined
																			}
																			className="settings-textarea settings-input-mono"
																			onChange={(event) =>
																				handleMcpServerFieldChange(
																					selectedMcpServer.id,
																					"envText",
																					event.target.value,
																				)
																			}
																			placeholder="每行一个 KEY=VALUE"
																			ref={bindAcpFieldRef(
																				"server",
																				selectedMcpServer.id,
																				"envText",
																			)}
																			rows={3}
																			value={selectedMcpServer.envText}
																		/>
																		{selectedMcpFieldIssues.envText ? (
																			<span
																				className="settings-field-error"
																				id={buildFieldIssueId(
																					"mcp",
																					"env-text",
																					selectedMcpServer.id,
																				)}
																			>
																				{selectedMcpFieldIssues.envText}
																			</span>
																		) : null}
																	</label>
																</>
															) : (
																<>
																	<label className="settings-label settings-label-stacked">
																		<span>URL</span>
																		<input
																			aria-describedby={joinDescribedByIds(
																				selectedMcpFieldIssues.url
																					? buildFieldIssueId("mcp", "url", selectedMcpServer.id)
																					: undefined,
																				`settings-mcp-${selectedMcpServer.id}-url-help`,
																			)}
																			aria-invalid={selectedMcpFieldIssues.url ? true : undefined}
																			className="settings-input settings-input-wide settings-input-mono"
																			onChange={(event) =>
																				handleMcpServerFieldChange(
																					selectedMcpServer.id,
																					"url",
																					event.target.value,
																				)
																			}
																			placeholder="https://example.com/mcp"
																			ref={bindAcpFieldRef("server", selectedMcpServer.id, "url")}
																			type="text"
																			value={selectedMcpServer.url}
																		/>
																		<span
																			className="settings-help-text settings-help-text-tight"
																			id={`settings-mcp-${selectedMcpServer.id}-url-help`}
																		>
																			支持 `http://` 或 `https://`，例如{" "}
																			{getMcpTransportMeta(selectedMcpServer.transport).example}
																		</span>
																		{selectedMcpFieldIssues.url ? (
																			<span
																				className="settings-field-error"
																				id={buildFieldIssueId("mcp", "url", selectedMcpServer.id)}
																			>
																				{selectedMcpFieldIssues.url}
																			</span>
																		) : null}
																	</label>
																	<label className="settings-label settings-label-stacked">
																		<span>请求头</span>
																		<textarea
																			aria-describedby={joinDescribedByIds(
																				selectedMcpFieldIssues.headersText
																					? buildFieldIssueId(
																							"mcp",
																							"headers-text",
																							selectedMcpServer.id,
																						)
																					: undefined,
																			)}
																			aria-invalid={
																				selectedMcpFieldIssues.headersText ? true : undefined
																			}
																			className="settings-textarea settings-input-mono"
																			onChange={(event) =>
																				handleMcpServerFieldChange(
																					selectedMcpServer.id,
																					"headersText",
																					event.target.value,
																				)
																			}
																			placeholder="每行一个 KEY=VALUE"
																			ref={bindAcpFieldRef(
																				"server",
																				selectedMcpServer.id,
																				"headersText",
																			)}
																			rows={3}
																			value={selectedMcpServer.headersText}
																		/>
																		<span className="settings-help-text settings-help-text-tight">
																			需要鉴权时再填写。每行一个 `KEY=VALUE`。
																		</span>
																		{selectedMcpFieldIssues.headersText ? (
																			<span
																				className="settings-field-error"
																				id={buildFieldIssueId(
																					"mcp",
																					"headers-text",
																					selectedMcpServer.id,
																				)}
																			>
																				{selectedMcpFieldIssues.headersText}
																			</span>
																		) : null}
																	</label>
																</>
															)}
														</div>
													</div>
												</>
											) : null}
											{isMcpCreateMode ? renderMcpCreateCard() : null}
										</div>
									</div>
								</section>
							) : null}

							{activeSection === "skills" ? (
								<section
									aria-labelledby={getSettingsTabId("skills")}
									className="settings-section"
									id={getSettingsPanelId("skills")}
									role="tabpanel"
									tabIndex={0}
								>
									<h2 className="settings-section-title">Skill</h2>
									{skillError ? (
										<div className="settings-banner settings-banner-error">{skillError}</div>
									) : null}

									<div className="settings-skills-layout">
										<div
											className="settings-editor-card settings-editor-card-subtle settings-skill-catalog-panel"
											id="skills-catalog"
											ref={bindSectionBlockRef("skills-catalog")}
										>
											<div className="settings-editor-card-header">
												<div className="settings-acp-sidebar-copy">
													<span className="settings-section-kicker">公共目录</span>
													<strong className="settings-agent-mcp-title">
														{skillCatalog.skills.length} 个 skill
													</strong>
												</div>
											</div>
											<p className="settings-help-text settings-help-text-tight">
												当前只读扫描{" "}
												<code className="settings-inline-code">{skillCatalog.rootPath}</code>
												，先在上面选一个 skill，再看下面的详情。
											</p>

											{!skillCatalog.exists ? (
												<div className="settings-empty-panel settings-empty-panel-subtle">
													<strong className="settings-empty-title">目录不存在</strong>
													<span className="settings-help-text settings-help-text-tight">
														没找到公共 skill 目录。当前只读取 `~/.agents/skills`。
													</span>
												</div>
											) : skillCatalog.skills.length === 0 ? (
												<div className="settings-empty-panel settings-empty-panel-subtle">
													<strong className="settings-empty-title">没有公共 skill</strong>
													<span className="settings-help-text settings-help-text-tight">
														目录存在，但下面没有可展示的 skill 子目录。
													</span>
												</div>
											) : (
												<div className="settings-catalog-grid settings-catalog-grid-3">
													{skillCatalog.skills.map((skill) => {
														const isSelected = selectedSkillId === skill.id;
														const skillTitle = skill.meta.name?.trim() || skill.directoryName;
														return (
															<article
																className={`settings-agent-list-item settings-skill-list-item ${isSelected ? "settings-agent-list-item-selected" : ""}`}
																key={skill.id}
															>
																<button
																	className="settings-agent-list-item-main settings-skill-list-item-main"
																	onClick={() => handleViewSkill(skill.id)}
																	type="button"
																>
																	<div className="settings-agent-title-row">
																		<strong className="settings-agent-name">{skillTitle}</strong>
																	</div>
																	<span className="settings-agent-command-preview">
																		{skill.relativePath}
																	</span>
																	<div className="settings-mcp-list-item-badges">
																		<span className="settings-status-chip">
																			{skill.directoryCount} 目录
																		</span>
																		<span className="settings-status-chip">
																			{skill.fileCount} 文件
																		</span>
																	</div>
																</button>
															</article>
														);
													})}
												</div>
											)}
										</div>

										<div
											className="settings-acp-detail"
											id="skills-detail"
											ref={bindSectionBlockRef("skills-detail")}
										>
											{selectedSkill ? (
												<>
													<div className="settings-acp-detail-header">
														<div className="settings-acp-detail-copy">
															<span className="settings-section-kicker">当前 Skill</span>
															<h3 className="settings-subsection-title">
																{selectedSkill.meta.name?.trim() || selectedSkill.directoryName}
															</h3>
														</div>
													</div>

													<div className="settings-editor-card">
														<div className="settings-editor-card-header">
															<strong className="settings-agent-mcp-title">Meta 信息</strong>
															<span className="settings-agent-meta">
																{selectedSkill.directoryCount} 个目录 · {selectedSkill.fileCount}{" "}
																个文件
															</span>
														</div>
														<div className="settings-skill-table-wrap">
															<table className="settings-skill-table">
																<tbody>
																	<tr>
																		<th scope="row">目录</th>
																		<td>
																			<code className="settings-inline-code">
																				{selectedSkill.relativePath}
																			</code>
																		</td>
																	</tr>
																	<tr>
																		<th scope="row">名称</th>
																		<td>{selectedSkill.meta.name ?? "未声明"}</td>
																	</tr>
																	<tr>
																		<th scope="row">描述</th>
																		<td>{selectedSkill.meta.description ?? "未声明"}</td>
																	</tr>
																	<tr>
																		<th scope="row">参数提示</th>
																		<td>{selectedSkill.meta.argumentHint ?? "未声明"}</td>
																	</tr>
																	<tr>
																		<th scope="row">License</th>
																		<td>{selectedSkill.meta.license ?? "未声明"}</td>
																	</tr>
																	<tr>
																		<th scope="row">目录数</th>
																		<td>{selectedSkill.directoryCount}</td>
																	</tr>
																	<tr>
																		<th scope="row">文件数</th>
																		<td>{selectedSkill.fileCount}</td>
																	</tr>
																</tbody>
															</table>
														</div>
														<div className="settings-editor-card-header">
															<strong className="settings-agent-mcp-title">metadata</strong>
															<span className="settings-agent-meta">
																{selectedSkill.meta.metadata.length > 0
																	? `${selectedSkill.meta.metadata.length} 项`
																	: "未声明"}
															</span>
														</div>
														{selectedSkill.meta.metadata.length > 0 ? (
															<div className="settings-skill-table-wrap">
																<table className="settings-skill-table">
																	<thead>
																		<tr>
																			<th scope="col">Key</th>
																			<th scope="col">Value</th>
																		</tr>
																	</thead>
																	<tbody>
																		{selectedSkill.meta.metadata.map((entry) => (
																			<tr key={`${entry.key}:${entry.value}`}>
																				<td>{entry.key}</td>
																				<td>{entry.value}</td>
																			</tr>
																		))}
																	</tbody>
																</table>
															</div>
														) : (
															<span className="settings-help-text settings-help-text-tight">
																`metadata` 段为空或未声明。
															</span>
														)}
													</div>

													<div className="settings-editor-card">
														<div className="settings-editor-card-header">
															<strong className="settings-agent-mcp-title">目录树</strong>
															<span className="settings-agent-meta">
																只展示名称与层级，不读取文件内容
															</span>
														</div>
														<div className="settings-skill-tree-panel">
															{renderSkillTreeNode(selectedSkill.tree)}
														</div>
													</div>
												</>
											) : (
												<div className="settings-empty-panel">
													<strong className="settings-empty-title">没有选中的 skill</strong>
													<span className="settings-help-text settings-help-text-tight">
														先从上面的卡片里选择一个公共 skill，下面才会显示 meta 和目录树。
													</span>
												</div>
											)}
										</div>
									</div>
								</section>
							) : null}

							{activeSection === "about" ? (
								<section
									aria-labelledby={getSettingsTabId("about")}
									className="settings-section"
									id={getSettingsPanelId("about")}
									role="tabpanel"
									tabIndex={0}
								>
									<h2 className="settings-section-title">关于</h2>
									<div
										className="settings-editor-card settings-editor-card-subtle"
										id="about-overview"
										ref={bindSectionBlockRef("about-overview")}
									>
										<div className="settings-item">
											<span className="settings-info-label">版本</span>
											<span className="settings-info-value">0.1.0</span>
										</div>
										<div className="settings-item">
											<span className="settings-info-label">检查更新</span>
											<button className="settings-button" type="button">
												检查更新
											</button>
										</div>
									</div>
								</section>
							) : null}
						</div>
					</div>
				</div>
			</section>
		</main>
	);
}
