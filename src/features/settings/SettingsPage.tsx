import { useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
	chooseDirectory,
	getAppSettings,
	getAcpAgents,
	getAcpMcpServers,
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
} from "../../lib/tauri/client";
import type {
	AcpAgentConfig,
	AcpMcpServerConfig,
	AppSettings,
	AppearanceSettings,
	GeneralSettings,
	LlmProviderConfig,
	LlmSettings,
	OcrSettings,
	PublicSkillCatalog,
	RagScanResult,
	RagSettings,
	ShortcutConfig,
	SkillTreeNode,
	WorkspaceState,
} from "../../lib/tauri/types";
import { useSettingsWindowFrame } from "./useSettingsWindowFrame";
import "./settings.css";

interface SettingsPageProps {
	onBack: () => void;
}

type SettingsSectionId =
	| "general"
	| "shortcuts"
	| "appearance"
	| "llm"
	| "rag"
	| "acp"
	| "mcp"
	| "skills"
	| "about";

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
			<span className="settings-agent-meta">新建 {transportMeta.label}</span>
			<p className="settings-install-guide-summary">{transportMeta.summary}</p>
			<div className="settings-install-guide-item">
				<strong className="settings-install-guide-item-title">会创建的字段</strong>
				<span className="settings-help-text settings-help-text-tight">
					{transportMeta.fieldsHint}
				</span>
			</div>
			<div className="settings-install-guide-item">
				<strong className="settings-install-guide-item-title">示例</strong>
				<code className="settings-install-guide-command">{transportMeta.example}</code>
			</div>
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

interface AcpAgentDraft {
	id: string;
	name: string;
	command: string;
}

type McpTransport = AcpMcpServerConfig["transport"];
type LlmProviderProtocolKind = LlmProviderConfig["protocol"];

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
}

interface SavedLlmDraftState {
	providers: LlmProviderConfig[];
	defaultProviderId: string | null;
}

interface RagDraftValidation {
	totalIssues: number;
	issues: string[];
}

interface SavedRagDraftState {
	sourceDirectories: string[];
	ignoreGlobs: string[];
	embeddingProviderId: string | null;
}

interface McpDraftValidation {
	totalIssues: number;
	serverIssues: Record<string, string[]>;
}

interface AcpInlineNotice {
	tone: "info" | "warn";
	text: string;
}

interface AcpIssueFocusTarget {
	agentId: string;
	fieldKey: "name" | "command";
}

interface McpIssueFocusTarget {
	serverId: string;
	serverFieldKey: "name" | "command" | "url" | "envText" | "headersText";
}

interface LlmIssueFocusTarget {
	providerId: string;
	fieldKey: "name" | "baseUrl" | "model";
}

const settingsSections: ReadonlyArray<{
	id: SettingsSectionId;
	label: string;
}> = [
	{ id: "general", label: "通用" },
	{ id: "shortcuts", label: "快捷键" },
	{ id: "appearance", label: "外观" },
	{ id: "llm", label: "LLM" },
	{ id: "rag", label: "RAG" },
	{ id: "acp", label: "ACP Agent" },
	{ id: "mcp", label: "MCP" },
	{ id: "skills", label: "Skills" },
	{ id: "about", label: "关于" },
] as const;

const llmProtocolOptions: ReadonlyArray<{
	protocol: LlmProviderProtocolKind;
	label: string;
	summary: string;
}> = [
	{
		protocol: "openai_chat",
		label: "OpenAI Chat",
		summary: "使用 `chat/completions`。当前 OCR 只支持这条协议。",
	},
	{
		protocol: "openai_responses",
		label: "OpenAI Responses",
		summary: "使用 `responses` 接口，适合新一代 OpenAI 风格响应链路。",
	},
	{
		protocol: "openai_embedding",
		label: "OpenAI Embedding",
		summary: "使用 `embeddings` 接口，只适合向量模型，不允许开启多模态。",
	},
] as const;

const customLlmModelOptionValue = "__custom__";
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
		fieldsHint: "需要填写 Command；可选填写 Args 和 Env。",
		example: "npx -y @modelcontextprotocol/server-filesystem ~/Desktop",
	},
	{
		transport: "http",
		label: "HTTP",
		description: "通过 URL 连接远程 MCP server",
		summary: "适合已经部署好的远程 MCP 服务，保存后会把 URL 和请求头随会话一起交给当前 Agent。",
		fieldsHint: "需要填写 URL；可选填写 Headers。",
		example: "https://example.com/mcp",
	},
	{
		transport: "sse",
		label: "SSE",
		description: "通过 SSE 流连接远程 MCP server",
		summary: "适合使用服务端事件流暴露能力的远程 MCP 服务，字段和 HTTP 类似，但连接语义是 SSE。",
		fieldsHint: "需要填写 URL；可选填写 Headers。",
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
		defaultProviderId: state.defaultProviderId,
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
			url: server.url.trim(),
			headers: parseKeyValueLines(server.headersText, "MCP headers"),
		};
	}

	return {
		transport: "sse",
		name: server.name.trim(),
		url: server.url.trim(),
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
		protocol: "openai_chat",
		baseUrl: "https://api.openai.com/v1",
		apiKey: "",
		model: "",
		supportsMultimodal: false,
	};
}

function getLlmProtocolLabel(protocol: LlmProviderProtocolKind) {
	return llmProtocolOptions.find((option) => option.protocol === protocol)?.label ?? protocol;
}

function canProviderSupportMultimodal(provider: Pick<LlmProviderConfig, "protocol">) {
	return provider.protocol !== "openai_embedding";
}

function summarizeLlmProviderProfile(provider: LlmProviderConfig) {
	const parts = [getLlmProtocolLabel(provider.protocol)];
	if (provider.supportsMultimodal) {
		parts.push("多模态");
	} else {
		parts.push("纯文本");
	}

	return parts.join(" · ");
}

function resolveLlmModelSelectValue(provider: LlmProviderConfig, models: string[]) {
	if (!provider.model.trim()) {
		return "";
	}

	return models.includes(provider.model) ? provider.model : customLlmModelOptionValue;
}

function getMcpTransportMeta(transport: McpTransport) {
	return (
		mcpTransportOptions.find((option) => option.transport === transport) ?? mcpTransportOptions[0]
	);
}

