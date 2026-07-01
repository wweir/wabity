import type { ReactNode } from "react";
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
	{ id: "prompts", label: "功能" },
	{ id: "llm", label: "模型" },
	{ id: "rag", label: "知识库" },
	{ id: "mcp", label: "Agent 配置" },
	{ id: "about", label: "关于" },
] as const;

export const settingsQuickLinks: Readonly<Record<SettingsSectionId, readonly SettingsQuickLink[]>> =
	{
		general: [],
		prompts: [
			{ id: "prompts-translation", label: "翻译配置", hint: "模型与提示词" },
			{ id: "prompts-rag-answer", label: "文档问答", hint: "模型与提示词" },
			{ id: "prompts-ocr", label: "截图识别", hint: "OCR 模型" },
		],
		llm: [],
		rag: [],
		mcp: [
			{ id: "extensions-agent", label: "Agent", hint: "会话配置" },
			{ id: "mcp-builtin", label: "内置工具", hint: "供 Agent 使用" },
			{ id: "mcp-catalog", label: "MCP 服务", hint: "Agent 连接" },
		],
		about: [{ id: "about-overview", label: "关于 Wabity", hint: "版本与项目" }],
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

const mcpTransportOptionsInternal = [
	{
		transport: "stdio",
		label: "本地进程",
		description: "通过命令启动 MCP 服务",
		summary: "适合本机已有命令行 MCP 服务的场景。Wabity 只保存连接参数，供 Agent 配置使用。",
		fieldsHint: "需要填写命令；可选填写参数和环境变量。",
		example: "npx -y @modelcontextprotocol/server-filesystem ~/Desktop",
	},
	{
		transport: "http",
		label: "HTTP",
		description: "通过服务地址连接远程 MCP 服务",
		summary: "适合已经部署好的远程 MCP 服务。Wabity 不再把它注入文档问答链路。",
		fieldsHint: "需要填写服务地址；可选填写请求头。",
		example: "https://example.com/mcp",
	},
	{
		transport: "sse",
		label: "SSE",
		description: "通过 SSE 流连接远程 MCP 服务",
		summary: "适合使用服务端事件流暴露能力的远程 MCP 服务，字段和 HTTP 类似。",
		fieldsHint: "需要填写服务地址；可选填写请求头。",
		example: "https://example.com/sse",
	},
] as const;

export const mcpTransportOptions = mcpTransportOptionsInternal;

export function getMcpTransportMeta(transport: McpTransport) {
	return (
		mcpTransportOptionsInternal.find((option) => option.transport === transport) ??
		mcpTransportOptionsInternal[0]
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
		return "等待填写服务地址";
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
