import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
	LlmEditableFieldKey,
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
	builtinLlmTemplates: BuiltinLlmProviderTemplate[];
	onAddLlmProvider: () => void;
	onAddLlmProviderModel: (providerId: string) => void;
	onSelectLlmProvider: (providerId: string) => void;
	onSelectLlmProviderModel: (providerId: string, modelId: string) => void;
	onRemoveLlmProviderModel: (providerId: string, modelId: string) => void;
	onRemoveLlmProvider: (providerId: string) => void;
	onLocateFirstLlmIssue: () => void;
	onDiscardLlmDraft: () => void;
	onSaveLlm: () => Promise<void>;
	onLlmProviderTemplateChange: (providerId: string, templateId: string) => void;
	onBuiltinProviderManagedBaseUrlChange: (providerId: string, managed: boolean) => void;
	onLlmProviderKindChange: (providerId: string, kind: LlmProviderKind) => void;
	onLlmProviderFieldChange: (
		providerId: string,
		key: LlmEditableFieldKey,
		value: string | boolean,
	) => void;
	onToggleLlmModelMenu: (
		provider: LlmProviderConfig,
		fieldKey: LlmModelFieldKey,
		options?: {
			focusOptionTarget?: "selected" | "first" | "last";
			returnFocusTarget?: "input" | "button";
		},
	) => Promise<void>;
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
		provider: Pick<LlmProviderConfig, "modelConfig" | "models" | "protocol">,
		modelId?: string | null,
	) => LlmProviderKind;
	getLlmProviderKindLabel: (kind: LlmProviderKind) => string;
	summarizeLlmProviderProfile: (provider: LlmProviderConfig) => string;
	providerCanHandleOcr: (
		provider: Pick<LlmProviderConfig, "modelConfig" | "models" | "protocol">,
		modelId?: string | null,
	) => boolean;
	providerIsLlmModel: (
		provider: Pick<LlmProviderConfig, "modelConfig" | "models">,
		modelId?: string | null,
	) => boolean;
	providerHasResponsesModel: (
		provider: Pick<LlmProviderConfig, "modelConfig" | "models" | "protocol">,
		modelId?: string | null,
	) => boolean;
	getLlmProviderModelPlaceholder: (
		provider: Pick<LlmProviderConfig, "modelConfig" | "models">,
		modelId?: string | null,
	) => string;
}

