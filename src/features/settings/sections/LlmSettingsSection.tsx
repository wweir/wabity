import type { KeyboardEvent as ReactKeyboardEvent, RefObject } from "react";
import type {
	BuiltinLlmProviderTemplate,
	BuiltinLlmProviderTemplateModel,
	LlmProviderConfig,
	LlmProviderModelEntry,
	LlmSettings,
} from "../../../lib/tauri/types";
import {
	buildFieldIssueId,
	getSettingsPanelId,
	getSettingsTabId,
	joinDescribedByIds,
} from "../settingsShared";
import {
	type BindLlmFieldRef,
	type BindSectionBlockRef,
	renderFieldError,
	renderValidationIssueBox,
} from "../sectionViewShared";
import type {
	FieldIssueMap,
	LlmDraftValidation,
	LlmFieldKey,
	LlmModelFieldKey,
	LlmProviderKind,
} from "../settingsTypes";

export interface LlmSettingsSectionProps {
	bindSectionBlockRef: BindSectionBlockRef;
	llmSettings: LlmSettings;
	selectedLlmProviderId: string | null;
	selectedLlmProvider: LlmProviderConfig | null;
	selectedLlmProviderIsBuiltin: boolean;
	selectedBuiltinLlmTemplate: BuiltinLlmProviderTemplate | null;
	selectedBuiltinLlmTemplateModel: BuiltinLlmProviderTemplateModel | null;
	selectedBuiltinLlmSelectableModels: BuiltinLlmProviderTemplateModel[];
	selectedLlmProviderKind: LlmProviderKind | null;
	selectedLlmProviderModels: LlmProviderModelEntry[];
	selectedLlmProviderModelsError: string | null;
	isLoadingSelectedLlmProviderModels: boolean;
	llmValidation: LlmDraftValidation;
	selectedLlmFieldIssues: FieldIssueMap<LlmFieldKey>;
	llmHasUnsavedChanges: boolean;
	savingLlm: boolean;
	openLlmModelPickerId: string | null;
	llmModelMenuRef: RefObject<HTMLDivElement | null>;
	selectedLlmProviderTriggersRagReindex: boolean;
	selectedLlmProviderUsedByPersistedRag: boolean;
	builtinLlmTemplates: BuiltinLlmProviderTemplate[];
	onAddLlmProvider: () => void;
	onSelectLlmProvider: (providerId: string) => void;
	onRemoveLlmProvider: (providerId: string) => void;
	onLocateFirstLlmIssue: () => void;
	onDiscardLlmDraft: () => void;
	onSaveLlm: () => Promise<void>;
	onLlmProviderTemplateChange: (providerId: string, templateId: string) => void;
	onBuiltinTemplateModelChange: (providerId: string, modelId: string) => void;
	onBuiltinProviderManagedBaseUrlChange: (providerId: string, managed: boolean) => void;
	onLlmProviderKindChange: (providerId: string, kind: LlmProviderKind) => void;
	onLlmProviderFieldChange: <K extends keyof LlmProviderConfig>(
		providerId: string,
		key: K,
		value: LlmProviderConfig[K],
	) => void;
	onToggleLlmModelMenu: (provider: LlmProviderConfig, fieldKey: LlmModelFieldKey) => Promise<void>;
	onFetchLlmProviderModels: (
		provider: LlmProviderConfig,
		options?: { openField?: LlmModelFieldKey },
	) => Promise<void>;
	onLlmModelOptionClick: (
		providerId: string,
		fieldKey: LlmModelFieldKey,
		option: LlmProviderModelEntry,
	) => void;
	onLlmModelInputKeyDown: (
		event: ReactKeyboardEvent<HTMLInputElement>,
		provider: LlmProviderConfig,
		fieldKey: LlmModelFieldKey,
	) => void;
	onOpenUrl: (url: string) => void;
	bindLlmFieldRef: BindLlmFieldRef;
	buildLlmModelPickerId: (providerId: string, fieldKey: LlmModelFieldKey) => string;
	getLlmProviderKind: (
		provider: Pick<LlmProviderConfig, "modelType" | "protocol" | "supportsStateful">,
	) => LlmProviderKind;
	getLlmProviderKindLabel: (kind: LlmProviderKind) => string;
	getLlmProviderUsageBadges: (
		provider: Pick<
			LlmProviderConfig,
			"modelType" | "protocol" | "supportsMultimodal" | "supportsStateful"
		>,
	) => string[];
	getLlmProviderUsageDescription: (
		provider: Pick<
			LlmProviderConfig,
			"modelType" | "protocol" | "supportsMultimodal" | "supportsStateful"
		>,
	) => string;
	summarizeLlmProviderProfile: (provider: LlmProviderConfig) => string;
	providerCanHandleOcr: (
		provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol" | "supportsMultimodal">,
	) => boolean;
	providerIsLlmModel: (provider: Pick<LlmProviderConfig, "modelType">) => boolean;
	providerHasResponsesModel: (
		provider: Pick<LlmProviderConfig, "modelType" | "model" | "protocol">,
	) => boolean;
	getLlmProviderModelPlaceholder: (provider: Pick<LlmProviderConfig, "modelType">) => string;
}

