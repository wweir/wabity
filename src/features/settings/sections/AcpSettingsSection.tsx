import type { Dispatch, ReactElement, SetStateAction } from "react";
import {
	DISCARD_DRAFT_BUTTON_LABEL,
	SettingsDraftActionCard,
	acpAgentOptions,
	acpAgentLaunchModeOptions,
	buildFieldIssueId,
	formatAcpAgentOptionLabel,
	getAcpAgentLaunchModeMeta,
	getSettingsPanelId,
	getSettingsTabId,
	joinDescribedByIds,
	renderPresetInstallGuide,
} from "../settingsShared";
import {
	type BindAcpFieldRef,
	type BindSectionBlockRef,
	renderFieldError,
	renderIssueList,
} from "../sectionViewShared";
import type {
	AcpAgentDraft,
	AcpDraftValidation,
	AcpFieldKey,
	AcpInlineNotice,
	FieldIssueMap,
} from "../settingsTypes";

interface AcpPresetSelectionCardProps {
	acpAgents: AcpAgentDraft[];
	bindSectionBlockRef: BindSectionBlockRef;
	blockId: string;
	onAddCustomAgent?: () => void;
	onApplyPresetSelection: () => void;
	selectedPresetInstallOption: (typeof acpAgentOptions)[number] | null;
	selectedPresetOptionId: string;
	setSelectedPresetOptionId: Dispatch<SetStateAction<string>>;
	showConfiguredState: boolean;
}

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
	bindAcpFieldRef: BindAcpFieldRef;
	setSelectedAgentId: Dispatch<SetStateAction<string | null>>;
	onAddCustomAgent: () => void;
	onLocateFirstAcpIssue: () => void;
	onDiscardAcpDraft: () => void;
	onSaveAgent: () => Promise<void>;
	onRemoveAgent: (agentId: string) => void;
	onApplyPresetSelection: () => void;
	onAgentFieldChange: (agentId: string, key: AcpFieldKey, value: string) => void;
}