export function LlmSettingsSection({
	bindSectionBlockRef,
	llmSettings,
	selectedLlmProviderId,
	selectedLlmProvider,
	selectedLlmProviderIsBuiltin,
	selectedBuiltinLlmTemplate,
	selectedBuiltinLlmTemplateModel,
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
	onAddLlmProviderModel,
	onSelectLlmProvider,
	onSelectLlmProviderModel,
	onRemoveLlmProviderModel,
	onRemoveLlmProvider,
	onLocateFirstLlmIssue,
	onDiscardLlmDraft,
	onSaveLlm,
	onLlmProviderTemplateChange,
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
	summarizeLlmProviderProfile,
	providerCanHandleOcr,
	providerIsLlmModel,
	providerHasResponsesModel,
	getLlmProviderModelPlaceholder,
}: LlmSettingsSectionProps) {
	const [llmModelFilter, setLlmModelFilter] = useState("");
	const [confirmingDeleteId, setConfirmingDeleteId] = useState<string | null>(null);

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

		const panel = event.currentTarget.closest("[data-llm-model-panel]");
		if (!(panel instanceof HTMLElement)) {
			return;
		}

		const options = Array.from(
			panel.querySelectorAll<HTMLButtonElement>("[data-llm-model-option]"),
		);
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

	function handleProviderModelKeyDown(
		event: ReactKeyboardEvent<HTMLButtonElement>,
		modelId: string,
	) {
		const navigationKeys = ["ArrowDown", "ArrowRight", "ArrowUp", "ArrowLeft", "Home", "End"];
		if (!navigationKeys.includes(event.key) || !selectedLlmProvider) {
			return;
		}

		const radioGroup = event.currentTarget.closest('[role="radiogroup"]');
		if (!(radioGroup instanceof HTMLElement)) {
			return;
		}

		const radioButtons = Array.from(
			radioGroup.querySelectorAll<HTMLButtonElement>('[role="radio"]'),
		);
		const currentIndex = radioButtons.findIndex((button) => button.dataset.modelId === modelId);
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
		const nextModelId = nextButton?.dataset.modelId;
		if (!nextButton || !nextModelId) {
			return;
		}

		onSelectLlmProviderModel(selectedLlmProvider.id, nextModelId);
		requestAnimationFrame(() => {
			nextButton.focus();
		});
	}

	const pendingModelFocusRef = useRef<{
		providerId: string;
		target: "selected" | "first" | "last";
	} | null>(null);

	const focusRenderedLlmModelOption = useCallback(
		(providerId: string, target: "selected" | "first" | "last" = "selected") => {
			pendingModelFocusRef.current = { providerId, target };
		},
		[],
	);

	useEffect(() => {
		const pending = pendingModelFocusRef.current;
		if (!pending) {
			return;
		}

		const panel = document.getElementById(`llm-provider-model-menu-${pending.providerId}`);
		if (!(panel instanceof HTMLElement)) {
			return;
		}

		function tryFocus() {
			const options = Array.from(
				panel!.querySelectorAll<HTMLButtonElement>("[data-llm-model-option]"),
			);
			if (options.length === 0) {
				return false;
			}

			pendingModelFocusRef.current = null;

			if (pending!.target === "first") {
				options[0]?.focus();
				return true;
			}

			if (pending!.target === "last") {
				options[options.length - 1]?.focus();
				return true;
			}

			const selectedOption =
				options.find((option) => option.getAttribute("aria-selected") === "true") ?? options[0];
			selectedOption?.focus();
			return true;
		}

		if (tryFocus()) {
			return;
		}

		const observer = new MutationObserver(() => {
			if (tryFocus()) {
				observer.disconnect();
			}
		});
		observer.observe(panel, { childList: true, subtree: true });
		return () => observer.disconnect();
	}, [openLlmModelPickerId]);

	function formatLlmProviderCatalogEndpoint(baseUrl: string) {
		const trimmed = baseUrl.trim();
		if (!trimmed) {
			return "未设置 Base URL";
		}

		try {
			const parsed = new URL(trimmed);
			const normalizedPath = parsed.pathname.replace(/\/+$/, "");
			return `${parsed.host}${normalizedPath}`.replace(/\/$/, "") || parsed.host;
		} catch {
			return trimmed;
		}
	}

	function getProviderSelectedModel(provider: Pick<LlmProviderConfig, "modelConfig" | "models">) {
		return (
			provider.models.find((model) => model.id === provider.modelConfig?.id) ??
			provider.models[0] ??
			provider.modelConfig ??
			null
		);
	}

	function toDomIdPart(value: string) {
		return value.replace(/[^a-zA-Z0-9_-]/g, "_");
	}

	function buildLlmModelOptionId(providerId: string, source: "remote" | "custom", modelId: string) {
		return `llm-provider-model-option-${source}-${toDomIdPart(providerId)}-${toDomIdPart(modelId)}`;
	}

	const selectedProviderIssues = selectedLlmProvider
		? (llmValidation.providerIssues[selectedLlmProvider.id] ?? [])
		: [];
	const selectedProviderModel = selectedLlmProvider
		? getProviderSelectedModel(selectedLlmProvider)
		: null;
	const selectedProviderModelId = selectedProviderModel?.id ?? null;
	const selectedProviderIssueCount = selectedProviderIssues.length;
	const selectedModelPickerId = selectedLlmProvider
		? buildLlmModelPickerId(selectedLlmProvider.id, "model")
		: null;
	const selectedModelMenuId = selectedLlmProvider
		? `llm-provider-model-menu-${selectedLlmProvider.id}`
		: undefined;
	const selectedModelLabelId = selectedLlmProvider
		? `llm-provider-model-label-${selectedLlmProvider.id}`
		: undefined;
	const isSelectedModelPickerOpen =
		selectedModelPickerId !== null && openLlmModelPickerId === selectedModelPickerId;
	const templateSupportsModelListing = selectedBuiltinLlmTemplate?.supportsModelListing ?? false;
	const modelSourceTitle =
		selectedLlmProviderModels.length > 0
			? `已缓存 ${selectedLlmProviderModels.length} 个远端模型`
			: selectedLlmProviderIsBuiltin && !templateSupportsModelListing
				? "当前预设不提供远端目录"
				: "可手填，也可拉取远端目录";
	const modelSourceDescription = selectedLlmProviderIsBuiltin
		? selectedLlmProviderModelsError
			? selectedLlmProviderModelsError
			: isLoadingSelectedLlmProviderModels
				? "正在拉取 /models……"
				: templateSupportsModelListing
					? "以当前服务返回的 /models 为准，也可以直接手填。"
					: "直接手填模型名。"
		: isLoadingSelectedLlmProviderModels
			? "正在拉取 /models……"
			: selectedLlmProviderModelsError
				? selectedLlmProviderModelsError
				: selectedLlmProviderModels.length > 0
					? "可展开已缓存目录，也可直接手填。"
					: selectedLlmProvider?.baseUrl.trim()
						? "点「拉取」读取 /models。"
						: "先填 Base URL，再拉取或手填模型名。";
	const templateSummary = selectedLlmProviderIsBuiltin
		? `${selectedBuiltinLlmTemplate?.displayName ?? "预设"} 提供官方入口和默认 Base URL。`
		: "供应商预设只填入口和默认 Base URL，不保存 API Key。";
	const providerKindSummary = selectedBuiltinLlmTemplateModel
		? `命中 ${selectedBuiltinLlmTemplate?.displayName ?? "预设"} 元数据，调用方式已自动回填。`
		: "决定翻译、问答、OCR 和知识库 Embedding 是否能选用这个模型。";
	const saveStatusLabel = llmHasUnsavedChanges ? "草稿未保存" : "已写入配置";
	const saveStatusDetail =
		selectedProviderIssueCount > 0 ? `${selectedProviderIssueCount} 个问题待修复` : "";
	const selectedProviderHasModel = Boolean(selectedProviderModel?.model.trim().length);
	const selectedProviderOcrReady = selectedLlmProvider
		? providerCanHandleOcr(selectedLlmProvider, selectedProviderModelId)
		: false;
	const modelToggleButtonText = isLoadingSelectedLlmProviderModels
		? "加载"
		: selectedLlmProviderIsBuiltin
			? selectedLlmProviderModels.length > 0
				? "目录"
				: templateSupportsModelListing
					? "拉取"
					: "手填"
			: selectedLlmProviderModels.length > 0
				? "目录"
				: "拉取";
	const normalizedLlmModelFilter = llmModelFilter.trim().toLowerCase();
	const filteredLlmProviderModels = useMemo(() => {
		if (!normalizedLlmModelFilter) {
			return selectedLlmProviderModels;
		}

		return selectedLlmProviderModels.filter((model) =>
			model.id.toLowerCase().includes(normalizedLlmModelFilter),
		);
	}, [normalizedLlmModelFilter, selectedLlmProviderModels]);
	const hasVisibleLlmModelOptions = filteredLlmProviderModels.length > 0;
	const selectedModelControlsId =
		isSelectedModelPickerOpen && hasVisibleLlmModelOptions ? selectedModelMenuId : undefined;
	const llmModelPanelSummary =
		filteredLlmProviderModels.length === selectedLlmProviderModels.length
			? `远端目录 · 已拉取 ${selectedLlmProviderModels.length} 个模型`
			: `远端目录 · 显示 ${filteredLlmProviderModels.length} / ${selectedLlmProviderModels.length} 个模型`;
	const llmModelPanelEmptyMessage = selectedLlmProviderIsBuiltin
		? selectedLlmProviderModels.length > 0
			? "没有匹配当前筛选词的模型。改个关键词再试。"
			: templateSupportsModelListing
				? "当前还没有远端目录。点上方按钮拉取，或直接手填模型名。"
				: "当前模板不提供远端目录，请直接手填模型名。"
		: selectedLlmProviderModels.length > 0
			? "没有匹配当前筛选词的模型。改个关键词再试。"
			: "当前服务没有返回可用模型目录。你仍可以手填模型名。";
	const isLlmModel = selectedLlmProvider
		? providerIsLlmModel(selectedLlmProvider, selectedProviderModelId)
		: false;
	const ocrAvailable = isLlmModel && selectedLlmProviderKind !== "llm_chat_completions";

	function getAvailabilityToneForModel(
		hasModelName: boolean,
		available: boolean,
	): "success" | "info" | "warn" {
		if (!available) {
			return "warn";
		}

		return hasModelName ? "success" : "info";
	}

	const selectedProviderAvailabilityItems = selectedLlmProvider
		? [
				{
					key: "translation",
					label: "翻译",
					tone: getAvailabilityToneForModel(selectedProviderHasModel, isLlmModel),
				},
				{
					key: "question-answer",
					label: "文档问答",
					tone: getAvailabilityToneForModel(selectedProviderHasModel, isLlmModel),
				},
				{
					key: "ocr",
					label: "OCR",
					tone: getAvailabilityToneForModel(
						selectedProviderHasModel,
						ocrAvailable && selectedProviderOcrReady,
					),
				},
				{
					key: "rag-embedding",
					label: "知识库 Embedding",
					tone: getAvailabilityToneForModel(selectedProviderHasModel, !isLlmModel),
				},
			]
		: [];

	function getModelAvailabilityItems(modelId: string, hasModelName: boolean) {
		if (!selectedLlmProvider) {
			return [];
		}

		const modelIsLlm = providerIsLlmModel(selectedLlmProvider, modelId);
		const modelKind = getLlmProviderKind(selectedLlmProvider, modelId);
		const modelCanHandleOcr =
			modelIsLlm &&
			modelKind !== "llm_chat_completions" &&
			providerCanHandleOcr(selectedLlmProvider, modelId);
		return [
			{
				key: "translation",
				label: "翻译",
				tone: getAvailabilityToneForModel(hasModelName, modelIsLlm),
			},
			{
				key: "question-answer",
				label: "问答",
				tone: getAvailabilityToneForModel(hasModelName, modelIsLlm),
			},
			{
				key: "ocr",
				label: "OCR",
				tone: getAvailabilityToneForModel(hasModelName, modelCanHandleOcr),
			},
			{
				key: "rag-embedding",
				label: "知识库",
				tone: getAvailabilityToneForModel(hasModelName, !modelIsLlm),
			},
		];
	}
	useEffect(() => {
		setLlmModelFilter("");
		setConfirmingDeleteId(null);
	}, [selectedLlmProvider?.id, isSelectedModelPickerOpen]);

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
							<h3 className="settings-subsection-title">还没有 Provider 组</h3>
							<span className="settings-help-text settings-help-text-tight">
								先新增一个 Provider 组。翻译、文档问答、OCR 和知识库都会引用这里的具体模型。
							</span>
						</div>
						<button className="settings-button" onClick={onAddLlmProvider} type="button">
							新增 Provider
						</button>
					</div>
					<div className="settings-llm-empty-steps">
						<div className="settings-llm-empty-step">
							<strong>1. 先选模板或调用方式</strong>
							<span className="settings-agent-meta">
								已知模板模型会自动回填协议和能力；未知模型再手动指定类型。
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
						aria-labelledby="llm-catalog-title"
						className="settings-acp-sidebar settings-acp-sidebar-secondary settings-llm-sidebar"
						id="llm-catalog"
						ref={bindSectionBlockRef("llm-catalog")}
					>
						<div className="settings-acp-sidebar-header">
							<div className="settings-acp-sidebar-copy">
								<h3 className="settings-subsection-title" id="llm-catalog-title">
									Provider 组
								</h3>
							</div>
							<button
								className="settings-button settings-button-compact"
								onClick={onAddLlmProvider}
								type="button"
							>
								新增
							</button>
						</div>

						<div
							aria-label="Provider 组目录"
							className="settings-llm-provider-grid"
							role="radiogroup"
						>
							{llmSettings.providers.map((provider) => {
								const issueCount = llmValidation.providerIssues[provider.id]?.length ?? 0;
								const isSelected = provider.id === selectedLlmProviderId;
								const providerSelectedModel = getProviderSelectedModel(provider);
								const providerSelectedModelId = providerSelectedModel?.id ?? null;
								const providerKindLabel = getLlmProviderKindLabel(
									getLlmProviderKind(provider, providerSelectedModelId),
								);
								const providerModelLabel = providerSelectedModel?.model.trim() || "未配置模型";
								const providerEndpointLabel = formatLlmProviderCatalogEndpoint(provider.baseUrl);
								return (
									<div
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
												<strong className="settings-agent-name">
													{provider.name.trim() || "未命名 Provider 组"}
												</strong>
												<div className="settings-llm-provider-card-badges">
													{provider.models.some(
														(model) => llmSettings.translationModelId === model.id,
													) ? (
														<span className="settings-status-chip settings-status-chip-strong">
															翻译
														</span>
													) : null}
													{provider.models.some(
														(model) => llmSettings.questionAnswerModelId === model.id,
													) ? (
														<span className="settings-status-chip settings-status-chip-strong">
															问答
														</span>
													) : null}
													{provider.models.some((model) =>
														providerCanHandleOcr(provider, model.id),
													) ? (
														<span className="settings-status-chip">多模态</span>
													) : null}
													{issueCount > 0 ? (
														<span className="settings-llm-provider-card-issue-badge">
															{issueCount} 个问题
														</span>
													) : null}
												</div>
											</div>

											<div className="settings-llm-provider-card-summary">
												<span className="settings-agent-meta">
													{providerKindLabel} · {providerModelLabel}
												</span>
												<span className="settings-agent-meta">
													{providerEndpointLabel} · {summarizeLlmProviderProfile(provider)}
												</span>
											</div>
										</button>
									</div>
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
										<div
											aria-live="polite"
											className="settings-rag-toolbar-heading settings-llm-toolbar-heading"
										>
											<strong className="settings-agent-name">
												{selectedLlmProvider.name.trim() || "未命名条目"}
											</strong>
											<span
												className={`settings-llm-toolbar-state-badge ${
													llmHasUnsavedChanges ? "settings-llm-toolbar-state-badge-dirty" : ""
												}`}
											>
												{saveStatusLabel}
											</span>
										</div>
										{saveStatusDetail ? (
											<span className="settings-rag-toolbar-note">{saveStatusDetail}</span>
										) : null}
									</div>
									<div className="settings-rag-toolbar-actions settings-llm-toolbar-actions">
										{selectedProviderIssueCount > 0 ? (
											<button
												className="settings-text-link settings-text-link-action"
												onClick={onLocateFirstLlmIssue}
												type="button"
											>
												定位第一个问题
											</button>
										) : null}
										{llmHasUnsavedChanges ? (
											<button
												className="settings-text-link settings-text-link-action"
												disabled={savingLlm}
												onClick={onDiscardLlmDraft}
												type="button"
											>
												恢复已保存
											</button>
										) : null}
										{confirmingDeleteId === selectedLlmProvider.id ? (
											<>
												<button
													className="settings-text-link settings-text-link-action settings-text-link-danger"
													onClick={() => {
														onRemoveLlmProvider(selectedLlmProvider.id);
														setConfirmingDeleteId(null);
													}}
													type="button"
												>
													确认删除
												</button>
												<button
													className="settings-text-link settings-text-link-action"
													onClick={() => setConfirmingDeleteId(null)}
													type="button"
												>
													取消
												</button>
											</>
										) : (
											<button
												className="settings-text-link settings-text-link-action settings-text-link-danger"
												onClick={() => setConfirmingDeleteId(selectedLlmProvider.id)}
												type="button"
											>
												删除条目
											</button>
										)}
										{llmHasUnsavedChanges ? (
											<button
												className="settings-button"
												disabled={savingLlm}
												onClick={() => void onSaveLlm()}
												type="button"
											>
												{savingLlm ? "保存中..." : "保存配置"}
											</button>
										) : null}
									</div>
								</div>

								{renderValidationIssueBox(selectedProviderIssues)}

								<div className="settings-llm-editor-grid">
									<div className="settings-llm-editor-main">
										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-acp-detail-copy">
													<div className="settings-task-card-title-row">
														<strong className="settings-agent-name">连接信息</strong>
														<span className="settings-status-chip">
															{selectedLlmProviderIsBuiltin ? "使用预设" : "手动接入"}
														</span>
													</div>
												</div>
											</div>
											<div className="settings-agent-fields settings-llm-panel-fields">
												<div className="settings-field-row settings-llm-form-row settings-llm-field-wide">
													<label className="settings-field-label" htmlFor="llm-provider-template">
														供应商预设
													</label>
													<select
														className="settings-select"
														disabled={savingLlm}
														id="llm-provider-template"
														onChange={(event) =>
															onLlmProviderTemplateChange(
																selectedLlmProvider.id,
																event.target.value,
															)
														}
														value={selectedLlmProvider.builtinPresetId ?? ""}
													>
														<option value="">不使用预设</option>
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
																className="settings-text-link settings-text-link-action settings-text-link-action-quiet settings-llm-mode-link"
																onClick={() =>
																	onOpenUrl(selectedBuiltinLlmTemplate.registrationUrl)
																}
																type="button"
															>
																{selectedBuiltinLlmTemplate.registrationLabel}
															</button>
															<button
																className="settings-text-link settings-text-link-action settings-text-link-action-quiet settings-llm-mode-link"
																onClick={() => onOpenUrl(selectedBuiltinLlmTemplate.apiKeyUrl)}
																type="button"
															>
																{selectedBuiltinLlmTemplate.apiKeyLabel}
															</button>
															<button
																className="settings-text-link settings-text-link-action settings-text-link-action-quiet settings-llm-mode-link"
																onClick={() => onOpenUrl(selectedBuiltinLlmTemplate.docsUrl)}
																type="button"
															>
																{selectedBuiltinLlmTemplate.docsLabel}
															</button>
														</div>
													) : null}
												</div>

												<div className="settings-field-row settings-llm-form-row">
													<label className="settings-field-label" htmlFor="llm-provider-name">
														名称
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

												<div className="settings-field-row settings-llm-form-row settings-llm-field-wide">
													<label className="settings-field-label" htmlFor="llm-provider-base-url">
														API Base URL
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
																	当前跟随预设默认接入点。
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
																		改为自定义
																	</button>
																</>
															) : (
																<>
																	当前使用自定义接入点。
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
																		恢复预设
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

												<div className="settings-field-row settings-llm-form-row">
													<label className="settings-field-label" htmlFor="llm-provider-api-key">
														API Key
													</label>
													<input
														aria-describedby="llm-provider-api-key-help"
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
													<span
														className="settings-help-text settings-help-text-tight"
														id="llm-provider-api-key-help"
													>
														只保存在本地配置；本地网关可以留空。
													</span>
												</div>
											</div>
										</div>

										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-acp-detail-copy">
													<div className="settings-task-card-title-row">
														<strong className="settings-agent-name">模型与用途</strong>
														<span className="settings-status-chip">
															{selectedLlmProvider.models.length} 个模型
														</span>
													</div>
												</div>
											</div>
											<div className="settings-agent-fields settings-llm-panel-fields">
												<div className="settings-llm-model-switcher">
													<div className="settings-llm-model-switcher-header">
														<div className="settings-llm-model-switcher-copy">
															<span className="settings-agent-meta">
																组内模型会作为翻译、问答、OCR 或知识库的可选条目。
															</span>
														</div>
														<div className="settings-inline-actions settings-llm-model-actions">
															<button
																className="settings-button settings-agent-secondary"
																disabled={savingLlm}
																onClick={() => onAddLlmProviderModel(selectedLlmProvider.id)}
																type="button"
															>
																新增模型
															</button>
															{selectedLlmProvider.models.length > 1 && selectedProviderModelId ? (
																<button
																	className="settings-text-link settings-text-link-action settings-text-link-action-quiet settings-text-link-danger"
																	disabled={savingLlm}
																	onClick={() =>
																		onRemoveLlmProviderModel(
																			selectedLlmProvider.id,
																			selectedProviderModelId,
																		)
																	}
																	type="button"
																>
																	删除当前模型
																</button>
															) : null}
														</div>
													</div>
													<div
														aria-label="Provider 组内模型"
														className="settings-llm-model-list"
														role="radiogroup"
													>
														{selectedLlmProvider.models.map((model) => {
															const modelName = model.model.trim();
															const modelKind = getLlmProviderKind(selectedLlmProvider, model.id);
															const modelAvailabilityItems = getModelAvailabilityItems(
																model.id,
																Boolean(modelName),
															);
															return (
																<button
																	aria-checked={model.id === selectedProviderModelId}
																	className={`settings-llm-model-row ${
																		model.id === selectedProviderModelId
																			? "settings-llm-model-row-selected"
																			: ""
																	}`}
																	data-model-id={model.id}
																	key={model.id}
																	onClick={() =>
																		onSelectLlmProviderModel(selectedLlmProvider.id, model.id)
																	}
																	onKeyDown={(event) => handleProviderModelKeyDown(event, model.id)}
																	role="radio"
																	tabIndex={model.id === selectedProviderModelId ? 0 : -1}
																	type="button"
																>
																	<span className="settings-llm-model-row-main">
																		<span className="settings-llm-model-row-name">
																			{modelName || "未命名模型"}
																		</span>
																		<span className="settings-llm-model-row-meta">
																			{modelName ? model.id : "模型名未填写"} ·{" "}
																			{getLlmProviderKindLabel(modelKind)}
																		</span>
																	</span>
																	<span className="settings-llm-model-row-badges">
																		{model.id === selectedProviderModelId ? (
																			<span className="settings-status-chip">当前</span>
																		) : null}
																		{modelAvailabilityItems
																			.filter((item) => item.tone !== "warn")
																			.map((item) => (
																				<span
																					className={`settings-status-chip settings-status-chip-${item.tone}`}
																					key={item.key}
																				>
																					{item.label}
																				</span>
																			))}
																	</span>
																</button>
															);
														})}
													</div>
												</div>
												<div className="settings-llm-model-source">
													<strong className="settings-agent-name">{modelSourceTitle}</strong>
													<span className="settings-agent-meta">{modelSourceDescription}</span>
												</div>
												<div className="settings-field-row settings-llm-form-row settings-llm-model-field">
													<label
														className="settings-field-label"
														htmlFor="llm-provider-model"
														id={selectedModelLabelId}
													>
														模型名
													</label>
													<div className="settings-llm-model-picker" ref={llmModelMenuRef}>
														<div className="settings-llm-model-input-row">
															<input
																aria-autocomplete="list"
																aria-controls={selectedModelControlsId}
																aria-describedby={joinDescribedByIds(
																	selectedLlmFieldIssues.model
																		? buildFieldIssueId("llm", "model", selectedLlmProvider.id)
																		: undefined,
																	"llm-provider-model-help",
																)}
																aria-expanded={isSelectedModelPickerOpen}
																aria-invalid={selectedLlmFieldIssues.model ? true : undefined}
																aria-labelledby={selectedModelLabelId}
																className="settings-input settings-input-wide settings-input-mono"
																disabled={savingLlm}
																id="llm-provider-model"
																onChange={(event) =>
																	onLlmProviderFieldChange(
																		selectedLlmProvider.id,
																		"model",
																		event.target.value,
																	)
																}
																onKeyDown={(event) =>
																	onLlmModelInputKeyDown(event, selectedLlmProvider, "model")
																}
																placeholder={getLlmProviderModelPlaceholder(
																	selectedLlmProvider,
																	selectedProviderModelId,
																)}
																ref={bindLlmFieldRef(selectedLlmProvider.id, "model")}
																role="combobox"
																type="text"
																value={selectedProviderModel?.model ?? ""}
															/>
															<button
																aria-controls={selectedModelControlsId}
																aria-expanded={isSelectedModelPickerOpen}
																aria-haspopup="listbox"
																aria-label={
																	selectedLlmProviderIsBuiltin
																		? selectedLlmProviderModels.length > 0
																			? "展开远端模型列表"
																			: templateSupportsModelListing
																				? "拉取远端模型列表"
																				: "当前模板不提供远端目录，请手填模型名"
																		: selectedLlmProviderModels.length > 0
																			? "展开模型列表"
																			: "通过 /models 拉取模型列表"
																}
																className="settings-button settings-button-subtle settings-llm-model-toggle"
																disabled={
																	savingLlm ||
																	((!selectedLlmProviderIsBuiltin ||
																		templateSupportsModelListing) &&
																		!selectedLlmProvider.baseUrl.trim())
																}
																id={`llm-provider-model-toggle-${selectedLlmProvider.id}`}
																onClick={() =>
																	void onToggleLlmModelMenu(selectedLlmProvider, "model", {
																		focusOptionTarget: "selected",
																		returnFocusTarget: "button",
																	}).then(() =>
																		focusRenderedLlmModelOption(selectedLlmProvider.id, "selected"),
																	)
																}
																type="button"
															>
																{modelToggleButtonText}
															</button>
														</div>
														<span
															className="settings-help-text settings-help-text-tight"
															id="llm-provider-model-help"
														>
															{selectedLlmProviderIsBuiltin
																? templateSupportsModelListing
																	? "模板不再提供预置模型；请拉取远端目录或直接手填。"
																	: "当前模板不提供远端目录；请直接手填模型名。"
																: selectedLlmProviderModels.length > 0
																	? "已缓存远端目录，可直接展开。"
																	: "可手填或点右侧按钮拉取远端目录。"}
														</span>
														{renderFieldError(
															buildFieldIssueId("llm", "model", selectedLlmProvider.id),
															selectedLlmFieldIssues.model,
														)}
														{isSelectedModelPickerOpen ? (
															<div
																className="settings-combobox-panel settings-llm-model-panel"
																data-llm-model-panel
															>
																<div className="settings-llm-model-panel-header">
																	<div className="settings-llm-model-panel-header-copy">
																		<span className="settings-combobox-option-meta">
																			{llmModelPanelSummary}
																		</span>
																		{selectedLlmProviderModels.length > 8 ? (
																			<input
																				aria-label="筛选远端模型"
																				className="settings-input settings-input-mono settings-llm-model-filter"
																				onChange={(event) => setLlmModelFilter(event.target.value)}
																				placeholder="筛选模型名"
																				type="text"
																				value={llmModelFilter}
																			/>
																		) : null}
																	</div>
																	{selectedLlmProviderIsBuiltin &&
																	!templateSupportsModelListing ? null : (
																		<button
																			className="settings-button settings-button-subtle settings-llm-model-refresh"
																			disabled={savingLlm || isLoadingSelectedLlmProviderModels}
																			onClick={() =>
																				void onFetchLlmProviderModels(selectedLlmProvider, {
																					openField: "model",
																				})
																			}
																			type="button"
																		>
																			{selectedLlmProviderIsBuiltin ? "拉取目录" : "重新拉取"}
																		</button>
																	)}
																</div>
																{hasVisibleLlmModelOptions ? (
																	<div
																		aria-labelledby={selectedModelLabelId}
																		className="settings-llm-model-options"
																		id={selectedModelMenuId}
																		role="listbox"
																	>
																		{filteredLlmProviderModels.map((model) => {
																			const isCurrentModel =
																				selectedProviderModel?.model === model.id;
																			const modelOptionSource = selectedLlmProviderIsBuiltin
																				? "remote"
																				: "custom";
																			return (
																				<button
																					aria-selected={isCurrentModel}
																					className={`settings-combobox-option settings-llm-model-option ${
																						isCurrentModel ? "settings-combobox-option-active" : ""
																					}`}
																					data-llm-model-option
																					id={buildLlmModelOptionId(
																						selectedLlmProvider.id,
																						modelOptionSource,
																						model.id,
																					)}
																					key={`${modelOptionSource}-${model.id}`}
																					onClick={() =>
																						onLlmModelOptionClick(
																							selectedLlmProvider.id,
																							"model",
																							model,
																						)
																					}
																					onKeyDown={handleLlmModelOptionKeyDown}
																					onMouseDown={(event) => event.preventDefault()}
																					role="option"
																					tabIndex={-1}
																					type="button"
																				>
																					<span className="settings-combobox-option-label">
																						{model.id}
																					</span>
																					<span className="settings-combobox-option-meta">
																						{isCurrentModel
																							? "当前已填入输入框"
																							: selectedLlmProviderIsBuiltin
																								? "来自当前服务目录"
																								: "点击填入输入框"}
																					</span>
																				</button>
																			);
																		})}
																	</div>
																) : (
																	<div className="settings-llm-model-panel-empty" role="status">
																		{llmModelPanelEmptyMessage}
																	</div>
																)}
															</div>
														) : null}
													</div>
												</div>
												<div className="settings-field-row settings-llm-form-row">
													<label className="settings-field-label" htmlFor="llm-provider-kind">
														调用方式
													</label>
													<select
														className="settings-select"
														disabled={savingLlm || Boolean(selectedBuiltinLlmTemplateModel)}
														id="llm-provider-kind"
														onChange={(event) =>
															onLlmProviderKindChange(
																selectedLlmProvider.id,
																event.target.value as LlmProviderKind,
															)
														}
														value={selectedLlmProviderKind ?? "embedding"}
													>
														<option value="llm_responses_stateless">
															LLM · responses stateless
														</option>
														<option value="llm_responses_stateful">LLM · responses stateful</option>
														<option value="llm_chat_completions">LLM · chat/completions</option>
														<option value="embedding">Embedding</option>
													</select>
													<span className="settings-help-text settings-help-text-tight">
														{providerKindSummary}
													</span>
												</div>
												{providerIsLlmModel(selectedLlmProvider, selectedProviderModelId) &&
												selectedLlmProviderKind !== "llm_chat_completions" ? (
													<label className="settings-llm-capability-toggle">
														<div className="settings-llm-capability-copy">
															<strong>OCR 多模态</strong>
															<span className="settings-agent-meta">
																{selectedProviderOcrReady
																	? "已开启，可被 OCR 选用。"
																	: "开启后可被 OCR 选用。"}
															</span>
														</div>
														<input
															checked={selectedProviderModel?.supportsMultimodal ?? false}
															className="settings-toggle"
															disabled={
																savingLlm ||
																!providerHasResponsesModel(
																	selectedLlmProvider,
																	selectedProviderModelId,
																)
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
												<div className="settings-llm-availability-badges">
													<span className="settings-agent-meta">可用于</span>
													{selectedProviderAvailabilityItems.map((item) => (
														<span
															className={`settings-status-chip settings-status-chip-${item.tone}`}
															key={item.key}
														>
															{item.label}
														</span>
													))}
												</div>
											</div>
										</div>
									</div>
								</div>
							</>
						) : (
							<div className="settings-empty-panel">
								<strong className="settings-empty-title">没有可编辑的 Provider 组</strong>
								<span className="settings-help-text settings-help-text-tight">
									左侧新增一个条目后，先选模板或调用方式，再补全名称、Base URL 和模型名。
								</span>
							</div>
						)}
					</div>
				</div>
			)}
		</section>
	);
}