function summarizeMcpServerDraft(server: AcpMcpServerDraft) {
	if (server.transport === "stdio") {
		return server.command.trim() || "还没有本地命令";
	}

	return server.url.trim() || "还没有连接地址";
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

function buildSavedLlmDraftState(
	providers: LlmProviderConfig[],
	defaultProviderId: string | null,
): SavedLlmDraftState {
	return cloneSavedLlmDraftState({
		providers,
		defaultProviderId,
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

function buildLlmDraftSnapshot(settings: LlmSettings) {
	return JSON.stringify({
		defaultProviderId: settings.defaultProviderId,
		providers: settings.providers.map((provider) => ({
			id: provider.id,
			name: provider.name,
			protocol: provider.protocol,
			baseUrl: provider.baseUrl,
			apiKey: provider.apiKey,
			model: provider.model,
			supportsMultimodal: provider.supportsMultimodal,
		})),
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
	let totalIssues = 0;

	agents.forEach((agent, agentIndex) => {
		const currentAgentIssues: string[] = [];
		if (!agent.name.trim()) {
			currentAgentIssues.push(`第 ${agentIndex + 1} 个 agent 缺少名称。`);
		}
		if (!agent.command.trim()) {
			currentAgentIssues.push(`第 ${agentIndex + 1} 个 agent 缺少启动命令。`);
		}

		if (currentAgentIssues.length > 0) {
			agentIssues[agent.id] = currentAgentIssues;
			totalIssues += currentAgentIssues.length;
		}
	});

	return {
		totalIssues,
		agentIssues,
	};
}

function validateLlmSettings(settings: LlmSettings): LlmDraftValidation {
	const providerIssues: Record<string, string[]> = {};
	let totalIssues = 0;

	settings.providers.forEach((provider, providerIndex) => {
		const currentProviderIssues: string[] = [];
		if (!provider.name.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个 provider 缺少名称。`);
		}
		if (!provider.baseUrl.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个 provider 缺少 Base URL。`);
		}
		if (!provider.model.trim()) {
			currentProviderIssues.push(`第 ${providerIndex + 1} 个 provider 缺少模型名。`);
		}

		if (currentProviderIssues.length > 0) {
			providerIssues[provider.id] = currentProviderIssues;
			totalIssues += currentProviderIssues.length;
		}
	});

	if (
		settings.defaultProviderId &&
		!settings.providers.some((provider) => provider.id === settings.defaultProviderId)
	) {
		totalIssues += 1;
	}

	return {
		totalIssues,
		providerIssues,
	};
}

function validateRagSettings(settings: RagSettings, llmSettings: LlmSettings): RagDraftValidation {
	const issues: string[] = [];

	settings.sourceDirectories.forEach((directory, index) => {
		if (!directory.trim()) {
			issues.push(`第 ${index + 1} 个扫描目录不能为空。`);
		}
	});

	settings.ignoreGlobs.forEach((pattern, index) => {
		if (!pattern.trim()) {
			issues.push(`第 ${index + 1} 个忽略模式不能为空。`);
		}
	});

	if (settings.sourceDirectories.length > 0 && !settings.embeddingProviderId) {
		issues.push("配置扫描目录时，必须选择一个 embedding provider。");
	}

	if (
		settings.embeddingProviderId &&
		!llmSettings.providers.some(
			(provider) =>
				provider.id === settings.embeddingProviderId && provider.protocol === "openai_embedding",
		)
	) {
		issues.push("RAG 选择的 embedding provider 不存在，或者协议不是 OpenAI Embedding。");
	}

	return {
		totalIssues: issues.length,
		issues,
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
	let totalIssues = 0;

	servers.forEach((server, serverIndex) => {
		const currentServerIssues: string[] = [];
		if (!server.name.trim()) {
			currentServerIssues.push(`第 ${serverIndex + 1} 个 MCP server 缺少名称。`);
		}

		if (server.transport === "stdio") {
			if (!server.command.trim()) {
				currentServerIssues.push("本地进程模式必须填写 command。");
			}

			try {
				parseKeyValueLines(server.envText, "MCP env");
			} catch (error: unknown) {
				currentServerIssues.push(getErrorMessage(error, "MCP env 格式错误。"));
			}
		} else {
			if (!server.url.trim()) {
				currentServerIssues.push(
					`${getMcpTransportMeta(server.transport).label} 模式必须填写 URL。`,
				);
			}

			try {
				parseKeyValueLines(server.headersText, "MCP headers");
			} catch (error: unknown) {
				currentServerIssues.push(getErrorMessage(error, "MCP headers 格式错误。"));
			}
		}

		if (currentServerIssues.length > 0) {
			serverIssues[server.id] = currentServerIssues;
			totalIssues += currentServerIssues.length;
		}
	});

	return {
		totalIssues,
		serverIssues,
	};
}

export function SettingsPage({ onBack }: SettingsPageProps) {
	const [activeSection, setActiveSection] = useState<SettingsSectionId>("general");
	const [generalSettings, setGeneralSettings] = useState<GeneralSettings>({
		autoStart: false,
		showInDock: true,
		language: "zh-CN",
	});

	const [shortcutSettings, setShortcutSettings] = useState<ShortcutConfig>({
		toggle_launcher: "Cmd+Shift+Space",
		ocr_capture: "Cmd+Shift+O",
	});
	const [llmSettings, setLlmSettings] = useState<LlmSettings>({
		providers: [],
		defaultProviderId: null,
	});
	const [selectedLlmProviderId, setSelectedLlmProviderId] = useState<string | null>(null);
	const [savedLlmSnapshot, setSavedLlmSnapshot] = useState(() =>
		buildLlmDraftSnapshot({
			providers: [],
			defaultProviderId: null,
		}),
	);
	const [savedLlmState, setSavedLlmState] = useState<SavedLlmDraftState>(() =>
		buildSavedLlmDraftState([], null),
	);
	const [llmModelOptions, setLlmModelOptions] = useState<Record<string, string[]>>({});
	const [llmModelErrors, setLlmModelErrors] = useState<Record<string, string>>({});
	const [loadingLlmModelProviderId, setLoadingLlmModelProviderId] = useState<string | null>(null);
	const [ragSettings, setRagSettings] = useState<RagSettings>({
		sourceDirectories: [],
		ignoreGlobs: [],
		embeddingProviderId: null,
	});
	const [savedRagSnapshot, setSavedRagSnapshot] = useState(() =>
		buildRagDraftSnapshot({
			sourceDirectories: [],
			ignoreGlobs: [],
			embeddingProviderId: null,
		}),
	);
	const [savedRagState, setSavedRagState] = useState<SavedRagDraftState>(() =>
		buildSavedRagDraftState({
			sourceDirectories: [],
			ignoreGlobs: [],
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
		llm: {
			providers: [],
			defaultProviderId: null,
		},
		ocr: {
			provider: "system",
			llmProviderId: null,
		},
		rag: {
			sourceDirectories: [],
			ignoreGlobs: [],
			embeddingProviderId: null,
		},
	});

	// Track which shortcut is being edited
	const [editingShortcut, setEditingShortcut] = useState<keyof ShortcutConfig | null>(null);
	const [savingShortcutKey, setSavingShortcutKey] = useState<keyof ShortcutConfig | null>(null);
	const [savingSettings, setSavingSettings] = useState(false);
	const [savingLlm, setSavingLlm] = useState(false);
	const [savingOcr, setSavingOcr] = useState(false);
	const [savingRag, setSavingRag] = useState(false);
	const [scanningRag, setScanningRag] = useState(false);
	const [savingMcp, setSavingMcp] = useState(false);
	const [settingsError, setSettingsError] = useState<string | null>(null);
	const acpFieldRefs = useRef<Record<string, HTMLInputElement | HTMLTextAreaElement | null>>({});
	const llmFieldRefs = useRef<Record<string, HTMLInputElement | null>>({});
	const shellRef = useRef<HTMLElement | null>(null);
	const frameHeight = useSettingsWindowFrame(shellRef);
	const showLlmOcrFields = ocrSettings.provider === "llm_ocr";
	const ragSourceDirectoryPlaceholder = resolveDocumentsDirectoryPath(workspaceContext);
	const ragDirectoryPickerDefaultPath = resolveRagDirectoryPickerDefaultPath(workspaceContext);

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
	}

	async function handleFetchLlmProviderModels(provider: LlmProviderConfig) {
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

	// Load initial shortcut config
	useEffect(() => {
		void getAppSettings().then((settings) => {
			setGeneralSettings(settings.general);
			setAppearanceSettings(settings.appearance);
			setLlmSettings(settings.llm);
			setSelectedLlmProviderId(settings.llm.providers[0]?.id ?? null);
			setSavedLlmSnapshot(buildLlmDraftSnapshot(settings.llm));
			setSavedLlmState(
				buildSavedLlmDraftState(settings.llm.providers, settings.llm.defaultProviderId),
			);
			setLlmModelOptions({});
			setLlmModelErrors({});
			setLoadingLlmModelProviderId(null);
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
			setSavedMcpSnapshot(buildMcpDraftSnapshot(draftServers));
			setSavedMcpState(buildSavedMcpDraftState(draftServers));
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
	}, []);

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
		if (ocrSettings.provider !== "llm_ocr" || !ocrSettings.llmProviderId) {
			return;
		}

		const selectedOcrProvider = llmSettings.providers.find(
			(provider) => provider.id === ocrSettings.llmProviderId,
		);
		if (
			selectedOcrProvider &&
			selectedOcrProvider.protocol === "openai_chat" &&
			selectedOcrProvider.supportsMultimodal
		) {
			return;
		}

		setOcrSettings((current) =>
			current.provider === "llm_ocr" ? { ...current, llmProviderId: null } : current,
		);
	}, [llmSettings.providers, ocrSettings.llmProviderId, ocrSettings.provider]);

	useEffect(() => {
		if (!ragSettings.embeddingProviderId) {
			return;
		}

		const selectedEmbeddingProvider = llmSettings.providers.find(
			(provider) => provider.id === ragSettings.embeddingProviderId,
		);
		if (selectedEmbeddingProvider?.protocol === "openai_embedding") {
			return;
		}

		setRagSettings((current) => ({ ...current, embeddingProviderId: null }));
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
		const nextSelectedSkillId = selectExistingIdOrFirst(skillCatalog.skills, selectedSkillId);
		if (nextSelectedSkillId !== selectedSkillId) {
			setSelectedSkillId(nextSelectedSkillId);
		}
	}, [selectedSkillId, skillCatalog.skills]);

	const handleShortcutClick = (key: keyof ShortcutConfig) => {
		setEditingShortcut(key);
	};

	const isRecording = (key: keyof ShortcutConfig) => editingShortcut === key;
	const [savingAgent, setSavingAgent] = useState(false);
	const llmValidation = validateLlmSettings(llmSettings);
	const ragValidation = validateRagSettings(ragSettings, llmSettings);
	const acpValidation = validateAcpAgents(acpAgents);
	const mcpValidation = validateMcpServers(mcpServers);
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
	const selectedLlmProviderModels = selectedLlmProvider
		? (llmModelOptions[selectedLlmProvider.id] ?? [])
		: [];
	const selectedLlmModelSelectValue = selectedLlmProvider
		? resolveLlmModelSelectValue(selectedLlmProvider, selectedLlmProviderModels)
		: "";
	const selectedLlmProviderModelsError = selectedLlmProvider
		? (llmModelErrors[selectedLlmProvider.id] ?? null)
		: null;
	const isLoadingSelectedLlmProviderModels =
		selectedLlmProvider !== null && loadingLlmModelProviderId === selectedLlmProvider.id;
	const eligibleOcrProviders = llmSettings.providers.filter(
		(provider) => provider.protocol === "openai_chat" && provider.supportsMultimodal,
	);
	const eligibleRagEmbeddingProviders = llmSettings.providers.filter(
		(provider) => provider.protocol === "openai_embedding",
	);
	const selectedAgent = acpAgents.find((agent) => agent.id === selectedAgentId) ?? null;
	const selectedMcpServer = mcpServers.find((server) => server.id === selectedMcpServerId) ?? null;
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
	const selectedSkill = skillCatalog.skills.find((skill) => skill.id === selectedSkillId) ?? null;

	function buildAcpFieldRefKey(
		scope: "agent" | "server",
		id: string,
		field: "name" | "command" | "url" | "envText" | "headersText",
	) {
		return `${scope}:${id}:${field}`;
	}

	function bindAcpFieldRef(
		scope: "agent" | "server",
		id: string,
		field: "name" | "command" | "url" | "envText" | "headersText",
	) {
		const refKey = buildAcpFieldRefKey(scope, id, field);
		return (node: HTMLInputElement | HTMLTextAreaElement | null) => {
			acpFieldRefs.current[refKey] = node;
		};
	}

	function focusAcpField(
		scope: "agent" | "server",
		id: string,
		field: "name" | "command" | "url" | "envText" | "headersText",
	) {
		const refKey = buildAcpFieldRefKey(scope, id, field);
		window.requestAnimationFrame(() => {
			acpFieldRefs.current[refKey]?.focus();
		});
	}

	function buildLlmFieldRefKey(id: string, field: "name" | "baseUrl" | "model") {
		return `${id}:${field}`;
	}

	function bindLlmFieldRef(id: string, field: "name" | "baseUrl" | "model") {
		const refKey = buildLlmFieldRefKey(id, field);
		return (node: HTMLInputElement | null) => {
			llmFieldRefs.current[refKey] = node;
		};
	}

	function focusLlmField(id: string, field: "name" | "baseUrl" | "model") {
		const refKey = buildLlmFieldRefKey(id, field);
		window.requestAnimationFrame(() => {
			llmFieldRefs.current[refKey]?.focus();
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
		setSelectedMcpServerId(nextIssue.serverId);
		focusAcpField("server", nextIssue.serverId, nextIssue.serverFieldKey);
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
			applyMcpServerDrafts(
				nextDraftServers,
				nextDraftServers.some((server) => server.id === selectedMcpServerId)
					? selectedMcpServerId
					: (nextDraftServers[0]?.id ?? null),
			);
			setSavedMcpSnapshot(buildMcpDraftSnapshot(nextDraftServers));
			setSavedMcpState(buildSavedMcpDraftState(nextDraftServers));
		} catch (error: unknown) {
			setSettingsError(getErrorMessage(error, "MCP 配置保存失败"));
		} finally {
			setSavingMcp(false);
		}
	}

	async function saveAppSettings(nextGeneral: GeneralSettings, nextAppearance: AppearanceSettings) {
		await persistAppSettings(
			{
				general: nextGeneral,
				appearance: nextAppearance,
				llm: persistedAppSettings.llm,
				ocr: persistedAppSettings.ocr,
				rag: persistedAppSettings.rag,
			},
			setSavingSettings,
			"设置保存失败",
			{
				adoptLlm: false,
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
				llm: llmSettings,
				ocr: ocrSettings,
				rag: persistedAppSettings.rag,
			},
			setSavingLlm,
			"LLM 配置保存失败",
			{
				adoptLlm: true,
				adoptOcr: true,
				adoptRag: false,
			},
		);
	}

	async function handleSaveOcr() {
		await persistAppSettings(
			{
				general: persistedAppSettings.general,
				appearance: persistedAppSettings.appearance,
				llm: persistedAppSettings.llm,
				ocr: ocrSettings,
				rag: persistedAppSettings.rag,
			},
			setSavingOcr,
			"OCR 配置保存失败",
			{
				adoptLlm: false,
				adoptOcr: true,
				adoptRag: false,
			},
		);
	}

	async function handleSaveRag() {
		if (ragValidation.totalIssues > 0) {
			setActiveSection("rag");
			setSettingsError(ragValidation.issues[0] ?? "RAG 配置不合法");
			return;
		}

		await persistAppSettings(
			{
				general: persistedAppSettings.general,
				appearance: persistedAppSettings.appearance,
				llm: persistedAppSettings.llm,
				ocr: persistedAppSettings.ocr,
				rag: ragSettings,
			},
			setSavingRag,
			"RAG 配置保存失败",
			{
				adoptLlm: false,
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
			adoptLlm: boolean;
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
			if (options.adoptLlm) {
				setLlmSettings(saved.llm);
				setSelectedLlmProviderId(
					saved.llm.providers.some((provider) => provider.id === selectedLlmProviderId)
						? selectedLlmProviderId
						: (saved.llm.providers[0]?.id ?? null),
				);
				setSavedLlmSnapshot(buildLlmDraftSnapshot(saved.llm));
				setSavedLlmState(buildSavedLlmDraftState(saved.llm.providers, saved.llm.defaultProviderId));
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
		setLlmSettings((current) => ({
			providers: [...current.providers, nextProvider],
			defaultProviderId: current.defaultProviderId ?? nextProvider.id,
		}));
		setSelectedLlmProviderId(nextProvider.id);
		setSettingsError(null);
	}

	function handleRemoveLlmProvider(providerId: string) {
		const nextSelectedProviderId =
			selectedLlmProviderId === providerId
				? (llmSettings.providers.find((provider) => provider.id !== providerId)?.id ?? null)
				: selectedLlmProviderId;
		setLlmSettings((current) => {
			const nextProviders = current.providers.filter((provider) => provider.id !== providerId);
			const nextDefaultProviderId =
				current.defaultProviderId === providerId
					? (nextProviders[0]?.id ?? null)
					: current.defaultProviderId;
			return {
				providers: nextProviders,
				defaultProviderId: nextDefaultProviderId,
			};
		});
		setSelectedLlmProviderId(nextSelectedProviderId);
		clearLlmModelCatalog(providerId);
		if (ocrSettings.llmProviderId === providerId) {
			setOcrSettings((current) => ({
				...current,
				llmProviderId: null,
			}));
		}
		if (ragSettings.embeddingProviderId === providerId) {
			setRagSettings((current) => ({
				...current,
				embeddingProviderId: null,
			}));
		}
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

		setLlmSettings((current) => ({
			...current,
			providers: current.providers.map((provider) =>
				provider.id === providerId
					? (() => {
							const nextProvider = {
								...provider,
								[key]: value,
							};
							if (key === "protocol" && value === "openai_embedding") {
								nextProvider.supportsMultimodal = false;
							}
							return nextProvider;
						})()
					: provider,
			),
		}));
		if (
			(key === "protocol" && value !== "openai_chat") ||
			(key === "supportsMultimodal" && value === false)
		) {
			setOcrSettings((current) =>
				current.llmProviderId === providerId ? { ...current, llmProviderId: null } : current,
			);
		}
		if (key === "protocol" && value !== "openai_embedding") {
			setRagSettings((current) =>
				current.embeddingProviderId === providerId
					? { ...current, embeddingProviderId: null }
					: current,
			);
		}
		setSettingsError(null);
	}

	function handleLlmModelSelectionChange(providerId: string, value: string) {
		if (value === customLlmModelOptionValue) {
			focusLlmField(providerId, "model");
			return;
		}

		handleLlmProviderFieldChange(providerId, "model", value);
	}

	function handleLlmDefaultProviderChange(providerId: string | null) {
		setLlmSettings((current) => ({
			...current,
			defaultProviderId: providerId ?? current.defaultProviderId,
		}));
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
			setActiveSection("rag");
			setSettingsError(ragValidation.issues[0] ?? "RAG 配置不合法");
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
			text: `已创建 ${transportMeta.label} MCP 草稿。补全参数后再保存。`,
		});
		applyMcpServerDrafts([...mcpServers, nextServer], nextServer.id);
	}

	function handleRemoveMcpServer(serverId: string) {
		setMcpNotice(null);
		const nextServers = mcpServers.filter((server) => server.id !== serverId);
		applyMcpServerDrafts(
			nextServers,
			selectedMcpServerId === serverId ? (nextServers[0]?.id ?? null) : selectedMcpServerId,
		);
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

	function handleDiscardAcpChanges() {
		const restored = cloneSavedAcpDraftState(savedAcpState);
		setAcpNotice(null);
		applyAgentDrafts(restored.agents, restored.agents[0]?.id ?? null);
		setDefaultAgentId(restored.defaultAgentId);
		setSettingsError(null);
	}

	function handleDiscardLlmChanges() {
		const restored = cloneSavedLlmDraftState(savedLlmState);
		setLlmSettings({
			providers: restored.providers,
			defaultProviderId: restored.defaultProviderId,
		});
		setSelectedLlmProviderId(restored.providers[0]?.id ?? null);
		setSettingsError(null);
	}

	function handleDiscardRagChanges() {
		const restored = cloneSavedRagDraftState(savedRagState);
		setRagSettings({
			sourceDirectories: restored.sourceDirectories,
			ignoreGlobs: restored.ignoreGlobs,
			embeddingProviderId: restored.embeddingProviderId,
		});
		setSettingsError(null);
	}

	function handleDiscardMcpChanges() {
		const restored = cloneSavedMcpDraftState(savedMcpState);
		setMcpNotice(null);
		applyMcpServerDrafts(restored.servers, restored.servers[0]?.id ?? null);
		setSettingsError(null);
	}
	return (
		<main className="settings-shell" ref={shellRef}>
			<section
				className="settings-frame"
				style={{ height: `${frameHeight}px`, maxHeight: `${frameHeight}px` }}
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
					{settingsError ? (
						<div className="settings-banner settings-banner-error">{settingsError}</div>
					) : null}
					<nav aria-label="设置分组" className="settings-nav">
						{settingsSections.map((section) => (
							<button
								className={`settings-nav-button ${activeSection === section.id ? "settings-nav-button-active" : ""}`}
								key={section.id}
								onClick={() => setActiveSection(section.id)}
								type="button"
							>
								{section.label}
							</button>
						))}
					</nav>

					<section className="settings-section" hidden={activeSection !== "general"}>
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

						<div className="settings-editor-card settings-editor-card-subtle">
							<div className="settings-editor-card-header">
								<div className="settings-acp-detail-copy">
									<span className="settings-section-kicker">OCR</span>
									<h3 className="settings-subsection-title">截图识别</h3>
									<span className="settings-help-text settings-help-text-tight">
										OCR 配置移到了通用页。截图流程仍只在 macOS 可用；大模型 OCR 仍然要求 OpenAI Chat
										协议并显式启用多模态。
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
														? (current.llmProviderId ??
															llmSettings.defaultProviderId ??
															llmSettings.providers[0]?.id ??
															null)
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
										<span>LLM Provider</span>
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
										<option value="">选择一个支持多模态的 provider</option>
										{eligibleOcrProviders.map((provider) => (
											<option key={provider.id} value={provider.id}>
												{provider.name || provider.model || provider.baseUrl}
												{` · ${summarizeLlmProviderProfile(provider)}`}
											</option>
										))}
									</select>
									<span className="settings-help-text">
										OCR 只接受协议为 OpenAI Chat 且显式声明支持多模态的 provider；实际调用的是
										`chat/completions`。
									</span>
									{eligibleOcrProviders.length === 0 ? (
										<span className="settings-help-text settings-help-text-tight">
											当前没有可用的 OCR provider。先在 LLM 页面把某个 provider 设为 OpenAI
											Chat，并开启多模态。
										</span>
									) : null}
								</div>
							) : null}
						</div>
					</section>

					<section className="settings-section" hidden={activeSection !== "shortcuts"}>
						<h2 className="settings-section-title">快捷键</h2>
						<div className="settings-item">
							<label className="settings-label">
								<span>打开启动器</span>
								<input
									className={`settings-input ${isRecording("toggle_launcher") ? "recording" : ""}`}
									onClick={() => handleShortcutClick("toggle_launcher")}
									placeholder={
										isRecording("toggle_launcher")
											? "按下快捷键..."
											: savingShortcutKey === "toggle_launcher"
												? "保存中..."
												: "点击设置"
									}
									readOnly
									type="text"
									value={shortcutSettings.toggle_launcher}
								/>
							</label>
						</div>
						<div className="settings-item">
							<label className="settings-label">
								<span>截图 OCR</span>
								<input
									className={`settings-input ${isRecording("ocr_capture") ? "recording" : ""}`}
									onClick={() => handleShortcutClick("ocr_capture")}
									placeholder={
										isRecording("ocr_capture")
											? "按下快捷键..."
											: savingShortcutKey === "ocr_capture"
												? "保存中..."
												: "点击设置"
									}
									readOnly
									type="text"
									value={shortcutSettings.ocr_capture}
								/>
							</label>
						</div>
					</section>

					<section className="settings-section" hidden={activeSection !== "appearance"}>
						<h2 className="settings-section-title">外观</h2>
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
					</section>

					<section className="settings-section" hidden={activeSection !== "llm"}>
						<h2 className="settings-section-title">LLM</h2>
						<div className="settings-acp-form-layout">
							<aside className="settings-acp-sidebar settings-acp-sidebar-secondary">
								<div className="settings-acp-sidebar-header">
									<div className="settings-acp-sidebar-copy">
										<span className="settings-section-kicker">已配置</span>
										<h3 className="settings-subsection-title">LLM Provider</h3>
										<span className="settings-help-text settings-help-text-tight">
											先填连接信息，再选模型，最后声明协议和多模态。Base URL 需要指向 API
											根路径，通常包含 <span className="settings-inline-code">/v1</span>； 填完 Base
											URL 和 API key 后可以通过
											<span className="settings-inline-code">/models</span> 拉模型列表。
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
									<div className="settings-agent-list settings-agent-list-master">
										{llmSettings.providers.map((provider) => {
											const issueCount = llmValidation.providerIssues[provider.id]?.length ?? 0;
											const isSelected = provider.id === selectedLlmProviderId;
											return (
												<div
													className={`settings-agent-list-item ${isSelected ? "settings-agent-list-item-selected" : ""} ${issueCount > 0 ? "settings-agent-list-item-invalid" : ""}`}
													key={provider.id}
												>
													<button
														className="settings-agent-list-item-main"
														onClick={() => setSelectedLlmProviderId(provider.id)}
														type="button"
													>
														<div className="settings-agent-title-row">
															<strong className="settings-agent-name">
																{provider.name.trim() || "未命名 Provider"}
															</strong>
															{llmSettings.defaultProviderId === provider.id ? (
																<span className="settings-status-chip settings-status-chip-strong">
																	默认
																</span>
															) : null}
														</div>
														<span className="settings-agent-command-preview">
															{provider.model.trim() || "未设置模型"}
														</span>
														<span className="settings-agent-meta">
															{summarizeLlmProviderProfile(provider)} ·{" "}
															{provider.baseUrl.trim() || "未设置 Base URL"}
														</span>
													</button>
												</div>
											);
										})}
									</div>
								) : (
									<div className="settings-empty-panel settings-empty-panel-subtle">
										<strong className="settings-empty-title">还没有 LLM provider</strong>
										<span className="settings-help-text settings-help-text-tight">
											先添加至少一个 provider。OCR 只会引用协议为 OpenAI Chat
											且显式声明支持多模态的模型。
										</span>
									</div>
								)}
							</aside>

							<div className="settings-acp-detail">
								{selectedLlmProvider ? (
									<>
										<div className="settings-acp-detail-header">
											<div className="settings-acp-detail-copy">
												<span className="settings-section-kicker">编辑</span>
												<h3 className="settings-subsection-title">
													{selectedLlmProvider.name.trim() || "LLM Provider"}
												</h3>
												<span className="settings-help-text settings-help-text-tight">
													当前编辑顺序是连接信息、模型、协议和能力声明。只有 OpenAI Chat 协议能被
													OCR 复用；Responses 和 Embedding provider 不会被 OCR 当成可识别模型。
												</span>
											</div>
											<button
												className="settings-button settings-agent-remove-inline"
												onClick={() => handleRemoveLlmProvider(selectedLlmProvider.id)}
												type="button"
											>
												删除
											</button>
										</div>

										<div className="settings-agent-fields">
											<div className="settings-item settings-item-stacked">
												<label
													className="settings-label settings-label-stacked"
													htmlFor="llm-provider-name"
												>
													<span>名称</span>
												</label>
												<input
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
											</div>

											<div className="settings-item settings-item-stacked">
												<label
													className="settings-label settings-label-stacked"
													htmlFor="llm-provider-base-url"
												>
													<span>Base URL</span>
												</label>
												<input
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
												<span className="settings-help-text settings-help-text-tight">
													先填写当前 provider 的鉴权信息，再用下方按钮请求
													<span className="settings-inline-code">/models</span>
													。如果服务允许匿名访问，这里可以留空。
												</span>
											</div>

											<div className="settings-item settings-item-stacked">
												<label
													className="settings-label settings-label-stacked"
													htmlFor="llm-provider-model"
												>
													<span>模型</span>
												</label>
												{selectedLlmProviderModels.length > 0 ? (
													<select
														className="settings-select"
														disabled={savingLlm}
														id="llm-provider-model-select"
														onChange={(event) =>
															handleLlmModelSelectionChange(
																selectedLlmProvider.id,
																event.target.value,
															)
														}
														value={selectedLlmModelSelectValue}
													>
														<option value="">从模型列表中选择</option>
														{selectedLlmProviderModels.map((model) => (
															<option key={model} value={model}>
																{model}
															</option>
														))}
														<option value={customLlmModelOptionValue}>手动输入其他模型</option>
													</select>
												) : null}
												<input
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
													placeholder="gpt-4.1-mini"
													ref={bindLlmFieldRef(selectedLlmProvider.id, "model")}
													type="text"
													value={selectedLlmProvider.model}
												/>
												<div className="settings-inline-actions">
													<button
														className="settings-button settings-agent-secondary"
														disabled={savingLlm || !selectedLlmProvider.baseUrl.trim()}
														onClick={() => void handleFetchLlmProviderModels(selectedLlmProvider)}
														type="button"
													>
														{isLoadingSelectedLlmProviderModels
															? "拉取中..."
															: selectedLlmProviderModels.length > 0
																? "重新拉取模型列表"
																: "通过 /models 拉取模型列表"}
													</button>
													<span className="settings-help-text settings-help-text-tight">
														{isLoadingSelectedLlmProviderModels
															? "正在从当前 Base URL 拉取模型列表。"
															: selectedLlmProviderModelsError
																? selectedLlmProviderModelsError
																: selectedLlmProviderModels.length > 0
																	? `已拉取 ${selectedLlmProviderModels.length} 个模型。上方下拉可以直接选，下面输入框也允许填写列表里没有的模型。`
																	: "先填写 Base URL 和 API key，再点击按钮请求 /models；如果 provider 不要求鉴权，API key 可以留空。"}
													</span>
												</div>
											</div>

											<div className="settings-item settings-item-stacked">
												<div className="settings-inline-field-row">
													<div className="settings-inline-field">
														<label
															className="settings-label settings-label-stacked"
															htmlFor="llm-provider-protocol"
														>
															<span>协议</span>
														</label>
														<select
															className="settings-select"
															disabled={savingLlm}
															id="llm-provider-protocol"
															onChange={(event) =>
																handleLlmProviderFieldChange(
																	selectedLlmProvider.id,
																	"protocol",
																	event.target.value as LlmProviderProtocolKind,
																)
															}
															value={selectedLlmProvider.protocol}
														>
															{llmProtocolOptions.map((option) => (
																<option key={option.protocol} value={option.protocol}>
																	{option.label}
																</option>
															))}
														</select>
													</div>

													<div className="settings-inline-field">
														<label className="settings-label">
															<span>支持多模态</span>
															<input
																checked={selectedLlmProvider.supportsMultimodal}
																className="settings-toggle"
																disabled={
																	savingLlm || !canProviderSupportMultimodal(selectedLlmProvider)
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
												</div>
												<span className="settings-help-text settings-help-text-tight">
													{
														llmProtocolOptions.find(
															(option) => option.protocol === selectedLlmProvider.protocol,
														)?.summary
													}
												</span>
												{!canProviderSupportMultimodal(selectedLlmProvider) ? (
													<span className="settings-help-text settings-help-text-tight">
														Embedding 协议不支持多模态，这里会固定关闭。
													</span>
												) : null}
											</div>

											<div className="settings-item">
												<label className="settings-label">
													<span>设为默认 Provider</span>
													<input
														checked={llmSettings.defaultProviderId === selectedLlmProvider.id}
														className="settings-toggle"
														disabled={savingLlm}
														onChange={(event) =>
															handleLlmDefaultProviderChange(
																event.target.checked ? selectedLlmProvider.id : null,
															)
														}
														type="checkbox"
													/>
												</label>
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
										<strong className="settings-empty-title">没有可编辑的 Provider</strong>
										<span className="settings-help-text settings-help-text-tight">
											左侧新增一个 provider 后再补全名称、Base URL、模型和能力声明。
										</span>
									</div>
								)}

								<div className="settings-acp-savebar">
									<div className="settings-acp-savebar-copy">
										<span className="settings-section-kicker">保存</span>
										<span className="settings-agent-meta">
											{llmHasUnsavedChanges
												? "当前 LLM 草稿尚未写回配置。"
												: "LLM 配置已同步到本地配置文件。"}
										</span>
									</div>
									<div className="settings-acp-savebar-actions">
										<button
											className="settings-button settings-agent-secondary"
											disabled={!llmHasUnsavedChanges || savingLlm}
											onClick={handleDiscardLlmChanges}
											type="button"
										>
											放弃修改
										</button>
										<button
											className="settings-button"
											disabled={!llmHasUnsavedChanges || savingLlm}
											onClick={() => void handleSaveLlm()}
											type="button"
										>
											{savingLlm ? "保存中..." : "保存 LLM 配置"}
										</button>
									</div>
								</div>
							</div>
						</div>
					</section>

					<section className="settings-section" hidden={activeSection !== "rag"}>
						<h2 className="settings-section-title">RAG</h2>
						<div className="settings-editor-card">
							<div className="settings-editor-card-header">
								<div className="settings-acp-detail-copy">
									<span className="settings-section-kicker">索引</span>
									<h3 className="settings-subsection-title">LanceDB 文档索引</h3>
									<span className="settings-help-text settings-help-text-tight">
										保存后会根据当前配置启动目录监听。初次扫描和后续文件变更都会重新切分文本、 调用
										embedding 模型，并把旧向量替换成新向量。Markdown 类文件会按文档结构切分，其他
										文本文件走通用语义切分。
									</span>
								</div>
							</div>

							<div className="settings-item settings-item-stacked">
								<label
									className="settings-label settings-label-stacked"
									htmlFor="rag-embedding-provider"
								>
									<span>Embedding Provider</span>
								</label>
								<select
									className="settings-select"
									disabled={savingRag || scanningRag}
									id="rag-embedding-provider"
									onChange={(event) =>
										setRagSettings((current) => ({
											...current,
											embeddingProviderId: event.target.value || null,
										}))
									}
									value={ragSettings.embeddingProviderId ?? ""}
								>
									<option value="">选择一个 OpenAI Embedding provider</option>
									{eligibleRagEmbeddingProviders.map((provider) => (
										<option key={provider.id} value={provider.id}>
											{provider.name || provider.model || provider.baseUrl}
											{` · ${provider.model}`}
										</option>
									))}
								</select>
								<span className="settings-help-text settings-help-text-tight">
									RAG 只接受协议为 OpenAI Embedding 的 provider。没有可选项时，先去 LLM 页面新增一个
									embedding provider。
								</span>
								<div className="settings-validation-box settings-banner-warn">
									RAG 建索引时会把切分后的文档内容发送给当前 embedding 模型。涉及隐私或敏感数据时，
									优先选用本机部署的 Ollama，或其它你明确信任的模型服务。
								</div>
							</div>

							<div className="settings-item settings-item-stacked">
								<label className="settings-label settings-label-stacked" htmlFor="rag-source-dirs">
									<span>扫描目录</span>
								</label>
								<textarea
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
									<span className="settings-help-text settings-help-text-tight">
										每行一个目录。保存后会监听这些目录内的文件变化；目录选择器默认从 `~/Documents`
										打开。
									</span>
								</div>
								<span className="settings-help-text settings-help-text-tight">
									当前只会向量化这些后缀：
									{ragSupportedFileExtensions.map((extension) => `.${extension}`).join("、")}
									。文件还必须可读、是 UTF-8 文本、大小不超过 50 MB，且不命中忽略 glob；
									不在名单里的后缀会直接跳过。
								</span>
							</div>

							<div className="settings-item settings-item-stacked">
								<label className="settings-label settings-label-stacked" htmlFor="rag-ignore-globs">
									<span>忽略通配符</span>
								</label>
								<textarea
									className="settings-textarea settings-input settings-input-mono"
									disabled={savingRag || scanningRag}
									id="rag-ignore-globs"
									onChange={(event) =>
										setRagSettings((current) => ({
											...current,
											ignoreGlobs: parseTextLines(event.target.value),
										}))
									}
									placeholder={"**/node_modules/**\n**/*.png\n**/target/**"}
									rows={5}
									value={formatTextLines(ragSettings.ignoreGlobs)}
								/>
								<span className="settings-help-text settings-help-text-tight">
									每行一个 glob。匹配到的文件不会被切分、向量化或写入 LanceDB。
								</span>
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

							{ragScanResult ? (
								<div className="settings-validation-box settings-banner-info">
									<div className="settings-rag-summary-grid">
										<span>{`数据库：${ragScanResult.databasePath}`}</span>
										<span>{`目录数：${ragScanResult.sourceCount}`}</span>
										<span>{`扫描文件：${ragScanResult.scannedFileCount}`}</span>
										<span>{`已索引文件：${ragScanResult.indexedFileCount}`}</span>
										<span>{`跳过文件：${ragScanResult.skippedFileCount}`}</span>
										<span>{`向量块：${ragScanResult.chunkCount}`}</span>
									</div>
								</div>
							) : null}

							<div className="settings-acp-savebar">
								<div className="settings-acp-savebar-copy">
									<span className="settings-section-kicker">保存</span>
									<span className="settings-agent-meta">
										{ragHasUnsavedChanges
											? "当前 RAG 草稿尚未写回配置。"
											: "RAG 配置已同步到本地配置文件。"}
									</span>
								</div>
								<div className="settings-acp-savebar-actions">
									<button
										className="settings-button settings-agent-secondary"
										disabled={!ragHasUnsavedChanges || savingRag || scanningRag}
										onClick={handleDiscardRagChanges}
										type="button"
									>
										放弃修改
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
										disabled={!ragHasUnsavedChanges || savingRag || scanningRag}
										onClick={() => void handleSaveRag()}
										type="button"
									>
										{savingRag ? "保存中..." : "保存 RAG 配置"}
									</button>
								</div>
							</div>
						</div>
					</section>

					<section className="settings-section" hidden={activeSection !== "acp"}>
						<h2 className="settings-section-title">ACP Agent</h2>
						{acpNotice ? (
							<div className={`settings-banner settings-banner-${acpNotice.tone}`}>
								{acpNotice.text}
							</div>
						) : null}

						<div className="settings-acp-form-layout">
							<aside className="settings-acp-sidebar settings-acp-sidebar-secondary">
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
									ACP Agent 就是一条启动命令配置。预设只负责填表；实际创建 Session 用哪个 Agent，看
									launcher 顶部选择。
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

										<div className="settings-editor-card">
											<div className="settings-editor-card-header">
												<strong className="settings-agent-mcp-title">选择 Agent</strong>
												<span className="settings-agent-meta">下拉项只负责填充表单默认值</span>
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
																onChange={(event) => setSelectedPresetOptionId(event.target.value)}
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

										<div className="settings-editor-card">
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
														className="settings-input settings-input-wide"
														onChange={(event) =>
															handleAgentFieldChange(selectedAgent.id, "name", event.target.value)
														}
														placeholder="例如 Codex"
														ref={bindAcpFieldRef("agent", selectedAgent.id, "name")}
														type="text"
														value={selectedAgent.name}
													/>
												</label>
												<label className="settings-label settings-label-stacked">
													<span>启动命令</span>
													<input
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
													<span className="settings-help-text settings-help-text-tight">
														这里只填写 Agent 启动命令；所有 Agent 仍共用同一份全局 MCP 配置。
													</span>
												</label>
											</div>
										</div>
									</>
								) : (
									<>
										<div className="settings-editor-card">
											<div className="settings-editor-card-header">
												<strong className="settings-agent-mcp-title">选择 Agent</strong>
												<span className="settings-agent-meta">下拉项只负责填充表单默认值</span>
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
																onChange={(event) => setSelectedPresetOptionId(event.target.value)}
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
										<div className="settings-empty-panel">
											<strong className="settings-empty-title">先创建一个 ACP Agent 草稿</strong>
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

								<div className="settings-acp-savebar">
									<div className="settings-acp-savebar-copy">
										<strong className="settings-agent-mcp-title">
											{acpValidation.totalIssues > 0
												? `先修复 ${acpValidation.totalIssues} 个问题`
												: acpHasUnsavedChanges
													? "有未保存的 ACP Agent 草稿"
													: "ACP Agent 配置已与本地 config.toml 同步"}
										</strong>
										<span className="settings-help-text settings-help-text-tight">
											这里保存的是 ACP Agent 表单，不会连带修改全局 MCP。
										</span>
									</div>
									<div className="settings-acp-savebar-actions">
										<button
											className="settings-agent-secondary"
											disabled={savingAgent || !acpHasUnsavedChanges}
											onClick={handleDiscardAcpChanges}
											type="button"
										>
											放弃变更
										</button>
										{acpValidation.totalIssues > 0 ? (
											<button
												className="settings-button"
												onClick={locateFirstAcpIssue}
												type="button"
											>
												定位问题
											</button>
										) : (
											<button
												className="settings-button"
												disabled={savingAgent || !acpHasUnsavedChanges}
												onClick={() => void handleSaveAgent()}
												type="button"
											>
												{savingAgent
													? "保存中..."
													: acpHasUnsavedChanges
														? "保存 ACP Agent"
														: "已保存"}
											</button>
										)}
									</div>
								</div>
							</div>
						</div>
					</section>

					<section className="settings-section" hidden={activeSection !== "mcp"}>
						<h2 className="settings-section-title">MCP</h2>
						{mcpNotice ? (
							<div className={`settings-banner settings-banner-${mcpNotice.tone}`}>
								{mcpNotice.text}
							</div>
						) : null}

						<div className="settings-acp-form-layout">
							<aside className="settings-acp-sidebar settings-acp-sidebar-secondary">
								<div className="settings-acp-sidebar-header">
									<div className="settings-acp-sidebar-copy">
										<span className="settings-section-kicker">已配置</span>
									</div>
									<button
										className="settings-button settings-button-compact"
										onClick={() => handleAddMcpServer(selectedMcpTransport)}
										type="button"
									>
										+ 新建
									</button>
								</div>
								<p className="settings-help-text settings-help-text-tight">
									MCP 是给所有 ACP Agent 共用的一组连接配置。
								</p>

								{mcpServers.length === 0 ? (
									<div className="settings-empty-panel">
										<strong className="settings-empty-title">还没有 MCP server</strong>
										<span className="settings-help-text settings-help-text-tight">
											先新建一个，再填写连接参数。
										</span>
									</div>
								) : (
									<div className="settings-mcp-list settings-mcp-list-master settings-mcp-list-compact">
										{mcpServers.map((server) => {
											const transportMeta = getMcpTransportMeta(server.transport);
											const issueCount = mcpValidation.serverIssues[server.id]?.length ?? 0;
											const isSelected = selectedMcpServerId === server.id;
											return (
												<article
													className={`settings-mcp-list-item ${isSelected ? "settings-mcp-list-item-selected" : ""} ${
														issueCount > 0 ? "settings-mcp-list-item-invalid" : ""
													}`}
													key={server.id}
												>
													<button
														className="settings-mcp-list-item-main"
														onClick={() => setSelectedMcpServerId(server.id)}
														type="button"
													>
														<div className="settings-agent-title-row">
															<span className="settings-mcp-badge">{transportMeta.label}</span>
															<strong className="settings-agent-name">
																{server.name.trim() || "未命名 MCP Server"}
															</strong>
															<span className="settings-agent-meta">{issueCount} 个问题</span>
														</div>
														<span className="settings-agent-command-preview">
															{summarizeMcpServerDraft(server)}
														</span>
													</button>
												</article>
											);
										})}
									</div>
								)}
							</aside>

							<div className="settings-acp-detail">
								<div className="settings-editor-card">
									<div className="settings-editor-card-header">
										<strong className="settings-agent-mcp-title">快速新建</strong>
										<span className="settings-agent-meta">下拉项只负责创建空白表单</span>
									</div>
									<div className="settings-acp-preset-layout">
										<div className="settings-acp-preset-controls">
											<div className="settings-acp-preset-panel">
												<span className="settings-acp-preset-kicker">快速填充</span>
												<label className="settings-label settings-label-stacked">
													<span className="settings-acp-preset-label">连接类型</span>
													<select
														className="settings-select"
														onChange={(event) =>
															setSelectedMcpTransport(event.target.value as McpTransport)
														}
														value={selectedMcpTransport}
													>
														{mcpTransportOptions.map((option) => (
															<option key={option.transport} value={option.transport}>
																{option.label} · {option.description}
															</option>
														))}
													</select>
												</label>
												<span className="settings-help-text settings-help-text-tight">
													只会创建当前 transport 的空白 MCP 表单，不会直接保存。
												</span>
												<button
													className="settings-button settings-button-compact settings-acp-preset-action"
													onClick={handleApplyMcpTransportSelection}
													type="button"
												>
													新建一个
												</button>
											</div>
										</div>
										<div className="settings-install-guide settings-install-guide-card">
											{renderMcpTransportGuide(selectedMcpTransport)}
										</div>
									</div>
								</div>

								{selectedMcpServer ? (
									<>
										<div className="settings-acp-detail-header">
											<div className="settings-acp-detail-copy">
												<span className="settings-section-kicker">当前表单</span>
												<h3 className="settings-subsection-title">
													{selectedMcpServer.name.trim() || "未命名 MCP Server"}
												</h3>
											</div>
											<button
												className="settings-agent-remove settings-agent-remove-inline"
												onClick={() => handleRemoveMcpServer(selectedMcpServer.id)}
												type="button"
											>
												删除 MCP
											</button>
										</div>

										{(mcpValidation.serverIssues[selectedMcpServer.id]?.length ?? 0) > 0 ? (
											<ul className="settings-issue-list">
												{(mcpValidation.serverIssues[selectedMcpServer.id] ?? []).map((issue) => (
													<li key={issue}>{issue}</li>
												))}
											</ul>
										) : null}

										<div className="settings-editor-card">
											<div className="settings-editor-card-header">
												<strong className="settings-agent-mcp-title">基础信息</strong>
												<span className="settings-agent-meta">
													{selectedMcpIssueCount > 0
														? `${selectedMcpIssueCount} 个待处理问题`
														: "基础信息完整"}
												</span>
											</div>

											<div className="settings-agent-fields">
												<label className="settings-label settings-label-stacked">
													<span>名称</span>
													<input
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
													<span className="settings-help-text settings-help-text-tight">
														{getMcpTransportMeta(selectedMcpServer.transport).description}
													</span>
												</label>
											</div>
										</div>

										<div className="settings-editor-card">
											<div className="settings-editor-card-header">
												<strong className="settings-agent-mcp-title">连接配置</strong>
												<span className="settings-agent-meta">
													{getMcpTransportMeta(selectedMcpServer.transport).label}
												</span>
											</div>
											<div className="settings-agent-fields">
												{selectedMcpServer.transport === "stdio" ? (
													<>
														<label className="settings-label settings-label-stacked">
															<span>Command</span>
															<input
																className="settings-input settings-input-wide settings-input-mono"
																onChange={(event) =>
																	handleMcpServerFieldChange(
																		selectedMcpServer.id,
																		"command",
																		event.target.value,
																	)
																}
																placeholder="例如 npx"
																ref={bindAcpFieldRef("server", selectedMcpServer.id, "command")}
																type="text"
																value={selectedMcpServer.command}
															/>
														</label>
														<label className="settings-label settings-label-stacked">
															<span>Args</span>
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
															<span>Env</span>
															<textarea
																className="settings-textarea settings-input-mono"
																onChange={(event) =>
																	handleMcpServerFieldChange(
																		selectedMcpServer.id,
																		"envText",
																		event.target.value,
																	)
																}
																placeholder="每行一个 KEY=VALUE"
																ref={bindAcpFieldRef("server", selectedMcpServer.id, "envText")}
																rows={3}
																value={selectedMcpServer.envText}
															/>
														</label>
													</>
												) : (
													<>
														<label className="settings-label settings-label-stacked">
															<span>URL</span>
															<input
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
														</label>
														<label className="settings-label settings-label-stacked">
															<span>Headers</span>
															<textarea
																className="settings-textarea settings-input-mono"
																onChange={(event) =>
																	handleMcpServerFieldChange(
																		selectedMcpServer.id,
																		"headersText",
																		event.target.value,
																	)
																}
																placeholder="每行一个 KEY=VALUE"
																ref={bindAcpFieldRef("server", selectedMcpServer.id, "headersText")}
																rows={3}
																value={selectedMcpServer.headersText}
															/>
														</label>
													</>
												)}
											</div>
										</div>
									</>
								) : (
									<div className="settings-empty-panel">
										<strong className="settings-empty-title">先创建一个 MCP 草稿</strong>
										<span className="settings-help-text settings-help-text-tight">
											先选择一种连接类型填入空白表单，再补全名称和连接参数。
										</span>
										<div className="settings-acp-empty-actions">
											<button
												className="settings-button settings-button-compact"
												onClick={handleApplyMcpTransportSelection}
												type="button"
											>
												+ 新建 {getMcpTransportMeta(selectedMcpTransport).label}
											</button>
										</div>
									</div>
								)}

								<div className="settings-acp-savebar">
									<div className="settings-acp-savebar-copy">
										<strong className="settings-agent-mcp-title">
											{mcpValidation.totalIssues > 0
												? `先修复 ${mcpValidation.totalIssues} 个问题`
												: mcpHasUnsavedChanges
													? "有未保存的 MCP 草稿"
													: "MCP 配置已与本地 config.toml 同步"}
										</strong>
										<span className="settings-help-text settings-help-text-tight">
											这里保存的是全局 MCP 清单，所有 ACP Agent 共用。
										</span>
									</div>
									<div className="settings-acp-savebar-actions">
										<button
											className="settings-agent-secondary"
											disabled={savingMcp || !mcpHasUnsavedChanges}
											onClick={handleDiscardMcpChanges}
											type="button"
										>
											放弃变更
										</button>
										{mcpValidation.totalIssues > 0 ? (
											<button
												className="settings-button"
												onClick={locateFirstMcpIssue}
												type="button"
											>
												定位问题
											</button>
										) : (
											<button
												className="settings-button"
												disabled={savingMcp || !mcpHasUnsavedChanges}
												onClick={() => void handleSaveMcp()}
												type="button"
											>
												{savingMcp
													? "保存中..."
													: mcpHasUnsavedChanges
														? "保存 MCP 配置"
														: "已保存"}
											</button>
										)}
									</div>
								</div>
							</div>
						</div>
					</section>

					<section className="settings-section" hidden={activeSection !== "skills"}>
						<h2 className="settings-section-title">Skills</h2>
						{skillError ? (
							<div className="settings-banner settings-banner-error">{skillError}</div>
						) : null}

						<div className="settings-acp-form-layout">
							<aside className="settings-acp-sidebar settings-acp-sidebar-secondary">
								<div className="settings-acp-sidebar-header">
									<div className="settings-acp-sidebar-copy">
										<span className="settings-section-kicker">公共目录</span>
										<strong className="settings-agent-mcp-title">
											{skillCatalog.skills.length} 个 skill
										</strong>
									</div>
								</div>
								<p className="settings-help-text settings-help-text-tight">
									当前只读扫描 <code className="settings-inline-code">{skillCatalog.rootPath}</code>
									，不改写 skill 文件。
								</p>

								{!skillCatalog.exists ? (
									<div className="settings-empty-panel">
										<strong className="settings-empty-title">目录不存在</strong>
										<span className="settings-help-text settings-help-text-tight">
											没找到公共 skill 目录。当前只读取 `~/.agents/skills`。
										</span>
									</div>
								) : skillCatalog.skills.length === 0 ? (
									<div className="settings-empty-panel">
										<strong className="settings-empty-title">没有公共 skill</strong>
										<span className="settings-help-text settings-help-text-tight">
											目录存在，但下面没有可展示的 skill 子目录。
										</span>
									</div>
								) : (
									<div className="settings-agent-list settings-agent-list-master settings-agent-list-compact">
										{skillCatalog.skills.map((skill) => {
											const isSelected = selectedSkillId === skill.id;
											const skillTitle = skill.meta.name?.trim() || skill.directoryName;
											return (
												<article
													className={`settings-agent-list-item ${isSelected ? "settings-agent-list-item-selected" : ""}`}
													key={skill.id}
												>
													<button
														className="settings-agent-list-item-main"
														onClick={() => setSelectedSkillId(skill.id)}
														type="button"
													>
														<div className="settings-agent-title-row">
															<strong className="settings-agent-name">{skillTitle}</strong>
															<span className="settings-agent-meta">
																{skill.directoryCount} 目录
															</span>
														</div>
														<span className="settings-agent-command-preview">
															{skill.relativePath}
														</span>
														<span className="settings-agent-meta">{skill.fileCount} 文件</span>
													</button>
												</article>
											);
										})}
									</div>
								)}
							</aside>

							<div className="settings-acp-detail">
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
													{selectedSkill.directoryCount} 个目录 · {selectedSkill.fileCount} 个文件
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
											先从左侧选择一个公共 skill，右侧才会显示 meta 和目录树。
										</span>
									</div>
								)}
							</div>
						</div>
					</section>

					<section className="settings-section" hidden={activeSection !== "about"}>
						<h2 className="settings-section-title">关于</h2>
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
					</section>
				</div>
			</section>
		</main>
	);
}
