import type { Dispatch, KeyboardEvent as ReactKeyboardEvent, SetStateAction } from "react";
import type { BuiltinRagMcpServerStatus } from "../../../lib/tauri/types";
import {
	DISCARD_DRAFT_BUTTON_LABEL,
	SettingsDraftActionCard,
	buildFieldIssueId,
	getMcpServerDraftTitle,
	getMcpTransportMeta,
	getSettingsPanelId,
	getSettingsTabId,
	joinDescribedByIds,
	mcpTransportOptions,
	renderMcpTransportGuide,
	summarizeMcpServerDraft,
} from "../settingsShared";
import {
	type BindAcpFieldRef,
	type BindSectionBlockRef,
	renderFieldError,
} from "../sectionViewShared";
import type {
	AcpInlineNotice,
	AcpMcpServerDraft,
	FieldIssueMap,
	McpDraftValidation,
	McpFieldKey,
	McpTransport,
} from "../settingsTypes";

export interface McpSettingsSectionProps {
	bindSectionBlockRef: BindSectionBlockRef;
	mcpNotice: AcpInlineNotice | null;
	mcpServers: AcpMcpServerDraft[];
	regularMcpServers: AcpMcpServerDraft[];
	selectedMcpServerId: string | null;
	selectedMcpServer: AcpMcpServerDraft | null;
	selectedMcpIssueCount: number;
	selectedMcpFieldIssues: FieldIssueMap<McpFieldKey>;
	mcpValidation: McpDraftValidation;
	mcpHasUnsavedChanges: boolean;
	savingMcp: boolean;
	isMcpCreateMode: boolean;
	isMcpEditMode: boolean;
	selectedMcpTransport: McpTransport;
	setSelectedMcpTransport: Dispatch<SetStateAction<McpTransport>>;
	builtinRagMcpConfigured: boolean;
	builtinRagMcpServer: AcpMcpServerDraft | null;
	builtinRagMcpIssueCount: number;
	builtinRagMcpTransportMeta: ReturnType<typeof getMcpTransportMeta>;
	builtinRagMcpServerStatus: BuiltinRagMcpServerStatus | null;
	builtinRagMcpToggleDisabled: boolean;
	bindAcpFieldRef: BindAcpFieldRef;
	bindMcpListOptionRef: (serverId: string) => (node: HTMLButtonElement | null) => void;
	onSelectMcpServer: (serverId: string) => void;
	onMcpCatalogKeyDown: (event: ReactKeyboardEvent<HTMLButtonElement>, serverId: string) => void;
	onToggleBuiltinRagMcpServer: (enabled: boolean) => void;
	onEnterMcpCreateMode: () => void;
	onLocateFirstMcpIssue: () => void;
	onDiscardMcpDraft: () => void;
	onSaveMcp: () => Promise<void>;
	onRemoveMcpServer: (serverId: string) => void;
	onMcpServerFieldChange: (
		serverId: string,
		key: keyof Omit<AcpMcpServerDraft, "id">,
		value: string,
	) => void;
	onReturnToCurrentMcp: () => void;
	onApplyMcpTransportSelection: () => void;
}

