import type { Dispatch, SetStateAction } from "react";
import type { LlmProviderConfig, RagScanResult, RagSettings } from "../../../lib/tauri/types";
import {
	DISCARD_DRAFT_BUTTON_LABEL,
	buildFieldIssueId,
	getSettingsPanelId,
	getSettingsTabId,
	joinDescribedByIds,
} from "../settingsShared";
import {
	type BindRagFieldRef,
	type BindSectionBlockRef,
	renderFieldError,
	renderValidationIssueBox,
} from "../sectionViewShared";
import type { RagDraftValidation } from "../settingsTypes";

export interface RagSettingsSectionProps {
	bindSectionBlockRef: BindSectionBlockRef;
	ragSettings: RagSettings;
	ragValidation: RagDraftValidation;
	ragScanResult: RagScanResult | null;
	ragSummaryItems: Array<{ label: string; value: string }>;
	ragScanSummaryItems: Array<{ label: string; value: string }>;
	selectedRagEmbeddingProviderLabel: string;
	selectedRagEmbeddingProvider: LlmProviderConfig | null;
	eligibleRagEmbeddingProviders: LlmProviderConfig[];
	ragSupportedExtensionsLabel: string;
	ragHasUnsavedChanges: boolean;
	ragStatusTitle: string;
	ragStatusDescription: string;
	savingRag: boolean;
	scanningRag: boolean;
	ragSourceDirectoryPlaceholder: string;
	ragExtraIgnoreGlobs: string[];
	defaultRagIgnoreGlobs: readonly string[];
	extraRagIgnoreGlobPlaceholder: string;
	bindRagFieldRef: BindRagFieldRef;
	setRagSettings: Dispatch<SetStateAction<RagSettings>>;
	onLocateFirstRagIssue: () => void;
	onDiscardRagDraft: () => void;
	onScanRag: () => Promise<void>;
	onSaveRag: () => Promise<void>;
	onAppendRagSourceDirectory: () => Promise<void>;
	parseTextLines: (value: string) => string[];
	formatTextLines: (lines: string[]) => string;
	normalizeRagIgnoreGlobs: (ignoreGlobs: string[]) => string[];
}

