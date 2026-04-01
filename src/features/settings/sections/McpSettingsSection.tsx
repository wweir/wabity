import type { Dispatch, KeyboardEvent as ReactKeyboardEvent, SetStateAction } from "react";
import type { BuiltinMcpConfig, BuiltinMcpServerStatus } from "../../../lib/tauri/types";
import {
	DISCARD_DRAFT_BUTTON_LABEL,
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
	builtinMcpConfig: BuiltinMcpConfig;
	builtinMcpTransportMeta: ReturnType<typeof getMcpTransportMeta>;
	builtinMcpServerStatus: BuiltinMcpServerStatus | null;
	builtinMcpToggleDisabled: boolean;
	bindAcpFieldRef: BindAcpFieldRef;
	bindMcpListOptionRef: (serverId: string) => (node: HTMLButtonElement | null) => void;
	onSelectMcpServer: (serverId: string) => void;
	onMcpCatalogKeyDown: (event: ReactKeyboardEvent<HTMLButtonElement>, serverId: string) => void;
	onToggleBuiltinMcp: (enabled: boolean) => void;
	onToggleBuiltinMcpModule: (moduleKey: BuiltinMcpConfig["enabledModules"][number]) => void;
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
	builtinMcpConfig,
	builtinMcpTransportMeta,
	builtinMcpServerStatus,
	builtinMcpToggleDisabled,
	bindAcpFieldRef,
	bindMcpListOptionRef,
	onSelectMcpServer,
	onMcpCatalogKeyDown,
	onToggleBuiltinMcp,
	onToggleBuiltinMcpModule,
	onEnterMcpCreateMode,
	onLocateFirstMcpIssue,
	onDiscardMcpDraft,
	onSaveMcp,
	onRemoveMcpServer,
	onMcpServerFieldChange,
	onReturnToCurrentMcp,
	onApplyMcpTransportSelection,
}: McpSettingsSectionProps) {
	const saveStatusLabel = mcpHasUnsavedChanges ? "草稿未保存" : "已写入配置";
	const saveStatusDetail =
		mcpValidation.totalIssues > 0
			? `${mcpValidation.totalIssues} 个问题待修复`
			: mcpHasUnsavedChanges
				? "当前 MCP 草稿尚未写回配置。"
				: "MCP 配置已与本地 config.toml 同步。";

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
					{regularMcpServers.length === 0 ? (
						<div className="settings-empty-panel settings-empty-panel-subtle">
							<strong className="settings-empty-title">还没有自定义服务</strong>
							<span className="settings-help-text settings-help-text-tight">
								内置 MCP 在上面单独配置；如果还要接第三方 server，再新建自定义服务。
							</span>
						</div>
					) : null}
					<div className="settings-catalog-grid settings-catalog-grid-3 settings-mcp-catalog-grid">
						<article
							className={`settings-mcp-list-item settings-mcp-builtin-card ${
								builtinMcpConfig.enabled ? "settings-mcp-list-item-selected" : ""
							}`}
						>
							<div className="settings-mcp-builtin-card-header">
								<div className="settings-agent-title-row">
									<strong className="settings-agent-name">内置 MCP</strong>
								</div>
								<label className="settings-mcp-builtin-toggle">
									<span className="sr-only">启用内置 MCP</span>
									<input
										checked={builtinMcpConfig.enabled}
										className="settings-toggle"
										disabled={builtinMcpToggleDisabled}
										onChange={(event) => onToggleBuiltinMcp(event.target.checked)}
										type="checkbox"
									/>
								</label>
							</div>
							<div className="settings-mcp-list-item-badges">
								<span className="settings-status-chip">内置</span>
								<span className="settings-mcp-badge">{builtinMcpTransportMeta.label}</span>
								<span
									className={`settings-status-chip ${
										builtinMcpServerStatus?.running
											? "settings-status-chip-success"
											: "settings-status-chip-warn"
									}`}
								>
									{builtinMcpServerStatus?.running ? "运行中" : "未运行"}
								</span>
								<span className="settings-agent-meta">
									{builtinMcpConfig.enabledModules.length} 个模块已启用
								</span>
							</div>
							<span className="settings-agent-command-preview">
								{builtinMcpServerStatus?.server.transport === "http"
									? builtinMcpServerStatus.server.url
									: "http://127.0.0.1:43189/internal/mcp"}
							</span>
							<span className="settings-help-text settings-help-text-tight">
								{builtinMcpConfig.enabled
									? "保存后会把当前启用的内置模块统一暴露给 Agent。"
									: builtinMcpServerStatus?.running
										? "开启后会把下面勾选的内置模块统一挂到同一个本地 MCP server。"
										: builtinMcpServerStatus?.lastError || "桌面端启动后会自动暴露这个本地地址。"}
							</span>
							<div className="settings-mcp-builtin-modules">
								{builtinMcpServerStatus?.availableModules.map((module) => (
									<label className="settings-mcp-builtin-module" key={module.key}>
										<input
											checked={builtinMcpConfig.enabledModules.includes(module.key)}
											className="settings-toggle"
											onChange={() => onToggleBuiltinMcpModule(module.key)}
											type="checkbox"
										/>
										<span className="settings-mcp-builtin-module-copy">
											<strong>{module.title}</strong>
											<span className="settings-help-text settings-help-text-tight">
												{module.summary} · {module.toolCount} 个 tool
											</span>
										</span>
									</label>
								))}
							</div>
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
					{isMcpEditMode && selectedMcpServer ? (
						<>
							<div className="settings-rag-toolbar settings-mcp-toolbar">
								<div className="settings-mcp-toolbar-primary">
									<div
										aria-live="polite"
										className="settings-rag-toolbar-heading settings-mcp-toolbar-heading"
									>
										<strong className="settings-agent-name">
											{getMcpServerDraftTitle(selectedMcpServer)}
										</strong>
										<span
											className={`settings-mcp-toolbar-state-badge ${
												mcpHasUnsavedChanges ? "settings-mcp-toolbar-state-badge-dirty" : ""
											}`}
										>
											{saveStatusLabel}
										</span>
									</div>
									<span className="settings-rag-toolbar-note">{saveStatusDetail}</span>
								</div>
								<div className="settings-rag-toolbar-actions settings-mcp-toolbar-actions">
									{mcpValidation.totalIssues > 0 ? (
										<button
											className="settings-text-link settings-text-link-action"
											onClick={onLocateFirstMcpIssue}
											type="button"
										>
											定位问题
										</button>
									) : null}
									{mcpHasUnsavedChanges ? (
										<button
											className="settings-text-link settings-text-link-action"
											disabled={savingMcp}
											onClick={onDiscardMcpDraft}
											type="button"
										>
											{DISCARD_DRAFT_BUTTON_LABEL}
										</button>
									) : null}
									<button
										className="settings-text-link settings-text-link-action settings-text-link-danger"
										onClick={() => onRemoveMcpServer(selectedMcpServer.id)}
										type="button"
									>
										删除服务
									</button>
									{mcpHasUnsavedChanges ? (
										<button
											className="settings-button"
											disabled={savingMcp}
											onClick={() => void onSaveMcp()}
											type="button"
										>
											{savingMcp ? "保存中..." : "保存 MCP 配置"}
										</button>
									) : null}
								</div>
							</div>

							<div className="settings-mcp-editor-main">
								<div
									className="settings-rag-editor-panel settings-mcp-editor-panel"
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

									<div className="settings-agent-fields settings-mcp-panel-fields">
										<label className="settings-label settings-label-stacked settings-mcp-form-row">
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
										<div className="settings-mcp-static-row">
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

								<div className="settings-rag-editor-panel settings-mcp-editor-panel">
									<div className="settings-editor-card-header">
										<strong className="settings-agent-mcp-title">连接配置</strong>
										<span className="settings-agent-meta">
											{getMcpTransportMeta(selectedMcpServer.transport).description}
										</span>
									</div>
									<div className="settings-agent-fields settings-mcp-panel-fields">
										{selectedMcpServer.transport === "stdio" ? (
											<>
												<label className="settings-label settings-label-stacked settings-mcp-form-row">
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
												<label className="settings-label settings-label-stacked settings-mcp-field-wide">
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
												<label className="settings-label settings-label-stacked settings-mcp-field-wide">
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
												<label className="settings-label settings-label-stacked settings-mcp-form-row">
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
															onMcpServerFieldChange(
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
													{renderFieldError(
														buildFieldIssueId("mcp", "url", selectedMcpServer.id),
														selectedMcpFieldIssues.url,
													)}
												</label>
												<label className="settings-label settings-label-stacked settings-mcp-field-wide">
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
							</div>
						</>
					) : null}
					{isMcpCreateMode ? (
						<div
							className="settings-editor-card settings-editor-card-subtle settings-mcp-create-card"
							id="mcp-create"
							ref={bindSectionBlockRef("mcp-create")}
						>
							<div className="settings-rag-toolbar settings-mcp-toolbar settings-mcp-create-toolbar">
								<div className="settings-mcp-toolbar-primary">
									<div className="settings-rag-toolbar-heading settings-mcp-toolbar-heading">
										<strong className="settings-agent-name">新建服务</strong>
									</div>
									<span className="settings-rag-toolbar-note">
										只在需要另一种连接方式时新增，创建后会直接切到当前服务。
									</span>
								</div>
								{selectedMcpServer ? (
									<div className="settings-rag-toolbar-actions settings-mcp-toolbar-actions">
										<button
											className="settings-text-link settings-text-link-action"
											onClick={onReturnToCurrentMcp}
											type="button"
										>
											返回当前服务
										</button>
									</div>
								) : null}
							</div>
							<div className="settings-acp-preset-panel settings-mcp-create-panel">
								<div className="settings-mcp-create-copy">
									<span className="settings-acp-preset-kicker">空白</span>
									<strong className="settings-acp-preset-label">创建一个新服务</strong>
								</div>
								<label className="settings-label settings-label-stacked settings-mcp-form-row settings-mcp-create-field">
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