function AcpPresetSelectionCard({
	acpAgents,
	bindSectionBlockRef,
	blockId,
	onAddCustomAgent,
	onApplyPresetSelection,
	selectedPresetInstallOption,
	selectedPresetOptionId,
	setSelectedPresetOptionId,
	showConfiguredState,
}: AcpPresetSelectionCardProps): ReactElement {
	return (
		<div className="settings-editor-card" id={blockId} ref={bindSectionBlockRef(blockId)}>
			<div className="settings-editor-card-header">
				<strong className="settings-agent-mcp-title">选择 Agent</strong>
				<span className="settings-agent-meta">只填默认值，不会直接保存</span>
			</div>
			<div className="settings-acp-preset-layout">
				<div className="settings-acp-preset-controls">
					<div className="settings-acp-preset-panel">
						<span className="settings-acp-preset-kicker">快速填充</span>
						<label className="settings-label settings-label-stacked settings-acp-preset-field">
							<span className="settings-acp-preset-label">Agent 模板</span>
							<select
								aria-label="选择 Agent"
								className="settings-select"
								onChange={(event) => setSelectedPresetOptionId(event.target.value)}
								value={selectedPresetOptionId}
							>
								<option value="__custom__">自定义 · 空白表单</option>
								{acpAgentOptions.map((option) => {
									const alreadyAdded =
										showConfiguredState &&
										acpAgents.some(
											(agent) =>
												agent.command.trim() === option.command &&
												agent.launchMode === option.launchMode,
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
							onClick={onApplyPresetSelection}
							type="button"
						>
							填入表单
						</button>
						{onAddCustomAgent ? (
							<div className="settings-acp-empty-actions">
								<button
									className="settings-button settings-agent-secondary"
									onClick={onAddCustomAgent}
									type="button"
								>
									创建空白 Agent
								</button>
								<span className="settings-help-text settings-help-text-tight">
									先选模板填默认值，或者直接创建一个空白表单。
								</span>
							</div>
						) : null}
					</div>
				</div>
				<div className="settings-install-guide settings-install-guide-card">
					{renderPresetInstallGuide(selectedPresetInstallOption, selectedPresetOptionId)}
				</div>
			</div>
		</div>
	);
}

export function AcpSettingsSection({
	bindSectionBlockRef,
	acpNotice,
	acpAgents,
	acpValidation,
	selectedAgentId,
	selectedAgent,
	selectedAgentIssueCount,
	selectedAgentFieldIssues,
	acpHasUnsavedChanges,
	savingAgent,
	selectedPresetOptionId,
	setSelectedPresetOptionId,
	bindAcpFieldRef,
	setSelectedAgentId,
	onAddCustomAgent,
	onLocateFirstAcpIssue,
	onDiscardAcpDraft,
	onSaveAgent,
	onRemoveAgent,
	onApplyPresetSelection,
	onAgentFieldChange,
}: AcpSettingsSectionProps) {
	const selectedPresetInstallOption =
		acpAgentOptions.find((option) => option.id === selectedPresetOptionId) ?? null;

	return (
		<section
			aria-labelledby={getSettingsTabId("acp")}
			className="settings-section"
			id={getSettingsPanelId("acp")}
			role="tabpanel"
			tabIndex={0}
		>
			{acpNotice ? (
				<div className={`settings-banner settings-banner-${acpNotice.tone}`}>{acpNotice.text}</div>
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
						{acpAgents.length > 0 ? (
							<button
								className="settings-button settings-button-compact"
								onClick={onAddCustomAgent}
								type="button"
							>
								+ 新建
							</button>
						) : null}
					</div>
					<p className="settings-help-text settings-help-text-tight">
						ACP Agent 就是一条启动命令配置。预设只负责填表；实际创建 Session 用哪个 Agent，看
						launcher 顶部选择。
					</p>

					{acpAgents.length === 0 ? (
						<div className="settings-empty-panel">
							<strong className="settings-empty-title">还没有 Agent</strong>
							<span className="settings-help-text settings-help-text-tight">
								先从右侧模板填默认值，或者直接创建一个空白 Agent。
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
												<span className="settings-agent-meta">
													{issueCount} 个问题 · {getAcpAgentLaunchModeMeta(agent.launchMode).label}
												</span>
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
					{selectedAgent || acpHasUnsavedChanges || acpValidation.totalIssues > 0 ? (
						<SettingsDraftActionCard
							className="settings-draft-action-card-acp"
							actions={
								<>
									{acpValidation.totalIssues > 0 ? (
										<button
											className="settings-button settings-agent-secondary"
											onClick={onLocateFirstAcpIssue}
											type="button"
										>
											定位问题
										</button>
									) : null}
									<button
										className="settings-button settings-agent-secondary"
										disabled={savingAgent || !acpHasUnsavedChanges}
										onClick={onDiscardAcpDraft}
										type="button"
									>
										{DISCARD_DRAFT_BUTTON_LABEL}
									</button>
									<button
										className="settings-button"
										disabled={savingAgent || !acpHasUnsavedChanges}
										onClick={() => void onSaveAgent()}
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
					) : null}
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
									onClick={() => onRemoveAgent(selectedAgent.id)}
									type="button"
								>
									删除 Agent
								</button>
							</div>

							{renderIssueList(acpValidation.agentIssues[selectedAgent.id] ?? [])}

							<AcpPresetSelectionCard
								acpAgents={acpAgents}
								bindSectionBlockRef={bindSectionBlockRef}
								blockId="acp-presets"
								onApplyPresetSelection={onApplyPresetSelection}
								selectedPresetInstallOption={selectedPresetInstallOption}
								selectedPresetOptionId={selectedPresetOptionId}
								setSelectedPresetOptionId={setSelectedPresetOptionId}
								showConfiguredState={true}
							/>

							<div
								className="settings-editor-card settings-editor-card-acp-form"
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
								<div className="settings-agent-fields settings-agent-fields-acp">
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
												onAgentFieldChange(selectedAgent.id, "name", event.target.value)
											}
											placeholder="例如 Codex"
											ref={bindAcpFieldRef("agent", selectedAgent.id, "name")}
											type="text"
											value={selectedAgent.name}
										/>
										{renderFieldError(
											buildFieldIssueId("acp", "name", selectedAgent.id),
											selectedAgentFieldIssues.name,
										)}
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
												onAgentFieldChange(selectedAgent.id, "command", event.target.value)
											}
											placeholder="输入单行命令，例如 codex-acp"
											ref={bindAcpFieldRef("agent", selectedAgent.id, "command")}
											type="text"
											value={selectedAgent.command}
										/>
										<span
											className="settings-help-text settings-help-text-tight"
											id={`settings-acp-${selectedAgent.id}-command-help`}
										>
											这里只填写 Agent 启动命令；direct 会解析为 program + args，shell
											模式会把整行交给用户默认 shell。所有 Agent 仍共用同一份全局 MCP 配置。
										</span>
										{renderFieldError(
											buildFieldIssueId("acp", "command", selectedAgent.id),
											selectedAgentFieldIssues.command,
										)}
									</label>
									<label className="settings-label settings-label-stacked">
										<span>启动模式</span>
										<select
											aria-describedby={joinDescribedByIds(
												`settings-acp-${selectedAgent.id}-launch-mode-help`,
											)}
											className="settings-select"
											onChange={(event) =>
												onAgentFieldChange(selectedAgent.id, "launchMode", event.target.value)
											}
											ref={bindAcpFieldRef("agent", selectedAgent.id, "launchMode")}
											value={selectedAgent.launchMode}
										>
											{acpAgentLaunchModeOptions.map((option) => (
												<option key={option.value} value={option.value}>
													{option.label}
												</option>
											))}
										</select>
										<span
											className="settings-help-text settings-help-text-tight"
											id={`settings-acp-${selectedAgent.id}-launch-mode-help`}
										>
											{getAcpAgentLaunchModeMeta(selectedAgent.launchMode).description}
										</span>
									</label>
								</div>
							</div>
						</>
					) : (
						<AcpPresetSelectionCard
							acpAgents={acpAgents}
							bindSectionBlockRef={bindSectionBlockRef}
							blockId="acp-presets"
							onAddCustomAgent={onAddCustomAgent}
							onApplyPresetSelection={onApplyPresetSelection}
							selectedPresetInstallOption={selectedPresetInstallOption}
							selectedPresetOptionId={selectedPresetOptionId}
							setSelectedPresetOptionId={setSelectedPresetOptionId}
							showConfiguredState={false}
						/>
					)}
				</div>
			</div>
		</section>
	);
}