export function LlmSettingsSection({
	bindSectionBlockRef,
	llmSettings,
	selectedLlmProviderId,
	selectedLlmProvider,
	selectedLlmProviderIsBuiltin,
	selectedBuiltinLlmTemplate,
	selectedBuiltinLlmTemplateModel,
	selectedBuiltinLlmSelectableModels,
	selectedLlmProviderKind,
	selectedLlmProviderModels,
	selectedLlmProviderModelsError,
	isLoadingSelectedLlmProviderModels,
	llmValidation,
	selectedLlmFieldIssues,
	llmHasUnsavedChanges,
	savingLlm,
	openLlmModelPickerId,
	llmModelMenuRef,
	builtinLlmTemplates,
	onAddLlmProvider,
	onSelectLlmProvider,
	onRemoveLlmProvider,
	onLocateFirstLlmIssue,
	onDiscardLlmDraft,
	onSaveLlm,
	onLlmProviderTemplateChange,
	onBuiltinTemplateModelChange,
	onBuiltinProviderManagedBaseUrlChange,
	onLlmProviderKindChange,
	onLlmProviderFieldChange,
	onToggleLlmModelMenu,
	onFetchLlmProviderModels,
	onLlmModelOptionClick,
	onLlmModelInputKeyDown,
	onOpenUrl,
	bindLlmFieldRef,
	buildLlmModelPickerId,
	getLlmProviderKind,
	getLlmProviderKindLabel,
	getLlmProviderUsageBadges,
	getLlmProviderUsageDescription,
	summarizeLlmProviderProfile,
	providerCanHandleOcr,
	providerIsLlmModel,
	providerHasResponsesModel,
	getLlmProviderModelPlaceholder,
}: LlmSettingsSectionProps) {
	function handleProviderCatalogKeyDown(
		event: ReactKeyboardEvent<HTMLButtonElement>,
		providerId: string,
	) {
		const navigationKeys = ["ArrowDown", "ArrowRight", "ArrowUp", "ArrowLeft", "Home", "End"];
		if (!navigationKeys.includes(event.key)) {
			return;
		}

		const radioGroup = event.currentTarget.closest('[role="radiogroup"]');
		if (!(radioGroup instanceof HTMLElement)) {
			return;
		}

		const radioButtons = Array.from(
			radioGroup.querySelectorAll<HTMLButtonElement>('[role="radio"]'),
		);
		const currentIndex = radioButtons.findIndex(
			(button) => button.dataset.providerId === providerId,
		);
		if (currentIndex < 0) {
			return;
		}

		event.preventDefault();
		let nextIndex = currentIndex;
		if (event.key === "Home") {
			nextIndex = 0;
		} else if (event.key === "End") {
			nextIndex = radioButtons.length - 1;
		} else if (event.key === "ArrowDown" || event.key === "ArrowRight") {
			nextIndex = (currentIndex + 1) % radioButtons.length;
		} else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
			nextIndex = (currentIndex - 1 + radioButtons.length) % radioButtons.length;
		}

		const nextButton = radioButtons[nextIndex];
		const nextProviderId = nextButton?.dataset.providerId;
		if (!nextButton || !nextProviderId) {
			return;
		}

		onSelectLlmProvider(nextProviderId);
		requestAnimationFrame(() => {
			nextButton.focus();
		});
	}

	function handleLlmModelOptionKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>) {
		const navigationKeys = ["ArrowDown", "ArrowUp", "Home", "End", "Escape"];
		if (!navigationKeys.includes(event.key)) {
			return;
		}

		const listbox = event.currentTarget.closest('[role="listbox"]');
		if (!(listbox instanceof HTMLElement)) {
			return;
		}

		const options = Array.from(listbox.querySelectorAll<HTMLButtonElement>('[role="option"]'));
		const currentIndex = options.indexOf(event.currentTarget);
		if (currentIndex < 0) {
			return;
		}

		if (event.key === "Escape") {
			event.preventDefault();
			if (selectedLlmProvider) {
				void onToggleLlmModelMenu(selectedLlmProvider, "model");
			}
			document.getElementById("llm-provider-model")?.focus();
			return;
		}

		event.preventDefault();
		let nextIndex = currentIndex;
		if (event.key === "Home") {
			nextIndex = 0;
		} else if (event.key === "End") {
			nextIndex = options.length - 1;
		} else if (event.key === "ArrowDown") {
			nextIndex = (currentIndex + 1) % options.length;
		} else if (event.key === "ArrowUp") {
			nextIndex = (currentIndex - 1 + options.length) % options.length;
		}

		options[nextIndex]?.focus();
	}

	const selectedProviderIssues = selectedLlmProvider
		? (llmValidation.providerIssues[selectedLlmProvider.id] ?? [])
		: [];
	const selectedProviderIssueCount = selectedProviderIssues.length;
	const selectedProviderUsageBadges = selectedLlmProvider
		? getLlmProviderUsageBadges(selectedLlmProvider)
		: [];
	const selectedProviderUsageDescription = selectedLlmProvider
		? getLlmProviderUsageDescription(selectedLlmProvider)
		: "";
	const selectedModelPickerId = selectedLlmProvider
		? buildLlmModelPickerId(selectedLlmProvider.id, "model")
		: null;
	const isSelectedModelPickerOpen =
		selectedModelPickerId !== null && openLlmModelPickerId === selectedModelPickerId;
	const modelSourceTitle = selectedLlmProviderIsBuiltin
		? `模板白名单 · ${selectedBuiltinLlmSelectableModels.length} 个可选模型`
		: selectedLlmProviderModels.length > 0
			? `远端目录 · 已缓存 ${selectedLlmProviderModels.length} 个模型`
			: "手动填写或拉取远端目录";
	const modelSourceDescription = selectedLlmProviderIsBuiltin
		? (selectedBuiltinLlmTemplateModel?.summary ??
			"模型只能从模板白名单里选；协议和能力跟随目录模型。")
		: isLoadingSelectedLlmProviderModels
			? "正在从当前 Base URL 拉取 /models。完成后会直接展开目录。"
			: selectedLlmProviderModelsError
				? selectedLlmProviderModelsError
				: selectedLlmProviderModels.length > 0
					? "右侧按钮会展开已缓存目录；如果服务目录里没有你要的模型，仍可直接手填。"
					: selectedLlmProvider?.baseUrl.trim()
						? "当前还没有远端目录缓存。点右侧“拉取”请求 /models；如果服务不要求鉴权，API Key 可以留空。"
						: "先补全 Base URL；之后可以拉取 /models 辅助选择，也可以直接手填模型名。";
	const templateSummary = selectedLlmProviderIsBuiltin
		? `${selectedBuiltinLlmTemplate?.displayName ?? "当前模板"} 会锁定模型白名单和协议边界；切回手动条目后会保留当前字段值，但不再受目录约束。`
		: "模板只负责填默认接入点、协议和白名单，不会保存你的 API Key。";
	const providerKindSummary = selectedLlmProviderIsBuiltin
		? "模板条目的配置类型跟随目录模型；要改协议或条目类别，先切回手动条目。"
		: "切换配置类型会立即改写协议和能力边界，并影响 OCR / RAG 的可用范围。";
	const saveStatusLabel = llmHasUnsavedChanges ? "草稿未保存" : "已写入配置";
	const saveStatusDetail =
		selectedProviderIssueCount > 0
			? `当前条目有 ${selectedProviderIssueCount} 个字段问题。先修完再保存。`
			: llmHasUnsavedChanges
				? "当前改动只保留在设置草稿里，还没写回本地配置。"
				: "当前条目已同步到本地配置文件。";
	const modelToggleButtonText = isLoadingSelectedLlmProviderModels
		? "加载"
		: selectedLlmProviderIsBuiltin
			? "白名单"
			: selectedLlmProviderModels.length > 0
				? "目录"
				: "拉取";
	const modePanelStatusLabel = selectedLlmProviderIsBuiltin ? "模板" : "手动";
	const connectionPanelStatusLabel = selectedLlmProvider?.baseUrl.trim() ? "已填接入点" : "待补全";
	const modelPanelStatusLabel = selectedLlmProvider?.model.trim() ? "已选模型" : "待选择";

	return (
		<section
			aria-labelledby={getSettingsTabId("llm")}
			className="settings-section"
			id={getSettingsPanelId("llm")}
			role="tabpanel"
			tabIndex={0}
		>
			{llmSettings.providers.length === 0 ? (
				<div className="settings-editor-card settings-llm-empty-state">
					<div className="settings-acp-sidebar-header">
						<div className="settings-acp-sidebar-copy">
							<span className="settings-section-kicker">开始配置</span>
							<h3 className="settings-subsection-title">还没有 LLM 条目</h3>
							<span className="settings-help-text settings-help-text-tight">
								先新增一个普通 LLM 条目。翻译、文档问答和 OCR 都会引用这里的条目；RAG 索引则使用
								Embedding 条目。
							</span>
						</div>
						<button className="settings-button" onClick={onAddLlmProvider} type="button">
							新增条目
						</button>
					</div>
					<div className="settings-llm-empty-steps">
						<div className="settings-llm-empty-step">
							<strong>1. 先选配置类型</strong>
							<span className="settings-agent-meta">
								普通 LLM 用于翻译、问答和 OCR；Embedding 用于 RAG 建索引。
							</span>
						</div>
						<div className="settings-llm-empty-step">
							<strong>2. 再填接入信息</strong>
							<span className="settings-agent-meta">
								填写名称、Base URL、API Key 和模型名；需要时再套内置模板。
							</span>
						</div>
					</div>
				</div>
			) : (
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
									先选条目，再在下方直接编辑连接信息和模型。模板只负责填默认值，不会替你保存密钥。
								</span>
							</div>
							<button
								className="settings-button settings-button-compact"
								onClick={onAddLlmProvider}
								type="button"
							>
								新增
							</button>
						</div>

						<div aria-label="LLM 条目目录" className="settings-llm-provider-grid" role="radiogroup">
							{llmSettings.providers.map((provider) => {
								const issueCount = llmValidation.providerIssues[provider.id]?.length ?? 0;
								const isSelected = provider.id === selectedLlmProviderId;
								return (
									<article
										className={`settings-llm-provider-card ${isSelected ? "settings-llm-provider-card-selected" : ""} ${issueCount > 0 ? "settings-llm-provider-card-invalid" : ""}`}
										key={provider.id}
									>
										<button
											aria-checked={isSelected}
											data-provider-id={provider.id}
											className="settings-llm-provider-card-main"
											onKeyDown={(event) => handleProviderCatalogKeyDown(event, provider.id)}
											onClick={() => onSelectLlmProvider(provider.id)}
											role="radio"
											tabIndex={isSelected ? 0 : -1}
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
					</aside>

					<div
						className="settings-acp-detail settings-llm-detail"
						id="llm-editor"
						ref={bindSectionBlockRef("llm-editor")}
					>
						{selectedLlmProvider ? (
							<>
								<div className="settings-rag-toolbar settings-llm-toolbar">
									<div className="settings-llm-toolbar-primary">
										<div className="settings-acp-detail-copy">
											<span className="settings-section-kicker">当前编辑</span>
											<strong className="settings-agent-name">
												{selectedLlmProvider.name.trim() || "未命名条目"}
											</strong>
										</div>
										<div aria-live="polite" className="settings-llm-toolbar-state-row">
											<span
												className={`settings-llm-toolbar-state-badge ${
													llmHasUnsavedChanges ? "settings-llm-toolbar-state-badge-dirty" : ""
												}`}
											>
												{saveStatusLabel}
											</span>
											<span className="settings-rag-toolbar-note">{saveStatusDetail}</span>
										</div>
									</div>
									<div className="settings-rag-toolbar-actions settings-llm-toolbar-actions">
										{selectedProviderIssueCount > 0 ? (
											<button
												className="settings-text-link"
												onClick={onLocateFirstLlmIssue}
												type="button"
											>
												定位当前条目的第一个问题
											</button>
										) : null}
										{llmHasUnsavedChanges ? (
											<button
												className="settings-text-link"
												disabled={savingLlm}
												onClick={onDiscardLlmDraft}
												type="button"
											>
												恢复已保存版本
											</button>
										) : null}
										<button
											className="settings-text-link settings-text-link-danger"
											onClick={() => onRemoveLlmProvider(selectedLlmProvider.id)}
											type="button"
										>
											删除条目
										</button>
										{llmHasUnsavedChanges ? (
											<button
												className="settings-button"
												disabled={savingLlm}
												onClick={() => void onSaveLlm()}
												type="button"
											>
												{savingLlm ? "保存中..." : "保存 LLM 配置"}
											</button>
										) : null}
									</div>
								</div>

								<div className="settings-llm-editor-grid">
									<div className="settings-llm-editor-main">
										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-acp-detail-copy">
													<span className="settings-section-kicker">接入模式</span>
													<div className="settings-task-card-title-row">
														<strong className="settings-agent-name">模板与配置类型</strong>
														<span className="settings-status-chip">{modePanelStatusLabel}</span>
													</div>
													<span className="settings-help-text settings-help-text-tight">
														先决定这条目是否跟随内置模板，再决定它是普通 LLM 还是 Embedding。
													</span>
												</div>
											</div>

											<div className="settings-item settings-item-stacked settings-llm-mode-field">
												<label
													className="settings-label settings-label-stacked"
													htmlFor="llm-provider-template"
												>
													<span>使用模板</span>
												</label>
												<select
													className="settings-select"
													disabled={savingLlm}
													id="llm-provider-template"
													onChange={(event) =>
														onLlmProviderTemplateChange(selectedLlmProvider.id, event.target.value)
													}
													value={selectedLlmProvider.builtinPresetId ?? ""}
												>
													<option value="">不使用模板</option>
													{builtinLlmTemplates.map((template) => (
														<option key={template.id} value={template.id}>
															{template.displayName}
														</option>
													))}
												</select>
												<span className="settings-help-text settings-help-text-tight">
													{templateSummary}
												</span>
												{selectedBuiltinLlmTemplate ? (
													<div className="settings-llm-mode-links">
														<button
															className="settings-text-link"
															onClick={() => onOpenUrl(selectedBuiltinLlmTemplate.registrationUrl)}
															type="button"
														>
															注册 / 登录
														</button>
														<button
															className="settings-text-link"
															onClick={() => onOpenUrl(selectedBuiltinLlmTemplate.apiKeyUrl)}
															type="button"
														>
															API Key 页面
														</button>
														<button
															className="settings-text-link"
															onClick={() => onOpenUrl(selectedBuiltinLlmTemplate.docsUrl)}
															type="button"
														>
															官方文档
														</button>
													</div>
												) : null}
											</div>

											<div className="settings-item settings-item-stacked settings-llm-mode-field">
												<label
													className="settings-label settings-label-stacked"
													htmlFor="llm-provider-kind"
												>
													<span>配置类型</span>
												</label>
												<select
													className="settings-select"
													disabled={savingLlm || selectedLlmProviderIsBuiltin}
													id="llm-provider-kind"
													onChange={(event) =>
														onLlmProviderKindChange(
															selectedLlmProvider.id,
															event.target.value as LlmProviderKind,
														)
													}
													value={selectedLlmProviderKind ?? "embedding"}
												>
													<option value="llm_responses_stateless">LLM · responses stateless</option>
													<option value="llm_responses_stateful">LLM · responses stateful</option>
													<option value="llm_chat_completions">LLM · chat/completions</option>
													<option value="embedding">Embedding</option>
												</select>
												<span className="settings-help-text settings-help-text-tight">
													{providerKindSummary}
												</span>
											</div>
										</div>

										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-acp-detail-copy">
													<span className="settings-section-kicker">基础连接</span>
													<div className="settings-task-card-title-row">
														<strong className="settings-agent-name">名称、接入点与密钥</strong>
														<span className="settings-status-chip">
															{connectionPanelStatusLabel}
														</span>
													</div>
													<span className="settings-help-text settings-help-text-tight">
														这里只决定这条目如何连接服务。翻译和问答真正选用哪个条目，仍在 AI
														功能页设置。
													</span>
												</div>
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
														onLlmProviderFieldChange(
															selectedLlmProvider.id,
															"name",
															event.target.value,
														)
													}
													ref={bindLlmFieldRef(selectedLlmProvider.id, "name")}
													type="text"
													value={selectedLlmProvider.name}
												/>
												{renderFieldError(
													buildFieldIssueId("llm", "name", selectedLlmProvider.id),
													selectedLlmFieldIssues.name,
												)}
											</div>

											<div className="settings-item settings-item-stacked settings-item-wide">
												<label
													className="settings-label settings-label-stacked"
													htmlFor="llm-provider-base-url"
												>
													<span>API Base URL</span>
												</label>
												<input
													aria-describedby={joinDescribedByIds(
														selectedLlmFieldIssues.baseUrl
															? buildFieldIssueId("llm", "base-url", selectedLlmProvider.id)
															: undefined,
													)}
													aria-invalid={selectedLlmFieldIssues.baseUrl ? true : undefined}
													className="settings-input settings-input-wide settings-input-mono"
													disabled={
														savingLlm ||
														(selectedLlmProviderIsBuiltin && selectedLlmProvider.managedBaseUrl)
													}
													id="llm-provider-base-url"
													onChange={(event) =>
														onLlmProviderFieldChange(
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
												{selectedLlmProviderIsBuiltin ? (
													<span className="settings-help-text settings-help-text-tight">
														{selectedLlmProvider.managedBaseUrl ? (
															<>
																当前使用模板默认接入点。
																<button
																	className="settings-text-link"
																	onClick={() =>
																		onBuiltinProviderManagedBaseUrlChange(
																			selectedLlmProvider.id,
																			false,
																		)
																	}
																	type="button"
																>
																	改为自定义接入点
																</button>
															</>
														) : (
															<>
																当前已脱离模板默认接入点管理。
																<button
																	className="settings-text-link"
																	onClick={() =>
																		onBuiltinProviderManagedBaseUrlChange(
																			selectedLlmProvider.id,
																			true,
																		)
																	}
																	type="button"
																>
																	恢复模板默认接入点
																</button>
															</>
														)}
													</span>
												) : null}
												{renderFieldError(
													buildFieldIssueId("llm", "base-url", selectedLlmProvider.id),
													selectedLlmFieldIssues.baseUrl,
												)}
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
														onLlmProviderFieldChange(
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
										</div>

										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-acp-detail-copy">
													<span className="settings-section-kicker">模型</span>
													<div className="settings-task-card-title-row">
														<strong className="settings-agent-name">模型来源与模型名</strong>
														<span className="settings-status-chip">{modelPanelStatusLabel}</span>
													</div>
													<span className="settings-help-text settings-help-text-tight">
														模板条目只允许从白名单里选模型；手动条目则可拉远端目录，或直接手填。
													</span>
												</div>
											</div>

											<div
												className="settings-item settings-item-stacked settings-item-wide"
												ref={llmModelMenuRef}
											>
												<label
													className="settings-label settings-label-stacked"
													htmlFor="llm-provider-model"
													id={`llm-provider-model-label-${selectedLlmProvider.id}`}
												>
													<span>模型名</span>
												</label>
												<div className="settings-llm-model-source">
													<strong className="settings-agent-name">{modelSourceTitle}</strong>
													<span className="settings-agent-meta">{modelSourceDescription}</span>
												</div>
												<div className="settings-llm-model-picker">
													<div className="settings-llm-model-input-row">
														<input
															aria-autocomplete="list"
															aria-controls={
																isSelectedModelPickerOpen
																	? `llm-provider-model-menu-${selectedLlmProvider.id}`
																	: undefined
															}
															aria-describedby={joinDescribedByIds(
																selectedLlmFieldIssues.model
																	? buildFieldIssueId("llm", "model", selectedLlmProvider.id)
																	: undefined,
																"llm-provider-model-help",
															)}
															aria-expanded={isSelectedModelPickerOpen}
															aria-haspopup="listbox"
															aria-invalid={selectedLlmFieldIssues.model ? true : undefined}
															aria-labelledby={`llm-provider-model-label-${selectedLlmProvider.id}`}
															className="settings-input settings-input-wide settings-input-mono"
															disabled={savingLlm}
															id="llm-provider-model"
															onChange={
																selectedLlmProviderIsBuiltin
																	? undefined
																	: (event) =>
																			onLlmProviderFieldChange(
																				selectedLlmProvider.id,
																				"model",
																				event.target.value,
																			)
															}
															onKeyDown={(event) =>
																onLlmModelInputKeyDown(event, selectedLlmProvider, "model")
															}
															placeholder={getLlmProviderModelPlaceholder(selectedLlmProvider)}
															readOnly={selectedLlmProviderIsBuiltin}
															ref={bindLlmFieldRef(selectedLlmProvider.id, "model")}
															role="combobox"
															type="text"
															value={selectedLlmProvider.model}
														/>
														<button
															aria-controls={`llm-provider-model-menu-${selectedLlmProvider.id}`}
															aria-expanded={isSelectedModelPickerOpen}
															aria-haspopup="listbox"
															aria-label={
																selectedLlmProviderIsBuiltin
																	? "展开模板白名单模型"
																	: selectedLlmProviderModels.length > 0
																		? "展开模型列表"
																		: "通过 /models 拉取模型列表"
															}
															className="settings-button settings-llm-model-toggle"
															disabled={
																savingLlm ||
																(!selectedLlmProviderIsBuiltin &&
																	!selectedLlmProvider.baseUrl.trim())
															}
															onClick={() =>
																void onToggleLlmModelMenu(selectedLlmProvider, "model")
															}
															type="button"
														>
															{modelToggleButtonText}
														</button>
													</div>
													{isSelectedModelPickerOpen ? (
														<div
															aria-labelledby={`llm-provider-model-label-${selectedLlmProvider.id}`}
															className="settings-combobox-panel settings-llm-model-panel"
															id={`llm-provider-model-menu-${selectedLlmProvider.id}`}
															role="listbox"
														>
															<div className="settings-llm-model-panel-header">
																<span className="settings-combobox-option-meta">
																	{selectedLlmProviderIsBuiltin
																		? `模板白名单 · ${selectedBuiltinLlmSelectableModels.length} 个可选模型`
																		: `远端目录 · 已拉取 ${selectedLlmProviderModels.length} 个模型`}
																</span>
																{selectedLlmProviderIsBuiltin ? null : (
																	<button
																		className="settings-button settings-llm-model-refresh"
																		disabled={savingLlm || isLoadingSelectedLlmProviderModels}
																		onClick={() =>
																			void onFetchLlmProviderModels(selectedLlmProvider, {
																				openField: "model",
																			})
																		}
																		type="button"
																	>
																		重新拉取
																	</button>
																)}
															</div>
															{selectedLlmProviderIsBuiltin ? (
																selectedBuiltinLlmSelectableModels.length > 0 ? (
																	selectedBuiltinLlmSelectableModels.map((model) => (
																		<button
																			aria-selected={
																				selectedLlmProvider.builtinPresetModelId === model.id
																			}
																			className={`settings-combobox-option ${
																				selectedLlmProvider.builtinPresetModelId === model.id
																					? "settings-combobox-option-active"
																					: ""
																			}`}
																			id={`llm-provider-model-option-${selectedLlmProvider.id}-${model.id}`}
																			key={model.id}
																			onClick={() =>
																				onBuiltinTemplateModelChange(
																					selectedLlmProvider.id,
																					model.id,
																				)
																			}
																			onKeyDown={handleLlmModelOptionKeyDown}
																			role="option"
																			tabIndex={-1}
																			type="button"
																		>
																			<span className="settings-combobox-option-label">
																				{model.displayName}
																			</span>
																			<span className="settings-combobox-option-meta">
																				{model.summary}
																			</span>
																		</button>
																	))
																) : (
																	<div className="settings-llm-model-panel-empty">
																		当前模板没有可在 Wabity 中使用的模型。
																	</div>
																)
															) : selectedLlmProviderModels.length > 0 ? (
																selectedLlmProviderModels.map((model) => (
																	<button
																		aria-selected={selectedLlmProvider.model === model.id}
																		className={`settings-combobox-option ${
																			selectedLlmProvider.model === model.id
																				? "settings-combobox-option-active"
																				: ""
																		}`}
																		id={`llm-provider-model-option-${selectedLlmProvider.id}-${model.id}`}
																		key={model.id}
																		onClick={() =>
																			onLlmModelOptionClick(selectedLlmProvider.id, "model", model)
																		}
																		onKeyDown={handleLlmModelOptionKeyDown}
																		role="option"
																		tabIndex={-1}
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
																))
															) : (
																<div className="settings-llm-model-panel-empty">
																	当前服务没有返回可用模型目录。你仍可以手填模型名。
																</div>
															)}
														</div>
													) : null}
												</div>
												<span
													className="settings-help-text settings-help-text-tight"
													id="llm-provider-model-help"
												>
													{selectedLlmProviderIsBuiltin
														? "模板条目只能从白名单里选模型；右侧按钮会展开目录。"
														: selectedLlmProviderModels.length > 0
															? "已缓存远端目录；右侧按钮会直接展开。"
															: "当前还没缓存远端目录；需要时再点右侧按钮拉取。"}
												</span>
												{renderFieldError(
													buildFieldIssueId("llm", "model", selectedLlmProvider.id),
													selectedLlmFieldIssues.model,
												)}
											</div>
										</div>

										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-acp-detail-copy">
													<span className="settings-section-kicker">用途与能力</span>
													<div className="settings-task-card-title-row">
														<strong className="settings-agent-name">当前可用范围</strong>
														<span className="settings-status-chip">
															{providerIsLlmModel(selectedLlmProvider) ? "LLM" : "Embedding"}
														</span>
													</div>
													<span className="settings-help-text settings-help-text-tight">
														这里只说明这条目能进入哪些功能列表，以及保存后可能影响的范围。
													</span>
												</div>
											</div>

											<div className="settings-item settings-item-stacked settings-item-wide">
												<div className="settings-llm-capability-strip">
													<div className="settings-llm-capability-summary">
														{selectedProviderUsageBadges.length > 0 ? (
															<div className="settings-llm-toolbar-facts">
																{selectedProviderUsageBadges.map((badge) => (
																	<span className="settings-status-chip" key={badge}>
																		{badge}
																	</span>
																))}
															</div>
														) : null}
														<span className="settings-agent-meta">
															{selectedProviderUsageDescription}
														</span>
													</div>
													{providerIsLlmModel(selectedLlmProvider) &&
													selectedLlmProviderKind !== "llm_chat_completions" ? (
														<label className="settings-llm-capability-toggle">
															<div className="settings-llm-capability-copy">
																<strong>OCR 多模态</strong>
																<span className="settings-agent-meta">
																	启用后这个条目才会进入 OCR 可选列表。
																</span>
															</div>
															<input
																checked={selectedLlmProvider.supportsMultimodal}
																className="settings-toggle"
																disabled={
																	savingLlm || !providerHasResponsesModel(selectedLlmProvider)
																}
																onChange={(event) =>
																	onLlmProviderFieldChange(
																		selectedLlmProvider.id,
																		"supportsMultimodal",
																		event.target.checked,
																	)
																}
																type="checkbox"
															/>
														</label>
													) : null}
												</div>
											</div>
										</div>
									</div>
								</div>

								{renderValidationIssueBox(selectedProviderIssues)}
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
			)}
		</section>
	);
}
