import type { ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AcpAgentLaunchMode } from "../../lib/tauri/types";
import type {
	AcpMcpServerDraft,
	McpTransport,
	SettingsQuickLink,
	SettingsSectionId,
} from "./settingsTypes";

export const settingsSections: ReadonlyArray<{
	id: SettingsSectionId;
	label: string;
}> = [
	{ id: "general", label: "通用" },
	{ id: "prompts", label: "AI 功能" },
	{ id: "llm", label: "模型接入" },
	{ id: "rag", label: "RAG" },
	{ id: "acp", label: "ACP Agent" },
	{ id: "mcp", label: "MCP" },
	{ id: "skills", label: "Skill" },
	{ id: "about", label: "关于" },
] as const;

export const settingsQuickLinks: Readonly<Record<SettingsSectionId, readonly SettingsQuickLink[]>> =
	{
		general: [],
		prompts: [
			{ id: "prompts-translation", label: "翻译配置", hint: "模型与提示词" },
			{ id: "prompts-rag-answer", label: "文档问答", hint: "模型与提示词" },
		],
		llm: [
			{ id: "llm-catalog", label: "条目列表", hint: "已配置条目" },
			{ id: "llm-editor", label: "编辑区", hint: "连接信息与能力" },
		],
		rag: [],
		acp: [
			{ id: "acp-catalog", label: "Agent 列表", hint: "已配置条目" },
			{ id: "acp-presets", label: "模板", hint: "预设与安装提示" },
			{ id: "acp-form", label: "基础信息", hint: "名称与命令" },
		],
		mcp: [],
		skills: [],
		about: [{ id: "about-overview", label: "关于 Wabity", hint: "版本与定位" }],
	} as const;

function joinClassNames(...classNames: Array<string | false | null | undefined>): string {
	return classNames.filter((className): className is string => Boolean(className)).join(" ");
}

function getShortcutRecorderStatusText(isRecording: boolean, isSaving: boolean): string {
	if (isRecording) {
		return "正在录制，直接按下目标快捷键，按 Escape 取消。";
	}

	if (isSaving) {
		return "正在保存快捷键。";
	}

	return "";
}

function getShortcutRecorderActionLabel(
	isRecording: boolean,
	isSaving: boolean,
	shortcutValue: string,
): string {
	if (isRecording) {
		return "正在录制";
	}

	if (isSaving) {
		return "保存中";
	}

	return shortcutValue.trim() ? "重新录制" : "开始录制";
}

function getPresetInstallLinkSeparator(index: number, total: number): string {
	if (index === 0) {
		return " ";
	}

	return index === total - 1 ? " 和 " : "、";
}

export function getSettingsTabId(sectionId: SettingsSectionId): string {
	return `settings-tab-${sectionId}`;
}

export function getSettingsPanelId(sectionId: SettingsSectionId): string {
	return `settings-panel-${sectionId}`;
}

export function joinDescribedByIds(
	...ids: Array<string | null | undefined | false>
): string | undefined {
	const joinedIds = ids.filter((id): id is string => Boolean(id)).join(" ");
	return joinedIds || undefined;
}

export function buildFieldIssueId(sectionId: string, fieldKey: string, itemId?: string): string {
	return itemId
		? `settings-${sectionId}-${itemId}-${fieldKey}-error`
		: `settings-${sectionId}-${fieldKey}-error`;
}

export const DISCARD_DRAFT_BUTTON_LABEL = "恢复已保存版本";

