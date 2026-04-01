import { useEffect, useMemo, useState } from "react";
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
	onBuiltinProviderManagedBaseUrlChange: (providerId: string, managed: boolean) => void;
	onLlmProviderKindChange: (providerId: string, kind: LlmProviderKind) => void;
	onLlmProviderFieldChange: <K extends keyof LlmProviderConfig>(
		providerId: string,
		key: K,
		value: LlmProviderConfig[K],
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
		provider: Pick<LlmProviderConfig, "modelType" | "protocol" | "supportsStateful">,
	) => LlmProviderKind;
	getLlmProviderKindLabel: (kind: LlmProviderKind) => string;
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

	function focusRenderedLlmModelOption(
		providerId: string,
		target: "selected" | "first" | "last" = "selected",
	) {
		requestAnimationFrame(() => {
			requestAnimationFrame(() => {
				const listbox = document.getElementById(`llm-provider-model-menu-${providerId}`);
				if (!(listbox instanceof HTMLElement)) {
					return;
				}

				const options = Array.from(listbox.querySelectorAll<HTMLButtonElement>('[role="option"]'));
				if (options.length === 0) {
					return;
				}

				if (target === "first") {
					options[0]?.focus();
					return;
				}

				if (target === "last") {
					options[options.length - 1]?.focus();
					return;
				}

				const selectedOption =
					options.find((option) => option.getAttribute("aria-selected") === "true") ?? options[0];
				selectedOption?.focus();
			});
		});
	}

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

	const selectedProviderIssues = selectedLlmProvider
		? (llmValidation.providerIssues[selectedLlmProvider.id] ?? [])
		: [];
	const selectedProviderIssueCount = selectedProviderIssues.length;
	const selectedModelPickerId = selectedLlmProvider
		? buildLlmModelPickerId(selectedLlmProvider.id, "model")
		: null;
	const isSelectedModelPickerOpen =
		selectedModelPickerId !== null && openLlmModelPickerId === selectedModelPickerId;
	const templateSupportsModelListing = selectedBuiltinLlmTemplate?.supportsModelListing ?? false;
	const modelSourceTitle = selectedLlmProviderIsBuiltin
		? selectedLlmProviderModels.length > 0
			? `远端目录 · 已缓存 ${selectedLlmProviderModels.length} 个模型`
			: templateSupportsModelListing
				? "模板已挂载，等待拉取远端目录"
				: "模板已挂载，当前只能手动填写"
		: selectedLlmProviderModels.length > 0
			? `远端目录 · 已缓存 ${selectedLlmProviderModels.length} 个模型`
			: "手动填写或拉取远端目录";
	const modelSourceDescription = selectedLlmProviderIsBuiltin
		? selectedLlmProviderModelsError
			? selectedLlmProviderModelsError
			: isLoadingSelectedLlmProviderModels
				? "正在拉取 /models……"
				: templateSupportsModelListing
					? "模板不再提供预置模型；请以当前服务实际返回的目录为准，也可直接手填。"
					: "当前模板不提供远端模型目录；请直接手填模型名。"
		: isLoadingSelectedLlmProviderModels
			? "正在拉取 /models……"
			: selectedLlmProviderModelsError
				? selectedLlmProviderModelsError
				: selectedLlmProviderModels.length > 0
					? "可展开已缓存目录，也可直接手填。"
					: selectedLlmProvider?.baseUrl.trim()
						? "点「拉取」获取远端模型目录。"
						: "先填 Base URL，再拉取或手填模型名。";
	const templateSummary = selectedLlmProviderIsBuiltin
		? `${selectedBuiltinLlmTemplate?.displayName ?? "模板"} 只负责提供官方入口和默认接入点；模型请以实际拉取结果或手填为准。`
		: "模板只做供应商入口与接入点预填，不托管 API Key，也不再预置模型。";
	const providerKindSummary = selectedBuiltinLlmTemplateModel
		? `当前模型命中 ${selectedBuiltinLlmTemplate?.displayName ?? "模板"} 目录，协议和能力按模型元数据自动回填；改模型后才可手动切换。`
		: "影响协议、能力边界和 OCR/RAG 可用范围。已知模板模型会按模型元数据自动回填这一层。";
	const saveStatusLabel = llmHasUnsavedChanges ? "草稿未保存" : "已写入配置";
	const saveStatusDetail =
		selectedProviderIssueCount > 0 ? `${selectedProviderIssueCount} 个问题待修复` : "";
	const selectedProviderHasModel = selectedLlmProvider?.model.trim().length ? true : false;
	const selectedProviderProtocolLabel = selectedLlmProviderKind
		? getLlmProviderKindLabel(selectedLlmProviderKind)
		: "";
	const selectedProviderOcrReady = selectedLlmProvider
		? providerCanHandleOcr(selectedLlmProvider)
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
	const modePanelStatusLabel = selectedLlmProviderIsBuiltin ? "模板已挂载" : "手动";
	const connectionPanelStatusLabel = selectedLlmProvider?.baseUrl.trim() ? "已填接入点" : "待补全";
	const modelPanelStatusLabel = selectedLlmProvider?.model.trim() ? "已选模型" : "待选择";
	const normalizedLlmModelFilter = llmModelFilter.trim().toLowerCase();
	const filteredLlmProviderModels = useMemo(() => {
		if (!normalizedLlmModelFilter) {
			return selectedLlmProviderModels;
		}

		return selectedLlmProviderModels.filter((model) =>
			model.id.toLowerCase().includes(normalizedLlmModelFilter),
		);
	}, [normalizedLlmModelFilter, selectedLlmProviderModels]);
	const llmModelPanelSummary =
		filteredLlmProviderModels.length === selectedLlmProviderModels.length
			? `远端目录 · 已拉取 ${selectedLlmProviderModels.length} 个模型`
			: `远端目录 · 显示 ${filteredLlmProviderModels.length} / ${selectedLlmProviderModels.length} 个模型`;
	const selectedProviderRouteSummary = selectedLlmProvider
		? providerIsLlmModel(selectedLlmProvider)
			? selectedLlmProviderKind === "llm_chat_completions"
				? selectedProviderHasModel
					? "这是普通 LLM 条目。它会出现在翻译和文档问答的模型列表里，但不会进入 OCR 列表。"
					: "这是普通 LLM 条目。补全模型名后，它才会出现在翻译和文档问答的模型列表里；OCR 仍不可用。"
				: selectedLlmProviderKind === "llm_responses_stateful"
					? selectedProviderHasModel
						? "这是普通 LLM 条目。它会走 responses stateful，并可供翻译和文档问答复用；OCR 资格取决于下方多模态开关。"
						: "这是普通 LLM 条目。补全模型名后，它会按 responses stateful 进入翻译和文档问答列表；OCR 资格再由下方多模态开关决定。"
					: selectedProviderHasModel
						? "这是普通 LLM 条目。它会走 responses stateless，并可供翻译和文档问答复用；OCR 资格取决于下方多模态开关。"
						: "这是普通 LLM 条目。补全模型名后，它会按 responses stateless 进入翻译和文档问答列表；OCR 资格再由下方多模态开关决定。"
			: selectedProviderHasModel
				? "这是 Embedding 条目，只会出现在 RAG 的 Embedding 列表里，不会进入翻译、文档问答或 OCR。"
				: "这是 Embedding 条目。补全模型名后，它才会出现在 RAG 的 Embedding 列表里；不会进入翻译、文档问答或 OCR。"
		: "";
	const selectedProviderAvailabilityItems = selectedLlmProvider
		? [
				{
					key: "translation",
					label: "翻译",
					status: providerIsLlmModel(selectedLlmProvider)
						? selectedProviderHasModel
							? "可选"
							: "待补全"
						: "不可用",
					statusTone: providerIsLlmModel(selectedLlmProvider)
						? selectedProviderHasModel
							? "success"
							: "info"
						: "warn",
					description: providerIsLlmModel(selectedLlmProvider)
						? selectedProviderHasModel
							? "会出现在 AI 功能页的翻译模型下拉里。"
							: "先补全模型名，之后才会出现在 AI 功能页的翻译模型下拉里。"
						: "Embedding 条目不会出现在翻译模型下拉里。",
				},
				{
					key: "question-answer",
					label: "文档问答",
					status: providerIsLlmModel(selectedLlmProvider)
						? selectedProviderHasModel
							? "可选"
							: "待补全"
						: "不可用",
					statusTone: providerIsLlmModel(selectedLlmProvider)
						? selectedProviderHasModel
							? "success"
							: "info"
						: "warn",
					description: providerIsLlmModel(selectedLlmProvider)
						? selectedProviderHasModel
							? "会出现在 AI 功能页的文档问答模型下拉里。"
							: "先补全模型名，之后才会出现在 AI 功能页的文档问答模型下拉里。"
						: "Embedding 条目不会出现在文档问答模型下拉里。",
				},
				{
					key: "ocr",
					label: "OCR",
					status: !providerIsLlmModel(selectedLlmProvider)
						? "不可用"
						: selectedLlmProviderKind === "llm_chat_completions"
							? "不可用"
							: !selectedProviderHasModel
								? "待补全"
								: selectedProviderOcrReady
									? "可选"
									: "待开启",
					statusTone:
						!providerIsLlmModel(selectedLlmProvider) ||
						selectedLlmProviderKind === "llm_chat_completions"
							? "warn"
							: !selectedProviderHasModel || !selectedProviderOcrReady
								? "info"
								: "success",
					description: !providerIsLlmModel(selectedLlmProvider)
						? "Embedding 条目不会出现在通用页的 OCR 模型下拉里。"
						: selectedLlmProviderKind === "llm_chat_completions"
							? "OCR 只接受 responses 协议；chat/completions 条目不会进入 OCR 列表。"
							: !selectedProviderHasModel
								? "先补全模型名，再决定是否加入通用页的 OCR 模型下拉。"
								: selectedProviderOcrReady
									? "已经满足条件，会出现在通用页的 OCR 模型下拉里。"
									: "打开下方“OCR 多模态资格”后，才会出现在通用页的 OCR 模型下拉里。",
				},
				{
					key: "rag-embedding",
					label: "RAG Embedding",
					status: providerIsLlmModel(selectedLlmProvider)
						? "不可用"
						: selectedProviderHasModel
							? "可选"
							: "待补全",
					statusTone: providerIsLlmModel(selectedLlmProvider)
						? "warn"
						: selectedProviderHasModel
							? "success"
							: "info",
					description: providerIsLlmModel(selectedLlmProvider)
						? "只有 Embedding 条目会出现在 RAG 页的 Embedding 列表里。"
						: selectedProviderHasModel
							? "会出现在 RAG 页的 Embedding 条目列表里。"
							: "先补全模型名，之后才会出现在 RAG 页的 Embedding 条目列表里。",
				},
			]
		: [];
	const ocrToggleDescription = selectedProviderOcrReady
		? "已开启。这个条目现在会出现在通用页的 OCR 模型下拉里。"
		: selectedProviderHasModel
			? "打开后，这个条目才会出现在通用页的 OCR 模型下拉里。"
			: "先补全模型名，才能决定是否加入通用页的 OCR 模型下拉。";

	useEffect(() => {
		setLlmModelFilter("");
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
							<h3 className="settings-subsection-title">还没有模型条目</h3>
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
						className="settings-acp-sidebar settings-acp-sidebar-secondary settings-llm-sidebar"
						id="llm-catalog"
						ref={bindSectionBlockRef("llm-catalog")}
					>
						<div className="settings-acp-sidebar-header">
							<div className="settings-acp-sidebar-copy">
								<h3 className="settings-subsection-title">模型条目</h3>
							</div>
							<button
								className="settings-button settings-button-compact"
								onClick={onAddLlmProvider}
								type="button"
							>
								新增
							</button>
						</div>

						<div aria-label="模型条目目录" className="settings-llm-provider-grid" role="radiogroup">
							{llmSettings.providers.map((provider) => {
								const issueCount = llmValidation.providerIssues[provider.id]?.length ?? 0;
								const isSelected = provider.id === selectedLlmProviderId;
								const providerKindLabel = getLlmProviderKindLabel(getLlmProviderKind(provider));
								const providerModelLabel = provider.model.trim() || "未配置模型";
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
													{provider.name.trim() || "未命名模型条目"}
												</strong>
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
										<button
											className="settings-text-link settings-text-link-action settings-text-link-danger"
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
												<div className="settings-task-card-title-row">
													<strong className="settings-agent-name">模板</strong>
													<span className="settings-status-chip">{modePanelStatusLabel}</span>
												</div>
											</div>
											<div className="settings-agent-fields settings-llm-panel-fields">
												<label className="settings-label settings-label-stacked settings-llm-form-row">
													<span>使用模板</span>
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
												</label>
											</div>
										</div>

										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-task-card-title-row">
													<strong className="settings-agent-name">名称、接入点与密钥</strong>
													<span className="settings-status-chip">{connectionPanelStatusLabel}</span>
												</div>
											</div>
											<div className="settings-agent-fields settings-llm-panel-fields">
												<label className="settings-label settings-label-stacked settings-llm-form-row">
													<span>名称</span>
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
												</label>

												<label className="settings-label settings-label-stacked settings-llm-form-row settings-llm-field-wide">
													<span>API Base URL</span>
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
																	当前跟随模板默认接入点。
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
																	当前已改为自定义接入点。
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
												</label>

												<label className="settings-label settings-label-stacked settings-llm-form-row">
													<span>API Key</span>
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
												</label>
											</div>
										</div>

										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-task-card-title-row">
													<strong className="settings-agent-name">来源与模型名</strong>
													<span className="settings-status-chip">{modelPanelStatusLabel}</span>
												</div>
											</div>
											<div className="settings-agent-fields settings-llm-panel-fields">
												<div className="settings-llm-model-source">
													<strong className="settings-agent-name">{modelSourceTitle}</strong>
													<span className="settings-agent-meta">{modelSourceDescription}</span>
												</div>
												<label
													className="settings-label settings-label-stacked settings-llm-form-row settings-llm-model-field"
													htmlFor="llm-provider-model"
													id={`llm-provider-model-label-${selectedLlmProvider.id}`}
												>
													<span>模型名</span>
													<div className="settings-llm-model-picker" ref={llmModelMenuRef}>
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
																placeholder={getLlmProviderModelPlaceholder(selectedLlmProvider)}
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
																aria-labelledby={`llm-provider-model-label-${selectedLlmProvider.id}`}
																className="settings-combobox-panel settings-llm-model-panel"
																id={`llm-provider-model-menu-${selectedLlmProvider.id}`}
																role="listbox"
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
																{selectedLlmProviderIsBuiltin ? (
																	filteredLlmProviderModels.length > 0 ? (
																		filteredLlmProviderModels.map((model) => (
																			<button
																				aria-selected={selectedLlmProvider.model === model.id}
																				className={`settings-combobox-option settings-llm-model-option ${
																					selectedLlmProvider.model === model.id
																						? "settings-combobox-option-active"
																						: ""
																				}`}
																				id={`llm-provider-model-option-remote-${selectedLlmProvider.id}-${model.id}`}
																				key={`remote-${model.id}`}
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
																					{selectedLlmProvider.model === model.id
																						? "当前已填入输入框"
																						: "来自当前服务目录"}
																				</span>
																			</button>
																		))
																	) : selectedLlmProviderModels.length > 0 ? (
																		<div className="settings-llm-model-panel-empty">
																			没有匹配当前筛选词的模型。改个关键词再试。
																		</div>
																	) : templateSupportsModelListing ? (
																		<div className="settings-llm-model-panel-empty">
																			当前还没有远端目录。点上方按钮拉取，或直接手填模型名。
																		</div>
																	) : (
																		<div className="settings-llm-model-panel-empty">
																			当前模板不提供远端目录，请直接手填模型名。
																		</div>
																	)
																) : filteredLlmProviderModels.length > 0 ? (
																	filteredLlmProviderModels.map((model) => (
																		<button
																			aria-selected={selectedLlmProvider.model === model.id}
																			className={`settings-combobox-option settings-llm-model-option ${
																				selectedLlmProvider.model === model.id
																					? "settings-combobox-option-active"
																					: ""
																			}`}
																			id={`llm-provider-model-option-${selectedLlmProvider.id}-${model.id}`}
																			key={model.id}
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
																				{selectedLlmProvider.model === model.id
																					? "当前已填入输入框"
																					: "点击填入输入框"}
																			</span>
																		</button>
																	))
																) : selectedLlmProviderModels.length > 0 ? (
																	<div className="settings-llm-model-panel-empty">
																		没有匹配当前筛选词的模型。改个关键词再试。
																	</div>
																) : (
																	<div className="settings-llm-model-panel-empty">
																		当前服务没有返回可用模型目录。你仍可以手填模型名。
																	</div>
																)}
															</div>
														) : null}
													</div>
												</label>
												<label className="settings-label settings-label-stacked settings-llm-form-row">
													<span>调用方式</span>
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
												</label>
											</div>
										</div>

										<div className="settings-rag-editor-panel settings-llm-editor-panel">
											<div className="settings-editor-card-header settings-task-card-header">
												<div className="settings-task-card-title-row">
													<strong className="settings-agent-name">会出现在这些位置</strong>
													<span className="settings-status-chip">
														{selectedProviderProtocolLabel}
													</span>
												</div>
												<span className="settings-help-text settings-help-text-tight">
													{selectedProviderRouteSummary}
												</span>
											</div>
											<div className="settings-agent-fields settings-llm-panel-fields">
												<div className="settings-llm-availability-list">
													{selectedProviderAvailabilityItems.map((item) => (
														<div className="settings-llm-availability-item" key={item.key}>
															<div className="settings-llm-availability-item-header">
																<strong>{item.label}</strong>
																<span
																	className={`settings-status-chip settings-status-chip-${item.statusTone}`}
																>
																	{item.status}
																</span>
															</div>
															<span className="settings-agent-meta">{item.description}</span>
														</div>
													))}
												</div>
												{providerIsLlmModel(selectedLlmProvider) &&
												selectedLlmProviderKind !== "llm_chat_completions" ? (
													<label className="settings-llm-capability-toggle settings-llm-availability-toggle">
														<div className="settings-llm-capability-copy">
															<strong>OCR 多模态资格</strong>
															<span className="settings-agent-meta">{ocrToggleDescription}</span>
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
							</>
						) : (
							<div className="settings-empty-panel">
								<strong className="settings-empty-title">没有可编辑的模型条目</strong>
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