export function McpSettingsSection({
	bindSectionBlockRef,
	mcpNotice,
	mcpServers,
	regularMcpServers,
	selectedMcpServerId,
	selectedMcpServer,
	selectedMcpIssueCount,
	selectedMcpFieldIssues,
	mcpValidation,
	mcpHasUnsavedChanges,
	savingMcp,
	isMcpCreateMode,
	isMcpEditMode,
	selectedMcpTransport,
	setSelectedMcpTransport,
	builtinRagMcpConfigured,
	builtinRagMcpServer,
	builtinRagMcpIssueCount,
	builtinRagMcpTransportMeta,
	builtinRagMcpServerStatus,
	builtinRagMcpToggleDisabled,
	bindAcpFieldRef,
	bindMcpListOptionRef,
	onSelectMcpServer,
	onMcpCatalogKeyDown,
	onToggleBuiltinRagMcpServer,
	onEnterMcpCreateMode,
	onLocateFirstMcpIssue,
	onDiscardMcpDraft,
	onSaveMcp,
	onRemoveMcpServer,
	onMcpServerFieldChange,
	onReturnToCurrentMcp,
	onApplyMcpTransportSelection,
}: McpSettingsSectionProps) {
	return (
		<section
			aria-labelledby={getSettingsTabId("mcp")}
			className="settings-section"
			id={getSettingsPanelId("mcp")}
			role="tabpanel"
			tabIndex={0}
		>
			{mcpNotice ? (
				<div className={`settings-banner settings-banner-${mcpNotice.tone}`}>{mcpNotice.text}</div>
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
								builtinRagMcpConfigured && selectedMcpServerId === builtinRagMcpServer?.id
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
										onChange={(event) => onToggleBuiltinRagMcpServer(event.target.checked)}
										type="checkbox"
									/>
								</label>
							</div>
							<div className="settings-mcp-list-item-badges">
								<span className="settings-status-chip">内置</span>
								<span className="settings-mcp-badge">{builtinRagMcpTransportMeta.label}</span>
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
									<span className="settings-agent-meta">{builtinRagMcpIssueCount} 个问题</span>
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
									onClick={() => onSelectMcpServer(builtinRagMcpServer.id)}
									type="button"
								>
									编辑当前服务
								</button>
							) : null}
						</article>
						<div aria-label="MCP 服务目录" className="settings-mcp-catalog-group" role="radiogroup">
							{regularMcpServers.map((server) => {
								const transportMeta = getMcpTransportMeta(server.transport);
								const issueCount = mcpValidation.serverIssues[server.id]?.length ?? 0;
								const isSelected = isMcpEditMode && selectedMcpServerId === server.id;
								const isFocusable =
									isSelected || (!isMcpEditMode && regularMcpServers[0]?.id === server.id);
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
											onClick={() => onSelectMcpServer(server.id)}
											onKeyDown={(event) => onMcpCatalogKeyDown(event, server.id)}
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
												<span className="settings-mcp-badge">{transportMeta.label}</span>
												{issueCount > 0 ? (
													<span className="settings-agent-meta">{issueCount} 个问题</span>
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
								onClick={onEnterMcpCreateMode}
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
										onClick={onLocateFirstMcpIssue}
										type="button"
									>
										定位问题
									</button>
								) : null}
								<button
									className="settings-button settings-agent-secondary"
									disabled={savingMcp || !mcpHasUnsavedChanges}
									onClick={onDiscardMcpDraft}
									type="button"
								>
									{DISCARD_DRAFT_BUTTON_LABEL}
								</button>
								<button
									className="settings-button"
									disabled={savingMcp || !mcpHasUnsavedChanges}
									onClick={() => void onSaveMcp()}
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
					{isMcpEditMode && selectedMcpServer ? (
						<>
							<div className="settings-acp-detail-header settings-mcp-detail-header">
								<div className="settings-acp-detail-copy">
									<span className="settings-section-kicker">当前服务</span>
									<h3 className="settings-subsection-title">
										{getMcpServerDraftTitle(selectedMcpServer)}
									</h3>
									<span className="settings-help-text settings-help-text-tight">
										目录和表单始终对应同一条服务。
									</span>
								</div>
								<button
									className="settings-agent-remove settings-agent-remove-inline"
									onClick={() => onRemoveMcpServer(selectedMcpServer.id)}
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
												onMcpServerFieldChange(selectedMcpServer.id, "name", event.target.value)
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
										{renderFieldError(
											buildFieldIssueId("mcp", "name", selectedMcpServer.id),
											selectedMcpFieldIssues.name,
										)}
									</label>
									<div className="settings-item settings-item-stacked settings-item-wide">
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
															? buildFieldIssueId("mcp", "command", selectedMcpServer.id)
															: undefined,
													)}
													aria-invalid={selectedMcpFieldIssues.command ? true : undefined}
													className="settings-input settings-input-wide settings-input-mono"
													onChange={(event) =>
														onMcpServerFieldChange(
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
												{renderFieldError(
													buildFieldIssueId("mcp", "command", selectedMcpServer.id),
													selectedMcpFieldIssues.command,
												)}
											</label>
											<label className="settings-label settings-label-stacked">
												<span>参数</span>
												<textarea
													className="settings-textarea settings-input-mono"
													onChange={(event) =>
														onMcpServerFieldChange(
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
															? buildFieldIssueId("mcp", "env-text", selectedMcpServer.id)
															: undefined,
													)}
													aria-invalid={selectedMcpFieldIssues.envText ? true : undefined}
													className="settings-textarea settings-input-mono"
													onChange={(event) =>
														onMcpServerFieldChange(
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
												{renderFieldError(
													buildFieldIssueId("mcp", "env-text", selectedMcpServer.id),
													selectedMcpFieldIssues.envText,
												)}
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
														onMcpServerFieldChange(selectedMcpServer.id, "url", event.target.value)
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
												{renderFieldError(
													buildFieldIssueId("mcp", "url", selectedMcpServer.id),
													selectedMcpFieldIssues.url,
												)}
											</label>
											<label className="settings-label settings-label-stacked">
												<span>请求头</span>
												<textarea
													aria-describedby={joinDescribedByIds(
														selectedMcpFieldIssues.headersText
															? buildFieldIssueId("mcp", "headers-text", selectedMcpServer.id)
															: undefined,
													)}
													aria-invalid={selectedMcpFieldIssues.headersText ? true : undefined}
													className="settings-textarea settings-input-mono"
													onChange={(event) =>
														onMcpServerFieldChange(
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
												<span className="settings-help-text settings-help-text-tight">
													需要鉴权时再填写。每行一个 `KEY=VALUE`。
												</span>
												{renderFieldError(
													buildFieldIssueId("mcp", "headers-text", selectedMcpServer.id),
													selectedMcpFieldIssues.headersText,
												)}
											</label>
										</>
									)}
								</div>
							</div>
						</>
					) : null}
					{isMcpCreateMode ? (
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
										onClick={onReturnToCurrentMcp}
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
								<div className="settings-mcp-transport-guide">
									{renderMcpTransportGuide(selectedMcpTransport)}
								</div>
								<button
									className="settings-button settings-button-compact settings-acp-preset-action"
									onClick={onApplyMcpTransportSelection}
									type="button"
								>
									新建 {getMcpTransportMeta(selectedMcpTransport).label}
								</button>
							</div>
						</div>
					) : null}
				</div>
			</div>
		</section>
	);
}
