import type { Dispatch, ReactElement, SetStateAction } from "react";
import { getSettingsPanelId, getSettingsTabId } from "../settingsShared";
import type { BindSectionBlockRef } from "../sectionViewShared";
import type {
	AcpAgentDraft,
	AcpDraftValidation,
	AcpFieldKey,
	AcpInlineNotice,
	FieldIssueMap,
} from "../settingsTypes";

export interface AcpSettingsSectionProps {
	bindSectionBlockRef: BindSectionBlockRef;
	acpNotice: AcpInlineNotice | null;
	acpAgents: AcpAgentDraft[];
	acpValidation: AcpDraftValidation;
	selectedAgentId: string | null;
	selectedAgent: AcpAgentDraft | null;
	selectedAgentIssueCount: number;
	selectedAgentFieldIssues: FieldIssueMap<AcpFieldKey>;
	acpHasUnsavedChanges: boolean;
	savingAgent: boolean;
	selectedPresetOptionId: string;
	setSelectedPresetOptionId: Dispatch<SetStateAction<string>>;
	bindAcpFieldRef: unknown;
	setSelectedAgentId: Dispatch<SetStateAction<string | null>>;
	onAddCustomAgent: () => void;
	onLocateFirstAcpIssue: () => void;
	onDiscardAcpDraft: () => void;
	onSaveAgent: () => Promise<void>;
	onRemoveAgent: (agentId: string) => void;
	onApplyPresetSelection: () => void;
	onAgentFieldChange: (agentId: string, key: AcpFieldKey, value: string) => void;
}

export function AcpSettingsSection({
	acpAgents,
	acpNotice,
	bindSectionBlockRef,
}: AcpSettingsSectionProps): ReactElement {
	const legacyAgentCount = acpAgents.length;

	return (
		<section
			aria-labelledby={getSettingsTabId("acp")}
			className="settings-section"
			id={getSettingsPanelId("acp")}
			role="tabpanel"
		>
			<div className="settings-section-header">
				<div>
					<span className="settings-section-kicker">Pi Agent</span>
					<h2>Pi Agent</h2>
					<p>Wabity 现在只支持内嵌 Pi SDK session，不再启动外部 Pi Agent / ACP agent 命令。</p>
				</div>
			</div>

			<div className="settings-editor-card" id="acp-form" ref={bindSectionBlockRef("acp-form")}>
				<div className="settings-editor-card-header">
					<div className="settings-acp-detail-copy">
						<span className="settings-section-kicker">运行时</span>
						<strong className="settings-agent-mcp-title">Pi SDK 单运行时</strong>
						<span className="settings-help-text settings-help-text-tight">
							Agent session 由 Rust 后端通过 <code>pi::sdk</code> 创建。第一阶段沿用 Pi 自身
							provider / model / tools 配置；Wabity 不再保存外部 agent 启动命令。
						</span>
					</div>
				</div>
				<div className="settings-info-list">
					<div className="settings-info-row">
						<span>外部命令</span>
						<strong>已移除</strong>
					</div>
					<div className="settings-info-row">
						<span>运行方式</span>
						<strong>内嵌 Pi SDK</strong>
					</div>
					<div className="settings-info-row">
						<span>全局 MCP</span>
						<strong>暂不自动注入 Pi Agent session</strong>
					</div>
				</div>
			</div>

			{legacyAgentCount > 0 || acpNotice ? (
				<div className="settings-editor-card settings-editor-card-subtle">
					<div className="settings-editor-card-header">
						<div className="settings-acp-detail-copy">
							<span className="settings-section-kicker">迁移提示</span>
							<strong className="settings-agent-mcp-title">旧 ACP 配置已停用</strong>
							<span className="settings-help-text settings-help-text-tight">
								检测到 {legacyAgentCount} 条旧外部 agent 配置。它们不会再用于创建
								session；请直接使用 Pi Agent 入口创建新的内嵌 session。
							</span>
						</div>
					</div>
					{acpNotice ? <p className="settings-inline-notice">{acpNotice.text}</p> : null}
				</div>
			) : null}
		</section>
	);
}