export function SettingsDraftActionCard({
	title,
	description,
	actions,
	className,
}: {
	title: string;
	description: string;
	actions?: ReactNode;
	className?: string;
}) {
	return (
		<div
			className={joinClassNames(
				"settings-editor-card",
				"settings-editor-card-subtle",
				"settings-draft-action-card",
				className,
			)}
		>
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

export function SettingsQuickJumpList({
	activeBlockId,
	compact = false,
	links,
	onSelect,
}: {
	activeBlockId: string | null;
	compact?: boolean;
	links: readonly SettingsQuickLink[];
	onSelect: (blockId: string) => void;
}) {
	if (links.length === 0 || (compact && links.length === 1)) {
		return null;
	}

	return (
		<div className={joinClassNames("settings-jump-list", compact && "settings-jump-list-compact")}>
			{links.map((link) => {
				const isActive = activeBlockId === link.id;
				return (
					<button
						aria-current={isActive ? "location" : undefined}
						className={joinClassNames(
							"settings-jump-button",
							compact && "settings-jump-button-compact",
							isActive && "settings-jump-button-active",
						)}
						key={link.id}
						onClick={() => onSelect(link.id)}
						type="button"
					>
						<span className="settings-jump-button-label">{link.label}</span>
						<span
							className={joinClassNames(
								"settings-jump-button-meta",
								compact && "settings-jump-button-meta-compact",
							)}
						>
							{link.hint}
						</span>
					</button>
				);
			})}
		</div>
	);
}

export function ShortcutRecorderField({
	instructionsId,
	isRecording,
	isSaving,
	label,
	onActivate,
	shortcutValue,
	statusId,
	triggerId,
}: {
	instructionsId: string;
	isRecording: boolean;
	isSaving: boolean;
	label: string;
	onActivate: () => void;
	shortcutValue: string;
	statusId: string;
	triggerId: string;
}) {
	const statusText = getShortcutRecorderStatusText(isRecording, isSaving);
	const actionLabel = getShortcutRecorderActionLabel(isRecording, isSaving, shortcutValue);
	const describedBy = joinDescribedByIds(instructionsId, statusId);
	const labelId = `${triggerId}-label`;
	const valueId = `${triggerId}-value`;
	const actionId = `${triggerId}-action`;
	const labelledBy = joinDescribedByIds(labelId, valueId, actionId);

	return (
		<div className="settings-item settings-shortcut-item">
			<div className="settings-shortcut-field">
				<div className="settings-shortcut-copy">
					<span className="settings-shortcut-label" id={labelId}>
						{label}
					</span>
					<span
						aria-live="polite"
						className={joinClassNames(
							"settings-help-text",
							"settings-help-text-tight",
							!statusText && "sr-only",
						)}
						id={statusId}
						role="status"
					>
						{statusText}
					</span>
				</div>
				<button
					aria-describedby={describedBy}
					aria-labelledby={labelledBy}
					aria-pressed={isRecording}
					className={joinClassNames(
						"settings-input",
						"settings-shortcut-trigger",
						isRecording && "recording",
					)}
					disabled={isSaving}
					id={triggerId}
					onClick={onActivate}
					type="button"
				>
					<span className="settings-shortcut-trigger-value" id={valueId}>
						{shortcutValue.trim() || "未设置"}
					</span>
					<span className="settings-shortcut-trigger-action" id={actionId}>
						{actionLabel}
					</span>
				</button>
			</div>
		</div>
	);
}

export const acpAgentOptions = [
	{
		id: "opencode",
		label: "OpenCode",
		command: "opencode acp",
		launchMode: "login_shell" as const,
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
		launchMode: "login_shell" as const,
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
		launchMode: "login_shell" as const,
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

export type AcpAgentOption = (typeof acpAgentOptions)[number];

const mcpTransportOptionsInternal = [
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
] as const;

const acpAgentLaunchModeOptionsInternal: ReadonlyArray<{
	value: AcpAgentLaunchMode;
	label: string;
	description: string;
}> = [
	{
		value: "direct",
		label: "直接执行",
		description: "直接把命令拆成 program + args 执行，不读取 shell 配置。",
	},
	{
		value: "login_shell",
		label: "Login Shell",
		description:
			"通过用户默认 shell 的 login 模式启动，读取 login profile，不保证读取 .zshrc / .bashrc。",
	},
	{
		value: "interactive_shell",
		label: "Interactive Shell",
		description:
			"通过用户默认 shell 的 login + interactive 模式启动，更可能读取 .zshrc / .bashrc，但任何输出都可能污染 ACP stdio。",
	},
] as const;

function isWindowsPlatform() {
	return typeof navigator !== "undefined" && /Windows/iu.test(navigator.userAgent);
}

export const acpAgentLaunchModeOptions = isWindowsPlatform()
	? acpAgentLaunchModeOptionsInternal.filter((option) => option.value !== "interactive_shell")
	: acpAgentLaunchModeOptionsInternal;

export const mcpTransportOptions = mcpTransportOptionsInternal;

export function getMcpTransportMeta(transport: McpTransport) {
	return (
		mcpTransportOptionsInternal.find((option) => option.transport === transport) ??
		mcpTransportOptionsInternal[0]
	);
}

export function formatAcpAgentOptionLabel(option: AcpAgentOption, alreadyAdded: boolean) {
	return `${option.label} · ${option.command} · ${getAcpAgentLaunchModeMeta(option.launchMode).label}${alreadyAdded ? " · 已配置" : ""}`;
}

export function getAcpAgentLaunchModeMeta(launchMode: AcpAgentLaunchMode) {
	if (isWindowsPlatform() && launchMode === "interactive_shell") {
		return {
			value: "login_shell" as const,
			label: "Login Shell",
			description:
				"Windows 默认命令处理器不区分 login / interactive shell，当前会按 Login Shell 执行。",
		};
	}

	return (
		acpAgentLaunchModeOptions.find((option) => option.value === launchMode) ??
		acpAgentLaunchModeOptions[1] ??
		acpAgentLaunchModeOptionsInternal[0]
	);
}

export function renderPresetInstallGuide(
	selectedPresetInstallOption: AcpAgentOption | null,
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
							{getPresetInstallLinkSeparator(index, selectedPresetInstallOption.links.length)}
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
					自己准备一个能在命令行里启动的 ACP Agent
					命令，然后填进下面的启动命令输入框，并选择合适的启动模式。
				</span>
			</>
		);
	}

	return (
		<>
			<span className="settings-acp-preset-kicker">安装指引</span>
			<span className="settings-help-text settings-help-text-tight">
				先从左侧选择一个预设，右侧会展示对应的安装命令和官方入口。
			</span>
		</>
	);
}

export function renderMcpTransportGuide(selectedTransport: McpTransport) {
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

function parseMcpRemoteUrl(url: string): URL | null {
	try {
		return new URL(url.trim());
	} catch {
		return null;
	}
}

export function getMcpServerDraftTitle(server: AcpMcpServerDraft) {
	return server.name.trim() || "未命名服务";
}

function summarizeMcpRemoteUrl(url: string): string {
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

export function summarizeMcpServerDraft(server: AcpMcpServerDraft) {
	if (server.transport === "stdio") {
		return server.command.trim() || "等待填写命令";
	}

	return summarizeMcpRemoteUrl(server.url);
}