export function RagSettingsSection({
	bindSectionBlockRef,
	ragSettings,
	ragValidation,
	ragScanResult,
	ragSummaryItems,
	ragScanSummaryItems,
	selectedRagEmbeddingProviderLabel,
	selectedRagEmbeddingProvider,
	eligibleRagEmbeddingProviders,
	ragSupportedExtensionsLabel,
	ragHasUnsavedChanges,
	ragStatusTitle,
	ragStatusDescription,
	savingRag,
	scanningRag,
	ragSourceDirectoryPlaceholder,
	ragExtraIgnoreGlobs,
	defaultRagIgnoreGlobs,
	extraRagIgnoreGlobPlaceholder,
	bindRagFieldRef,
	setRagSettings,
	onLocateFirstRagIssue,
	onDiscardRagDraft,
	onScanRag,
	onSaveRag,
	onAppendRagSourceDirectory,
	parseTextLines,
	formatTextLines,
	normalizeRagIgnoreGlobs,
}: RagSettingsSectionProps) {
	return (
		<section
			aria-labelledby={getSettingsTabId("rag")}
			className="settings-section"
			id={getSettingsPanelId("rag")}
			role="tabpanel"
			tabIndex={0}
		>
			<div className="settings-rag-layout">
				<aside
					className="settings-rag-sidebar"
					id="rag-summary"
					ref={bindSectionBlockRef("rag-summary")}
				>
					<div className="settings-rag-summary-panel">
						<div className="settings-rag-summary-header">
							<span className="settings-section-kicker">当前配置</span>
							<strong className="settings-agent-name">索引配置</strong>
							<span className="settings-help-text settings-help-text-tight">
								RAG 只关心 Embedding、扫描目录和忽略规则。
							</span>
						</div>
						<div className="settings-rag-summary-current">
							<span className="settings-rag-summary-current-label">当前 Embedding</span>
							<strong className="settings-rag-summary-current-value">
								{selectedRagEmbeddingProviderLabel}
							</strong>
							<span className="settings-agent-meta">
								{selectedRagEmbeddingProvider
									? `${selectedRagEmbeddingProvider.model} · ${selectedRagEmbeddingProvider.baseUrl}`
									: eligibleRagEmbeddingProviders.length > 0
										? "右侧选一个 Embedding 条目后，RAG 才能建立索引。"
										: "当前没有可用于 RAG 的 Embedding 条目，先去 LLM 页面新增一个。"}
							</span>
							<span className="settings-rag-summary-note">
								建立索引时，文档内容会发送给当前 Embedding
								模型。涉及隐私或敏感数据时，优先选本机部署或你明确信任的模型服务。
							</span>
						</div>
						<dl className="settings-rag-summary-facts">
							{ragSummaryItems.map((item) => (
								<div className="settings-rag-summary-fact" key={item.label}>
									<dt className="settings-rag-summary-fact-label">{item.label}</dt>
									<dd className="settings-rag-summary-fact-value">{item.value}</dd>
								</div>
							))}
						</dl>
						<div className="settings-rag-summary-footer">
							<div className="settings-rag-summary-rule">
								<span className="settings-rag-summary-rule-label">支持后缀</span>
								<span className="settings-rag-summary-note">{ragSupportedExtensionsLabel}</span>
							</div>
							<div className="settings-rag-summary-rule">
								<span className="settings-rag-summary-rule-label">索引边界</span>
								<span className="settings-rag-summary-note">
									只扫描显式目录内、未命中忽略规则且不超过 50 MB 的文件。纯文本类文件要求可读 UTF-8
									且不含 NUL；DOCX 会先抽取正文，文本型 PDF 会先按页抽取。
								</span>
							</div>
							<div className="settings-rag-summary-rule">
								<span className="settings-rag-summary-rule-label">重建规则</span>
								<span className="settings-rag-summary-note">
									更换 Embedding
									条目、扫描目录或忽略规则后，保存配置会自动重建索引；其它场景保持现状，需要时手动点击“立即重建索引”。
								</span>
							</div>
							<div className="settings-rag-summary-section">
								<div className="settings-rag-summary-section-header">
									<span className="settings-rag-summary-rule-label">最近一次手动重建</span>
									<span className="settings-rag-summary-note">
										{ragScanResult
											? "这里只展示当前窗口内最近一次手动触发的扫描结果。"
											: "还没有手动重建结果。保存配置不会自动跑一次全量扫描。"}
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
								) : null}
							</div>
						</div>
					</div>
				</aside>

				<div
					className="settings-rag-detail"
					id="rag-pipeline"
					ref={bindSectionBlockRef("rag-pipeline")}
				>
					<div className="settings-rag-toolbar">
						<div className="settings-rag-toolbar-heading">
							<span className="settings-section-kicker">索引配置</span>
							<span
								className={`settings-status-chip ${ragHasUnsavedChanges ? "settings-status-chip-strong" : ""}`}
							>
								{ragStatusTitle}
							</span>
							<span className="settings-rag-toolbar-note">{ragStatusDescription}</span>
						</div>
						<div className="settings-rag-toolbar-actions">
							{ragValidation.totalIssues > 0 ? (
								<button
									className="settings-button settings-agent-secondary"
									onClick={onLocateFirstRagIssue}
									type="button"
								>
									定位问题
								</button>
							) : null}
							{ragHasUnsavedChanges ? (
								<button
									className="settings-button settings-agent-secondary"
									disabled={savingRag || scanningRag}
									onClick={onDiscardRagDraft}
									type="button"
								>
									{DISCARD_DRAFT_BUTTON_LABEL}
								</button>
							) : null}
							<button
								className="settings-button settings-agent-secondary"
								disabled={savingRag || scanningRag}
								onClick={() => void onScanRag()}
								type="button"
							>
								{scanningRag ? "扫描中..." : "立即重建索引"}
							</button>
							{ragHasUnsavedChanges ? (
								<button
									className="settings-button"
									disabled={savingRag || scanningRag}
									onClick={() => void onSaveRag()}
									type="button"
								>
									{savingRag ? "保存中..." : "保存 RAG 配置"}
								</button>
							) : null}
						</div>
					</div>

					<div className="settings-rag-editor-grid">
						<div className="settings-rag-editor-main">
							<div className="settings-rag-editor-panel">
								<div className="settings-editor-card-header">
									<div className="settings-acp-detail-copy">
										<span className="settings-section-kicker">Embedding</span>
										<strong className="settings-agent-name">Embedding</strong>
									</div>
								</div>
								<div className="settings-item settings-item-stacked settings-item-wide">
									<label
										className="settings-label settings-label-stacked"
										htmlFor="rag-embedding-provider"
									>
										<span>Embedding 条目</span>
										<span
											className="settings-item-description"
											id="rag-embedding-provider-description"
										>
											RAG 只接受 Embedding 类型条目。没有可选项时，先去 LLM 页面新增一个。
										</span>
									</label>
									<select
										aria-describedby={joinDescribedByIds(
											"rag-embedding-provider-description",
											ragValidation.fieldIssues.embeddingProviderId
												? buildFieldIssueId("rag", "embedding-provider")
												: undefined,
										)}
										aria-invalid={ragValidation.fieldIssues.embeddingProviderId ? true : undefined}
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
									{renderFieldError(
										buildFieldIssueId("rag", "embedding-provider"),
										ragValidation.fieldIssues.embeddingProviderId,
									)}
								</div>
							</div>

							<div className="settings-rag-editor-panel">
								<div className="settings-editor-card-header">
									<div className="settings-acp-detail-copy">
										<span className="settings-section-kicker">目录范围</span>
										<strong className="settings-agent-name">目录与忽略规则</strong>
									</div>
								</div>

								<div className="settings-item settings-item-stacked">
									<label
										className="settings-label settings-label-stacked"
										htmlFor="rag-source-dirs"
									>
										<span>扫描目录</span>
										<span className="settings-item-description" id="rag-source-dirs-description">
											扫描目录与忽略规则。每行一个目录，目录选择器默认从 `~/Documents`
											打开。Markdown 和 DOCX 会按标题、列表、代码块和段落预切；TXT、RST、 ADOC
											走通用文本切分；PDF 先按页抽取再进入索引。
										</span>
									</label>
									<textarea
										aria-describedby={joinDescribedByIds(
											"rag-source-dirs-description",
											ragValidation.fieldIssues.sourceDirectories
												? buildFieldIssueId("rag", "source-directories")
												: undefined,
										)}
										aria-invalid={ragValidation.fieldIssues.sourceDirectories ? true : undefined}
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
											onClick={() => void onAppendRagSourceDirectory()}
											type="button"
										>
											选择目录追加
										</button>
									</div>
									{renderFieldError(
										buildFieldIssueId("rag", "source-directories"),
										ragValidation.fieldIssues.sourceDirectories,
									)}
								</div>

								<div className="settings-item settings-item-stacked">
									<label className="settings-label settings-label-stacked">
										<span>内置忽略目录</span>
										<span className="settings-item-description" id="rag-fixed-ignore-globs-help">
											这些规则始终生效，不支持取消：`{defaultRagIgnoreGlobs.join("`、`")}
											`。命中后文件不会被切分、向量化或写入 LanceDB。
										</span>
									</label>
								</div>

								<div className="settings-item settings-item-stacked">
									<label
										className="settings-label settings-label-stacked"
										htmlFor="rag-ignore-globs"
									>
										<span>额外忽略通配符</span>
										<span className="settings-item-description" id="rag-ignore-globs-help">
											每行一个附加 glob。这里只能新增，不能移除上面的内置忽略目录。
										</span>
									</label>
									<textarea
										aria-describedby={joinDescribedByIds(
											"rag-ignore-globs-help",
											ragValidation.fieldIssues.ignoreGlobs
												? buildFieldIssueId("rag", "ignore-globs")
												: undefined,
										)}
										aria-invalid={ragValidation.fieldIssues.ignoreGlobs ? true : undefined}
										className="settings-textarea settings-input settings-input-mono"
										disabled={savingRag || scanningRag}
										id="rag-ignore-globs"
										onChange={(event) =>
											setRagSettings((current) => ({
												...current,
												ignoreGlobs: normalizeRagIgnoreGlobs(parseTextLines(event.target.value)),
											}))
										}
										placeholder={extraRagIgnoreGlobPlaceholder}
										ref={bindRagFieldRef("ignoreGlobs")}
										rows={5}
										value={formatTextLines(ragExtraIgnoreGlobs)}
									/>
									{renderFieldError(
										buildFieldIssueId("rag", "ignore-globs"),
										ragValidation.fieldIssues.ignoreGlobs,
									)}
								</div>
							</div>
						</div>
					</div>

					{renderValidationIssueBox(ragValidation.issues)}
				</div>
			</div>
		</section>
	);
}
