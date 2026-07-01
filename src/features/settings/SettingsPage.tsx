import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { LauncherPinButton } from "../../app/LauncherPinButton";
import {
	chooseDirectory,
	getAppSettings,
	getAcpMcpServers,
	getShortcut,
	getWorkspace,
	hideLauncherWindow,
	listBuiltinLlmProviderTemplates,
	listLlmProviderModels,
	onShortcutUpdated,
	scanRagSources,
	setShortcut,
	defaultRagIgnoreGlobs,
} from "../../lib/tauri/client";
import { browserBuiltinAgentToolStatus } from "../../lib/tauri/client/defaults";
import type {
	AppSettings,
	AppearanceSettings,
	BuiltinMcpConfig,
	BuiltinLlmProviderTemplate,
	GeneralSettings,
	LlmModelConfig,
	LlmProviderConfig,
	LlmProviderModelEntry,
	LlmSettings,
	NotificationSettings,
	OcrSettings,
	PromptsSettings,
	RagScanResult,
	RagSettings,
	ShortcutConfig,
	WorkspaceState,
} from "../../lib/tauri/types";
import { useSettingsWindowFrame } from "./useSettingsWindowFrame";
import { applyAppearanceSettings, defaultAppearanceSettings } from "../../app/appearance";
import {
	AboutSettingsSection,
	GeneralSettingsSection,
	LlmSettingsSection,
	McpSettingsSection,
	PromptsSettingsSection,
	RagSettingsSection,
} from "./SettingsSectionViews";
import {
	SettingsQuickJumpList,
	getMcpTransportMeta,
	getSettingsPanelId,
	getSettingsTabId,
	settingsSections,
} from "./settingsShared";
import {
	AcpInlineNotice,
	AcpMcpServerDraft,
	DependencyHealthItem,
	LlmEditableFieldKey,
	LlmFieldKey,
	LlmModelFieldKey,
	LlmProviderKind,
	McpFieldKey,
	McpPanelMode,
	McpTransport,
	PendingMcpFocusTarget,
	RagFieldKey,
	SavedLlmDraftState,
	SavedMcpDraftState,
	SavedRagDraftState,
	SettingsSectionId,
	defaultPromptsSettings,
} from "./settingsTypes";
import {
	applyLlmProviderKind,
	applyBuiltinTemplateModelMetadata,
	applyBuiltinTemplateToProvider,
	buildLlmDraftSnapshot,
	buildMcpDraftSnapshot,
	buildPromptsDraftSnapshot,
	buildQuestionAnswerTaskDraftSnapshot,
	buildRagDraftSnapshot,
	buildSavedLlmDraftState,
	buildSavedMcpDraftState,
	buildSavedRagDraftState,
	clearProviderModelIdentityHints,
	buildTranslationTaskDraftSnapshot,
	createDefaultAppSettings,
	createDefaultGeneralSettings,
	createDefaultLlmSettings,
	createDefaultNotificationSettings,
	createDefaultOcrSettings,
	createDefaultRagSettings,
	createDefaultShortcutSettings,
	createDefaultWorkspaceState,
	createLlmModelDraft,
	createLlmProviderDraft,
	createMcpServerDraft,
	createMcpServerDraftFromConfig,
	detachBuiltinTemplateFromProvider,
	extraRagIgnoreGlobPlaceholder,
	findBuiltinTemplate,
	findBuiltinTemplateModelByModelName,
	findFirstLlmIssue,
	findFirstMcpIssue,
	formatTextLines,
	getErrorMessage,
	getLlmProviderKind,
	getLlmProviderKindLabel,
	getLlmProviderModelPlaceholder,
	normalizeRagIgnoreGlobs,
	normalizeRecordedShortcutKey,
	parseTextLines,
	ragSupportedFileExtensions,
	providerCanHandleAiTask,
	providerCanHandleOcr,
	providerCanHandleRagEmbedding,
	providerHasResponsesModel,
	providerIsLlmModel,
	providerUsesBuiltinTemplate,
	resolveDocumentsDirectoryPath,
	reconcileLlmSettings,
	reconcileOcrSettings,
	reconcileRagSettings,
	resolveRagDirectoryPickerDefaultPath,
	selectExistingIdOrFirst,
	splitExtraRagIgnoreGlobs,
	summarizeLlmProviderProfile,
	validateLlmSettings,
	validateMcpServers,
	validateRagSettings,
} from "./settingsState";
import { useSettingsPersistence } from "./useSettingsPersistence";
import { useSettingsSectionNavigation } from "./useSettingsSectionNavigation";
import "./settings.css";

interface SettingsPageProps {
	launcherPinned?: boolean;
	onBack: () => void;
	onAppearanceChange?: (appearance: AppearanceSettings) => void;
	onLauncherPinnedChange?: (pinned: boolean) => void | Promise<void>;
}

function getBoundProviderLabel(provider: LlmProviderConfig | null) {
	return provider?.name || provider?.models[0]?.model || provider?.baseUrl || "未配置";
}

function buildModelDependencyHealthItem(
	label: string,
	selectedProvider: LlmProviderConfig | null,
	missingDetail: string,
): DependencyHealthItem {
	return selectedProvider
		? {
				status: "ok",
				label,
				detail: `当前使用 ${getBoundProviderLabel(selectedProvider)}。`,
			}
		: {
				status: "warning",
				label,
				detail: missingDetail,
			};
}

function hasEffectiveRagRebuildInputChanges(
	ragSettings: RagSettings,
	persistedRagSettings: RagSettings,
) {
	return (
		ragSettings.embeddingModelId !== persistedRagSettings.embeddingModelId ||
		JSON.stringify(ragSettings.sourceDirectories) !==
			JSON.stringify(persistedRagSettings.sourceDirectories) ||
		JSON.stringify(ragSettings.ignoreGlobs) !== JSON.stringify(persistedRagSettings.ignoreGlobs)
	);
}

export function SettingsPage({
	launcherPinned = false,
	onBack,
	onAppearanceChange,
	onLauncherPinnedChange,
}: SettingsPageProps) {
	const [activeSection, setActiveSection] = useState<SettingsSectionId>("general");
	const [translationPromptExpanded, setTranslationPromptExpanded] = useState(false);
	const [questionAnswerPromptExpanded, setQuestionAnswerPromptExpanded] = useState(false);
	const [generalSettings, setGeneralSettings] = useState<GeneralSettings>(
		createDefaultGeneralSettings,
	);
	const [notificationSettings, setNotificationSettings] = useState<NotificationSettings>(
		createDefaultNotificationSettings,
	);
	const [promptsSettings, setPromptsSettings] = useState<PromptsSettings>(defaultPromptsSettings);

	const [shortcutSettings, setShortcutSettings] = useState<ShortcutConfig>(
		createDefaultShortcutSettings,
	);
	const [llmSettings, setLlmSettings] = useState<LlmSettings>(createDefaultLlmSettings);
	const [builtinLlmTemplates, setBuiltinLlmTemplates] = useState<BuiltinLlmProviderTemplate[]>([]);
	const [selectedLlmProviderId, setSelectedLlmProviderId] = useState<string | null>(null);
	const [savedLlmSnapshot, setSavedLlmSnapshot] = useState(() =>
		buildLlmDraftSnapshot(createDefaultLlmSettings()),
	);
	const [, setSavedLlmState] = useState<SavedLlmDraftState>(() => buildSavedLlmDraftState([]));
	const [llmModelOptions, setLlmModelOptions] = useState<Record<string, LlmProviderModelEntry[]>>(
		{},
	);
	const [llmModelErrors, setLlmModelErrors] = useState<Record<string, string>>({});
	const [loadingLlmModelProviderId, setLoadingLlmModelProviderId] = useState<string | null>(null);
	const [openLlmModelPickerId, setOpenLlmModelPickerId] = useState<string | null>(null);
	const pendingLlmModelOptionFocusRef = useRef<{
		providerId: string;
		target: "selected" | "first" | "last";
	} | null>(null);
	const [ragSettings, setRagSettings] = useState<RagSettings>(createDefaultRagSettings);
	const [savedRagSnapshot, setSavedRagSnapshot] = useState(() =>
		buildRagDraftSnapshot(createDefaultRagSettings()),
	);
	const [, setSavedRagState] = useState<SavedRagDraftState>(() =>
		buildSavedRagDraftState(createDefaultRagSettings()),
	);
	const [ragScanResult, setRagScanResult] = useState<RagScanResult | null>(null);
	const [selectedMcpTransport, setSelectedMcpTransport] = useState<McpTransport>("stdio");
	const [mcpServers, setMcpServersState] = useState<AcpMcpServerDraft[]>([]);
	const [selectedMcpServerId, setSelectedMcpServerId] = useState<string | null>(null);
	const [savedMcpSnapshot, setSavedMcpSnapshot] = useState(() =>
		buildMcpDraftSnapshot([], { enabled: false, enabledModules: [] }),
	);
	const [savedMcpState, setSavedMcpState] = useState<SavedMcpDraftState>(() =>
		buildSavedMcpDraftState([], { enabled: false, enabledModules: [] }),
	);
	const [mcpNotice, setMcpNotice] = useState<AcpInlineNotice | null>(null);
	const [builtinMcpConfig, setBuiltinMcpConfig] = useState<BuiltinMcpConfig>({
		enabled: false,
		enabledModules: [],
	});
	const builtinAgentToolStatus = browserBuiltinAgentToolStatus;
	const [workspaceContext, setWorkspaceContext] = useState<WorkspaceState>(
		createDefaultWorkspaceState,
	);

	const [appearanceSettings, setAppearanceSettings] =
		useState<AppearanceSettings>(defaultAppearanceSettings);
	const [ocrSettings, setOcrSettings] = useState<OcrSettings>(createDefaultOcrSettings);
	const [persistedAppSettings, setPersistedAppSettings] =
		useState<AppSettings>(createDefaultAppSettings);

	// Track which shortcut is being edited
	const [editingShortcut, setEditingShortcut] = useState<keyof ShortcutConfig | null>(null);
	const [savingShortcutKey, setSavingShortcutKey] = useState<keyof ShortcutConfig | null>(null);
	const [scanningRag, setScanningRag] = useState(false);
	const [mcpPanelMode, setMcpPanelMode] = useState<McpPanelMode>("create");
	const [settingsError, setSettingsError] = useState<string | null>(null);
	const mcpFieldRefs = useRef<
		Record<string, HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | null>
	>({});
	const mcpListOptionRefs = useRef<Record<string, HTMLButtonElement | null>>({});
	const pendingMcpFocusRef = useRef<PendingMcpFocusTarget | null>(null);
	const llmFieldRefs = useRef<Record<string, HTMLInputElement | null>>({});
	const ragFieldRefs = useRef<Record<RagFieldKey, HTMLSelectElement | HTMLTextAreaElement | null>>({
		embeddingModelId: null,
		sourceDirectories: null,
		ignoreGlobs: null,
	});
	const llmModelMenuRef = useRef<HTMLDivElement | null>(null);
	const shellRef = useRef<HTMLElement | null>(null);
	const frameSize = useSettingsWindowFrame(shellRef);
	const showLlmOcrFields = ocrSettings.provider === "llm_ocr";
	const ragSourceDirectoryPlaceholder = resolveDocumentsDirectoryPath(workspaceContext);
	const ragDirectoryPickerDefaultPath = resolveRagDirectoryPickerDefaultPath(workspaceContext);
	const {
		activeQuickLinks,
		activeSectionBlockId,
		bindSectionBlockRef,
		contentRef,
		handleSectionTabKeyDown,
		handleSelectSection,
		scrollToSectionBlock,
		sectionTabRefs,
	} = useSettingsSectionNavigation({
		activeSection,
		setActiveSection,
	});

	function applyMcpServerDrafts(
		nextServers: AcpMcpServerDraft[],
		nextSelectedServerId: string | null,
	) {
		setMcpServersState(nextServers);
		setSelectedMcpServerId(nextSelectedServerId);
	}

	function bindMcpListOptionRef(serverId: string) {
		return (node: HTMLButtonElement | null) => {
			mcpListOptionRefs.current[serverId] = node;
		};
	}

	function selectMcpServer(serverId: string, options?: { focusOption?: boolean }) {
		setMcpPanelMode("edit");
		setSelectedMcpServerId(serverId);
		if (options?.focusOption) {
			window.requestAnimationFrame(() => {
				mcpListOptionRefs.current[serverId]?.focus();
			});
		}
	}

	function queueMcpFieldFocus(
		serverId: string,
		fieldKey: McpFieldKey,
		options?: { scrollToForm?: boolean },
	) {
		setMcpPanelMode("edit");
		pendingMcpFocusRef.current = {
			serverId,
			fieldKey,
			scrollToForm: options?.scrollToForm ?? true,
		};
		setSelectedMcpServerId(serverId);
	}

	function handleEnterMcpCreateMode() {
		setMcpPanelMode("create");
	}

	function handleReturnToCurrentMcp() {
		if (selectedMcpServerId) {
			setMcpPanelMode("edit");
		}
	}

	function handleMcpCatalogKeyDown(event: ReactKeyboardEvent<HTMLButtonElement>, serverId: string) {
		const currentIndex = regularMcpServers.findIndex((server) => server.id === serverId);
		if (currentIndex < 0) {
			return;
		}

		let nextIndex: number | null = null;
		switch (event.key) {
			case "ArrowDown":
			case "ArrowRight":
				nextIndex = (currentIndex + 1) % regularMcpServers.length;
				break;
			case "ArrowUp":
			case "ArrowLeft":
				nextIndex = (currentIndex - 1 + regularMcpServers.length) % regularMcpServers.length;
				break;
			case "Home":
				nextIndex = 0;
				break;
			case "End":
				nextIndex = regularMcpServers.length - 1;
				break;
			default:
				return;
		}

		event.preventDefault();
		const nextServerId = regularMcpServers[nextIndex]?.id;
		if (!nextServerId) {
			return;
		}

		selectMcpServer(nextServerId, { focusOption: true });
	}

	function clearLlmModelCatalog(providerId: string) {
		setLlmModelOptions((current) => {
			if (!(providerId in current)) {
				return current;
			}

			const next = { ...current };
			delete next[providerId];
			return next;
		});
		setLlmModelErrors((current) => {
			if (!(providerId in current)) {
				return current;
			}

			const next = { ...current };
			delete next[providerId];
			return next;
		});
		setLoadingLlmModelProviderId((current) => (current === providerId ? null : current));
		setOpenLlmModelPickerId((current) => (current?.startsWith(`${providerId}:`) ? null : current));
	}

	function buildLlmModelPickerId(providerId: string, fieldKey: LlmModelFieldKey) {
		return `${providerId}:${fieldKey}`;
	}

	function focusLlmModelOption(
		providerId: string,
		target: "selected" | "first" | "last" = "selected",
	) {
		requestAnimationFrame(() => {
			const panel = document.getElementById(`llm-provider-model-menu-${providerId}`);
			if (!(panel instanceof HTMLElement)) {
				return;
			}

			const options = Array.from(
				panel.querySelectorAll<HTMLButtonElement>("[data-llm-model-option]"),
			);
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
	}

	function queueLlmModelOptionFocus(providerId: string, target: "selected" | "first" | "last") {
		pendingLlmModelOptionFocusRef.current = { providerId, target };
	}

	function getSelectedProviderModel(
		provider: Pick<LlmProviderConfig, "modelConfig" | "models">,
	): LlmModelConfig {
		return (
			provider.models.find((model) => model.id === provider.modelConfig?.id) ??
			provider.models[0] ??
			provider.modelConfig
		);
	}

	function bindProviderToModel(
		provider: LlmProviderConfig,
		model: LlmModelConfig,
	): LlmProviderConfig {
		return {
			...provider,
			models: [model],
			modelConfig: model,
		};
	}

	function patchSelectedProviderModel(
		provider: LlmProviderConfig,
		update: (model: LlmModelConfig) => LlmModelConfig,
	): LlmProviderConfig {
		const currentModel = getSelectedProviderModel(provider);
		const nextModel = update(currentModel);
		return {
			...provider,
			models: provider.models.map((model) => (model.id === currentModel.id ? nextModel : model)),
			modelConfig: nextModel,
		};
	}

	function findLlmModelOption(providerId: string, modelName: string) {
		return (llmModelOptions[providerId] ?? []).find((option) => option.id === modelName) ?? null;
	}

	async function handleFetchLlmProviderModels(
		provider: LlmProviderConfig,
		options?: {
			openField?: LlmModelFieldKey;
			focusOptionTarget?: "selected" | "first" | "last";
		},
	) {
		const builtinTemplate = providerUsesBuiltinTemplate(provider)
			? findBuiltinTemplate(builtinLlmTemplates, provider.builtinPresetId)
			: null;
		if (builtinTemplate && !builtinTemplate.supportsModelListing) {
			if (options?.openField) {
				setOpenLlmModelPickerId(buildLlmModelPickerId(provider.id, options.openField));
				if (options.focusOptionTarget) {
					queueLlmModelOptionFocus(provider.id, options.focusOptionTarget);
				}
			}
			return;
		}

		setSettingsError(null);
		setLoadingLlmModelProviderId(provider.id);
		setLlmModelErrors((current) => {
			const next = { ...current };
			delete next[provider.id];
			return next;
		});

		try {
			const models = await listLlmProviderModels(provider);
			setLlmModelOptions((current) => ({
				...current,
				[provider.id]: models,
			}));
			const matchedOption =
				models.find((option) => option.id === getSelectedProviderModel(provider).model) ?? null;
			if (matchedOption) {
				setLlmSettings((current) =>
					reconcileLlmSettings({
						...current,
						providers: current.providers.map((candidate) =>
							candidate.id === provider.id
								? patchSelectedProviderModel(candidate, (model) => ({
										...model,
										modelIdentityHint: matchedOption.identityHint,
									}))
								: candidate,
						),
					}),
				);
			}
			if (options?.openField && models.length > 0) {
				setOpenLlmModelPickerId(buildLlmModelPickerId(provider.id, options.openField));
				if (options.focusOptionTarget) {
					queueLlmModelOptionFocus(provider.id, options.focusOptionTarget);
				}
			}
		} catch (error: unknown) {
			setLlmModelErrors((current) => ({
				...current,
				[provider.id]: getErrorMessage(error, "模型列表拉取失败"),
			}));
			setLlmModelOptions((current) => {
				const next = { ...current };
				delete next[provider.id];
				return next;
			});
		} finally {
			setLoadingLlmModelProviderId((current) => (current === provider.id ? null : current));
		}
	}

	function focusLlmModelControl(providerId: string, target: "input" | "button") {
		const elementId =
			target === "button" ? `llm-provider-model-toggle-${providerId}` : "llm-provider-model";
		document.getElementById(elementId)?.focus();
	}

	async function handleToggleLlmModelMenu(
		provider: LlmProviderConfig,
		fieldKey: LlmModelFieldKey,
		options?: {
			focusOptionTarget?: "selected" | "first" | "last";
			returnFocusTarget?: "input" | "button";
		},
	) {
		const pickerId = buildLlmModelPickerId(provider.id, fieldKey);
		const shouldOpen = openLlmModelPickerId !== pickerId;
		const builtinTemplate = providerUsesBuiltinTemplate(provider)
			? findBuiltinTemplate(builtinLlmTemplates, provider.builtinPresetId)
			: null;

		if (builtinTemplate && !builtinTemplate.supportsModelListing) {
			setOpenLlmModelPickerId(shouldOpen ? pickerId : null);
			if (shouldOpen && options?.focusOptionTarget) {
				queueLlmModelOptionFocus(provider.id, options.focusOptionTarget);
			}
			if (!shouldOpen && options?.returnFocusTarget) {
				pendingLlmModelOptionFocusRef.current = null;
				focusLlmModelControl(provider.id, options.returnFocusTarget);
			}
			return;
		}

		if (loadingLlmModelProviderId === provider.id || !provider.baseUrl.trim()) {
			return;
		}

		const models = llmModelOptions[provider.id] ?? [];
		if (models.length === 0) {
			await handleFetchLlmProviderModels(provider, {
				openField: fieldKey,
				focusOptionTarget: options?.focusOptionTarget,
			});
			return;
		}

		setOpenLlmModelPickerId(shouldOpen ? pickerId : null);
		if (shouldOpen && options?.focusOptionTarget) {
			queueLlmModelOptionFocus(provider.id, options.focusOptionTarget);
		}
		if (!shouldOpen && options?.returnFocusTarget) {
			pendingLlmModelOptionFocusRef.current = null;
			focusLlmModelControl(provider.id, options.returnFocusTarget);
		}
	}

	function handleLlmModelOptionClick(
		providerId: string,
		fieldKey: LlmModelFieldKey,
		option: LlmProviderModelEntry,
	) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					const nextProvider = patchSelectedProviderModel(provider, (model) => ({
						...model,
						[fieldKey]: option.id,
						modelIdentityHint: option.identityHint,
					}));
					const template = providerUsesBuiltinTemplate(nextProvider)
						? findBuiltinTemplate(builtinLlmTemplates, nextProvider.builtinPresetId)
						: null;
					return applyBuiltinTemplateModelMetadata(
						nextProvider,
						findBuiltinTemplateModelByModelName(template, option.id),
						getSelectedProviderModel(nextProvider).id,
					);
				}),
			}),
		);
		setOpenLlmModelPickerId(null);
		setSettingsError(null);
		pendingLlmModelOptionFocusRef.current = null;
		focusLlmModelControl(providerId, "input");
	}

	function handleLlmModelInputKeyDown(
		event: ReactKeyboardEvent<HTMLInputElement>,
		provider: LlmProviderConfig,
		fieldKey: LlmModelFieldKey,
	) {
		const pickerId = buildLlmModelPickerId(provider.id, fieldKey);
		if (event.key === "ArrowDown" || event.key === "ArrowUp") {
			event.preventDefault();
			const focusTarget = event.key === "ArrowUp" ? "last" : "selected";
			if (openLlmModelPickerId === pickerId) {
				focusLlmModelOption(provider.id, focusTarget);
				return;
			}

			void handleToggleLlmModelMenu(provider, fieldKey, {
				focusOptionTarget: focusTarget,
				returnFocusTarget: "input",
			});
			return;
		}

		if (event.key === "Escape") {
			setOpenLlmModelPickerId((current) => (current === pickerId ? null : current));
		}
	}

	const syncAppearanceState = useCallback(
		(nextAppearance: AppearanceSettings) => {
			setAppearanceSettings(nextAppearance);
			applyAppearanceSettings(nextAppearance);
			onAppearanceChange?.(nextAppearance);
		},
		[onAppearanceChange],
	);

	// Load initial shortcut config
	useEffect(() => {
		void Promise.all([listBuiltinLlmProviderTemplates().catch(() => []), getAppSettings()]).then(
			([templates, settings]) => {
				setBuiltinLlmTemplates(templates);
				const nextLlmSettings = reconcileLlmSettings(settings.llm, templates);
				setGeneralSettings(settings.general);
				setNotificationSettings(settings.notification);
				syncAppearanceState(settings.appearance);
				setPromptsSettings(settings.prompts);
				setLlmSettings(nextLlmSettings);
				setSelectedLlmProviderId(nextLlmSettings.providers[0]?.id ?? null);
				setSavedLlmSnapshot(buildLlmDraftSnapshot(nextLlmSettings));
				setSavedLlmState(buildSavedLlmDraftState(nextLlmSettings.providers));
				setLlmModelOptions({});
				setLlmModelErrors({});
				setLoadingLlmModelProviderId(null);
				setOpenLlmModelPickerId(null);
				setOcrSettings(settings.ocr);
				const nextRagSettings = reconcileRagSettings(settings.rag, nextLlmSettings.providers);
				setRagSettings(nextRagSettings);
				setSavedRagSnapshot(buildRagDraftSnapshot(nextRagSettings));
				setSavedRagState(buildSavedRagDraftState(nextRagSettings));
				setRagScanResult(null);
				setPersistedAppSettings({
					...settings,
					llm: nextLlmSettings,
					rag: nextRagSettings,
				});
			},
		);
		void getShortcut().then((config) => {
			setShortcutSettings(config);
		});
		void getAcpMcpServers().then((catalog) => {
			const draftServers = catalog.servers.map(createMcpServerDraftFromConfig);
			applyMcpServerDrafts(draftServers, draftServers[0]?.id ?? null);
			setMcpPanelMode(draftServers.length > 0 ? "edit" : "create");
			setBuiltinMcpConfig(catalog.builtin);
			setSavedMcpSnapshot(buildMcpDraftSnapshot(draftServers, catalog.builtin));
			setSavedMcpState(buildSavedMcpDraftState(draftServers, catalog.builtin));
		});
		void getWorkspace()
			.then((workspace) => {
				setWorkspaceContext(workspace);
			})
			.catch(() => {
				setWorkspaceContext(createDefaultWorkspaceState());
			});

		// Listen for shortcut updates from backend
		const unlistenPromise = onShortcutUpdated((config) => {
			setShortcutSettings(config);
		});

		return () => {
			void unlistenPromise.then((unlisten) => unlisten?.());
		};
	}, [syncAppearanceState]);

	// Handle keydown when recording shortcut
	useEffect(() => {
		if (!editingShortcut) return;

		const handleKeyDown = (e: KeyboardEvent) => {
			e.preventDefault();
			e.stopPropagation();

			if (e.key === "Escape") {
				setEditingShortcut(null);
				return;
			}

			const key = normalizeRecordedShortcutKey(e.code, e.key);
			if (key === null) {
				return;
			}

			const modifiers: string[] = [];
			if (e.metaKey) modifiers.push("Cmd");
			if (e.ctrlKey) modifiers.push("Ctrl");
			if (e.altKey) modifiers.push("Alt");
			if (e.shiftKey) modifiers.push("Shift");

			// Build shortcut string
			const shortcut = [...modifiers, key].join("+");

			// Stop editing
			setEditingShortcut(null);
			setSavingShortcutKey(editingShortcut);
			setSettingsError(null);

			void setShortcut(editingShortcut, shortcut)
				.then(() => {
					setShortcutSettings((prev) => ({
						...prev,
						[editingShortcut]: shortcut,
					}));
				})
				.catch((error: unknown) => {
					setSettingsError(getErrorMessage(error, "快捷键保存失败"));
				})
				.finally(() => {
					setSavingShortcutKey((current) => (current === editingShortcut ? null : current));
				});
		};

		const handleKeyUp = (e: KeyboardEvent) => {
			if (e.key === "Escape") {
				setEditingShortcut(null);
			}
		};

		window.addEventListener("keydown", handleKeyDown, true);
		window.addEventListener("keyup", handleKeyUp, true);

		return () => {
			window.removeEventListener("keydown", handleKeyDown, true);
			window.removeEventListener("keyup", handleKeyUp, true);
		};
	}, [editingShortcut]);

	useEffect(() => {
		const nextSelectedLlmProviderId = selectExistingIdOrFirst(
			llmSettings.providers,
			selectedLlmProviderId,
		);
		if (nextSelectedLlmProviderId !== selectedLlmProviderId) {
			setSelectedLlmProviderId(nextSelectedLlmProviderId);
		}
	}, [llmSettings.providers, selectedLlmProviderId]);

	useEffect(() => {
		if (
			openLlmModelPickerId !== null &&
			!openLlmModelPickerId.startsWith(`${selectedLlmProviderId ?? ""}:`)
		) {
			setOpenLlmModelPickerId(null);
		}
	}, [selectedLlmProviderId, openLlmModelPickerId]);

	useEffect(() => {
		if (
			openLlmModelPickerId !== null &&
			!llmSettings.providers.some((provider) => openLlmModelPickerId.startsWith(`${provider.id}:`))
		) {
			setOpenLlmModelPickerId(null);
		}
	}, [llmSettings.providers, openLlmModelPickerId]);

	useEffect(() => {
		const pendingFocus = pendingLlmModelOptionFocusRef.current;
		if (!pendingFocus || openLlmModelPickerId === null) {
			return;
		}

		if (!openLlmModelPickerId.startsWith(`${pendingFocus.providerId}:`)) {
			return;
		}

		pendingLlmModelOptionFocusRef.current = null;
		focusLlmModelOption(pendingFocus.providerId, pendingFocus.target);
	}, [llmModelOptions, openLlmModelPickerId]);

	useEffect(() => {
		function handlePointerDown(event: MouseEvent) {
			const target = event.target;
			if (!(target instanceof Node)) {
				return;
			}
			if (llmModelMenuRef.current?.contains(target)) {
				return;
			}
			setOpenLlmModelPickerId(null);
		}

		document.addEventListener("mousedown", handlePointerDown);
		return () => {
			document.removeEventListener("mousedown", handlePointerDown);
		};
	}, []);

	useEffect(() => {
		setOcrSettings((current) => reconcileOcrSettings(current, llmSettings.providers));
	}, [llmSettings.providers, ocrSettings.llmModelId, ocrSettings.provider]);

	useEffect(() => {
		setRagSettings((current) => reconcileRagSettings(current, llmSettings.providers));
	}, [llmSettings.providers, ragSettings.embeddingModelId]);

	useEffect(() => {
		const nextSelectedMcpServerId = selectExistingIdOrFirst(mcpServers, selectedMcpServerId);
		if (nextSelectedMcpServerId !== selectedMcpServerId) {
			setSelectedMcpServerId(nextSelectedMcpServerId);
		}
	}, [mcpServers, selectedMcpServerId]);

	useEffect(() => {
		const pendingTarget = pendingMcpFocusRef.current;
		if (!pendingTarget || pendingTarget.serverId !== selectedMcpServerId) {
			return;
		}

		if (pendingTarget.scrollToForm) {
			scrollToSectionBlock("mcp-form");
		}

		const refKey = buildMcpFieldRefKey("server", pendingTarget.serverId, pendingTarget.fieldKey);
		const focusTimer = window.setTimeout(
			() => {
				mcpFieldRefs.current[refKey]?.focus();
			},
			pendingTarget.scrollToForm ? 180 : 0,
		);
		pendingMcpFocusRef.current = null;

		return () => {
			window.clearTimeout(focusTimer);
		};
	}, [mcpServers, scrollToSectionBlock, selectedMcpServerId]);

	const handleShortcutClick = (key: keyof ShortcutConfig) => {
		setEditingShortcut(key);
	};

	const isRecording = (key: keyof ShortcutConfig) => editingShortcut === key;
	const llmValidation = validateLlmSettings(llmSettings, builtinLlmTemplates);
	const ragValidation = validateRagSettings(ragSettings, llmSettings);
	const mcpValidation = validateMcpServers(mcpServers);
	const promptsDraftSnapshot = buildPromptsDraftSnapshot(promptsSettings, llmSettings);
	const persistedPromptsDraftSnapshot = buildPromptsDraftSnapshot(
		persistedAppSettings.prompts,
		persistedAppSettings.llm,
	);
	const promptsHaveUnsavedChanges = promptsDraftSnapshot !== persistedPromptsDraftSnapshot;
	const translationTaskDraftSnapshot = buildTranslationTaskDraftSnapshot(
		promptsSettings,
		llmSettings,
	);
	const persistedTranslationTaskDraftSnapshot = buildTranslationTaskDraftSnapshot(
		persistedAppSettings.prompts,
		persistedAppSettings.llm,
	);
	const translationHasUnsavedChanges =
		translationTaskDraftSnapshot !== persistedTranslationTaskDraftSnapshot;
	const questionAnswerTaskDraftSnapshot = buildQuestionAnswerTaskDraftSnapshot(
		promptsSettings,
		llmSettings,
	);
	const persistedQuestionAnswerTaskDraftSnapshot = buildQuestionAnswerTaskDraftSnapshot(
		persistedAppSettings.prompts,
		persistedAppSettings.llm,
	);
	const questionAnswerHasUnsavedChanges =
		questionAnswerTaskDraftSnapshot !== persistedQuestionAnswerTaskDraftSnapshot;
	const {
		handleDiscardLlmDraft,
		handleDiscardMcpDraft,
		handleDiscardQuestionAnswerDraft,
		handleDiscardRagDraft,
		handleDiscardTranslationDraft,
		handleSaveLlm,
		handleSaveMcp,
		handleSaveOcr,
		handleSaveQuestionAnswerConfig,
		handleSaveRag,
		handleSaveTranslationConfig,
		saveAppSettings,
		saveNotificationSettings,
		savingLlm,
		savingMcp,
		savingOcr,
		savingQuestionAnswerConfig,
		savingRag,
		savingSettings,
		savingTranslationConfig,
	} = useSettingsPersistence({
		appearanceSettings,
		applyMcpServerDrafts,
		builtinMcpConfig,
		generalSettings,
		llmSettings,
		llmValidationIssueCount: llmValidation.totalIssues,
		locateFirstLlmIssue,
		locateFirstMcpIssue,
		locateFirstRagIssue,
		mcpServers,
		mcpValidationIssueCount: mcpValidation.totalIssues,
		ocrSettings,
		persistedAppSettings,
		promptsSettings,
		ragSettings,
		ragValidationIssueCount: ragValidation.totalIssues,
		savedLlmSnapshot,
		savedMcpSnapshot,
		savedMcpState,
		selectedLlmProviderId,
		selectedMcpServerId,
		setAppearanceSettings,
		setBuiltinMcpConfig,
		setGeneralSettings,
		setLlmModelErrors,
		setLlmModelOptions,
		setLlmSettings,
		setLoadingLlmModelProviderId,
		setMcpNotice,
		setMcpPanelMode,
		setNotificationSettings,
		setOcrSettings,
		setOpenLlmModelPickerId,
		setPersistedAppSettings,
		setPromptsSettings,
		setRagSettings,
		setSavedLlmSnapshot,
		setSavedLlmState,
		setSavedMcpSnapshot,
		setSavedMcpState,
		setSavedRagSnapshot,
		setSavedRagState,
		setSelectedLlmProviderId,
		setSettingsError,
		syncAppearanceState,
	});
	const savingAiTaskConfig = savingTranslationConfig || savingQuestionAnswerConfig;
	const llmDraftSnapshot = buildLlmDraftSnapshot(llmSettings);
	const llmHasUnsavedChanges = llmDraftSnapshot !== savedLlmSnapshot;
	const ragDraftSnapshot = buildRagDraftSnapshot(ragSettings);
	const ragHasUnsavedChanges = ragDraftSnapshot !== savedRagSnapshot;
	const mcpDraftSnapshot = buildMcpDraftSnapshot(mcpServers, builtinMcpConfig);
	const mcpHasUnsavedChanges = mcpDraftSnapshot !== savedMcpSnapshot;
	const selectedLlmProvider =
		llmSettings.providers.find((provider) => provider.id === selectedLlmProviderId) ?? null;
	const selectedLlmModel = selectedLlmProvider
		? getSelectedProviderModel(selectedLlmProvider)
		: null;
	const selectedLlmProviderIsBuiltin =
		selectedLlmProvider !== null && providerUsesBuiltinTemplate(selectedLlmProvider);
	const selectedBuiltinLlmTemplate = findBuiltinTemplate(
		builtinLlmTemplates,
		selectedLlmProvider?.builtinPresetId,
	);
	const selectedBuiltinLlmTemplateModel = findBuiltinTemplateModelByModelName(
		selectedBuiltinLlmTemplate,
		selectedLlmModel?.model,
	);
	const selectedLlmProviderKind = selectedLlmProvider
		? getLlmProviderKind(selectedLlmProvider, selectedLlmModel?.id)
		: null;
	const selectedLlmProviderModels = selectedLlmProvider
		? (llmModelOptions[selectedLlmProvider.id] ?? [])
		: [];
	const selectedLlmProviderModelsError = selectedLlmProvider
		? (llmModelErrors[selectedLlmProvider.id] ?? null)
		: null;
	const isLoadingSelectedLlmProviderModels =
		selectedLlmProvider !== null && loadingLlmModelProviderId === selectedLlmProvider.id;
	const eligibleOcrProviders = llmSettings.providers.flatMap((provider) =>
		provider.models
			.filter((model) => providerCanHandleOcr(provider, model.id))
			.map((model) => bindProviderToModel(provider, model)),
	);
	const eligibleAiTaskProviders = llmSettings.providers.flatMap((provider) =>
		provider.models
			.filter((model) => providerCanHandleAiTask(provider, model.id))
			.map((model) => bindProviderToModel(provider, model)),
	);
	const selectedTranslationProvider =
		eligibleAiTaskProviders.find(
			(provider) => provider.models[0]?.id === llmSettings.translationModelId,
		) ?? null;
	const selectedQuestionAnswerProvider =
		eligibleAiTaskProviders.find(
			(provider) => provider.models[0]?.id === llmSettings.questionAnswerModelId,
		) ?? null;
	const selectedOcrProvider =
		ocrSettings.provider === "llm_ocr"
			? (eligibleOcrProviders.find(
					(provider) => provider.models[0]?.id === ocrSettings.llmModelId,
				) ?? null)
			: null;
	const promptsDependencyHealthItems: DependencyHealthItem[] = [
		buildModelDependencyHealthItem(
			"翻译模型",
			selectedTranslationProvider,
			"未选择可用的普通 LLM 条目；翻译会在运行时不可用。",
		),
		buildModelDependencyHealthItem(
			"文档问答模型",
			selectedQuestionAnswerProvider,
			"未选择可用的普通 LLM 条目；文档问答和 Agent 复用都会回退或不可用。",
		),
		ocrSettings.provider === "llm_ocr"
			? buildModelDependencyHealthItem(
					"OCR 模型",
					selectedOcrProvider,
					"已启用大模型 OCR，但没有选择可用的多模态 responses 模型。",
				)
			: {
					status: "info",
					label: "OCR 模型",
					detail:
						ocrSettings.provider === "system"
							? "当前使用系统 OCR，不依赖模型页里的多模态条目。"
							: "OCR 已禁用；无选中文本的 OCR 翻译路径会不可用。",
				},
	];
	const translationPromptIsDefault =
		promptsSettings.translationPrompt.trim() === defaultPromptsSettings.translationPrompt.trim();
	const questionAnswerPromptIsDefault =
		promptsSettings.ragAnswerSystemPrompt.trim() ===
		defaultPromptsSettings.ragAnswerSystemPrompt.trim();
	const translationPromptSummary =
		promptsSettings.translationPrompt.trim().length === 0
			? "当前为空，保存后会恢复默认提示词。"
			: translationPromptIsDefault
				? "当前使用内置默认提示词。"
				: "当前使用自定义提示词。";
	const questionAnswerPromptSummary =
		promptsSettings.ragAnswerSystemPrompt.trim().length === 0
			? "当前为空，保存后会恢复默认提示词。"
			: questionAnswerPromptIsDefault
				? "当前使用内置默认提示词。"
				: "当前使用自定义提示词。";
	const eligibleRagEmbeddingProviders = llmSettings.providers.flatMap((provider) =>
		provider.models
			.filter((model) => providerCanHandleRagEmbedding(provider, model.id))
			.map((model) => bindProviderToModel(provider, model)),
	);
	const selectedRagEmbeddingProvider =
		eligibleRagEmbeddingProviders.find(
			(provider) => provider.models[0]?.id === ragSettings.embeddingModelId,
		) ?? null;
	const selectedRagEmbeddingProviderLabel =
		selectedRagEmbeddingProvider?.name ||
		selectedRagEmbeddingProvider?.models[0]?.model ||
		selectedRagEmbeddingProvider?.baseUrl ||
		"未选择 Embedding";
	const ragSourceDirectoryCount = ragSettings.sourceDirectories.length;
	const ragExtraIgnoreGlobs = splitExtraRagIgnoreGlobs(ragSettings.ignoreGlobs);
	const ragExtraIgnoreGlobCount = ragExtraIgnoreGlobs.length;
	const ragSupportedExtensionsLabel = ragSupportedFileExtensions
		.map((extension) => `.${extension}`)
		.join("、");
	const ragStatusTitle = ragHasUnsavedChanges ? "有未保存的更改" : "已同步";
	const ragStatusDescription = ragHasUnsavedChanges
		? "保存后写回 Embedding、目录和忽略规则。"
		: "配置已写入本地；需要时再手动重建。";
	const ragRebuildInputChanged = hasEffectiveRagRebuildInputChanges(
		ragSettings,
		persistedAppSettings.rag,
	);
	const ragDependencyHealthItems: DependencyHealthItem[] = [
		selectedRagEmbeddingProvider
			? {
					status: "ok",
					label: "Embedding 模型",
					detail: `当前使用 ${getBoundProviderLabel(selectedRagEmbeddingProvider)}。`,
				}
			: ragSettings.embeddingModelId
				? {
						status: "warning",
						label: "Embedding 模型",
						detail: "当前选择的 Embedding 条目已经不存在或不再具备 Embedding 能力。",
					}
				: ragSourceDirectoryCount > 0
					? {
							status: "warning",
							label: "Embedding 模型",
							detail: "已配置扫描目录，但还没有选择 Embedding 条目；保存前需要补齐。",
						}
					: {
							status: "info",
							label: "Embedding 模型",
							detail: "尚未启用知识库目录；可以先保存空白配置。",
						},
		{
			status: ragRebuildInputChanged ? "warning" : "ok",
			label: "索引重建",
			detail: ragRebuildInputChanged
				? "Embedding、扫描目录或忽略规则有变化；保存后会触发索引重建。"
				: "当前草稿不会改变有效索引输入；需要时可手动重建。",
		},
	];
	const agentDependencyHealthItems: DependencyHealthItem[] = [
		selectedQuestionAnswerProvider
			? {
					status: "info",
					label: "Agent 模型",
					detail: `会尝试把文档问答模型 ${getBoundProviderLabel(selectedQuestionAnswerProvider)} 用作 Agent 默认模型；无法安全映射时读取底层 settings.json / models.json，thinking 和工具策略仍由底层配置决定。`,
				}
			: {
					status: "warning",
					label: "Agent 模型",
					detail:
						"功能页未配置文档问答模型；Agent session 会读取底层 settings.json / models.json。",
				},
		{
			status:
				builtinMcpConfig.enabled && builtinMcpConfig.enabledModules.length > 0 ? "ok" : "info",
			label: "Agent 工具",
			detail:
				builtinMcpConfig.enabled && builtinMcpConfig.enabledModules.length > 0
					? `已启用 ${builtinMcpConfig.enabledModules.length} 个内置工具模块；只注入 Agent session，不再暴露本地 MCP endpoint。`
					: "内置工具未启用；Wabity 不再对外暴露本地 MCP endpoint。",
		},
	];
	const ragSummaryItems: Array<{ label: string; value: string }> = [
		{ label: "扫描目录", value: `${ragSourceDirectoryCount} 个` },
		{ label: "额外忽略", value: `${ragExtraIgnoreGlobCount} 条` },
		{
			label: "校验问题",
			value: ragValidation.totalIssues > 0 ? `${ragValidation.totalIssues} 个` : "无",
		},
	];
	const ragScanSummaryItems = ragScanResult
		? [
				{ label: "数据库", value: ragScanResult.databasePath },
				{ label: "目录数", value: String(ragScanResult.sourceCount) },
				{ label: "扫描文件", value: String(ragScanResult.scannedFileCount) },
				{ label: "已索引文件", value: String(ragScanResult.indexedFileCount) },
				{ label: "跳过文件", value: String(ragScanResult.skippedFileCount) },
				{ label: "向量块", value: String(ragScanResult.chunkCount) },
				{ label: "警告", value: String(ragScanResult.warningCount) },
			]
		: [];
	const selectedMcpServer = mcpServers.find((server) => server.id === selectedMcpServerId) ?? null;
	const sectionSummaryText: Record<SettingsSectionId, string> = {
		general: "即时生效",
		prompts: promptsHaveUnsavedChanges ? "有草稿" : "已同步",
		llm: llmHasUnsavedChanges ? "有草稿" : `${llmSettings.providers.length} 条`,
		rag: ragHasUnsavedChanges ? "有草稿" : `目录 ${ragSourceDirectoryCount}`,
		mcp: mcpHasUnsavedChanges
			? "有草稿"
			: `${mcpServers.length} 个服务 / ${builtinMcpConfig.enabledModules.length} 个工具模块`,
		about: "只读",
	};
	const sectionDescriptionText: Record<SettingsSectionId, string> = {
		general: "快捷键、通知、外观和桌面行为。",
		prompts: "翻译、文档问答、OCR 和提示词绑定。",
		llm: "维护可复用的模型资源。",
		rag: "本地文档知识库与索引。",
		mcp: "Agent 会话、模型默认值与工具能力。",
		about: "版本与项目信息。",
	};
	const activeSectionLabel =
		settingsSections.find((section) => section.id === activeSection)?.label ?? "设置";
	const selectedMcpIssueCount = selectedMcpServer
		? (mcpValidation.serverIssues[selectedMcpServer.id]?.length ?? 0)
		: 0;
	const regularMcpServers = mcpServers;
	const selectedLlmFieldIssues = selectedLlmProvider
		? (llmValidation.providerFieldIssues[selectedLlmProvider.id] ?? {})
		: {};
	const selectedMcpFieldIssues = selectedMcpServer
		? (mcpValidation.serverFieldIssues[selectedMcpServer.id] ?? {})
		: {};
	const isMcpCreateMode =
		mcpPanelMode === "create" || (!selectedMcpServer && mcpServers.length === 0);
	const isMcpEditMode = !isMcpCreateMode && selectedMcpServer !== null;

	function buildMcpFieldRefKey(scope: "server", id: string, field: McpFieldKey) {
		return `${scope}:${id}:${field}`;
	}

	function bindMcpFieldRef(scope: "server", id: string, field: McpFieldKey) {
		const refKey = buildMcpFieldRefKey(scope, id, field);
		return (node: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | null) => {
			mcpFieldRefs.current[refKey] = node;
		};
	}

	function buildLlmFieldRefKey(id: string, field: LlmFieldKey) {
		return `${id}:${field}`;
	}

	function bindLlmFieldRef(id: string, field: LlmFieldKey) {
		const refKey = buildLlmFieldRefKey(id, field);
		return (node: HTMLInputElement | null) => {
			llmFieldRefs.current[refKey] = node;
		};
	}

	function focusLlmField(id: string, field: LlmFieldKey) {
		const refKey = buildLlmFieldRefKey(id, field);
		window.requestAnimationFrame(() => {
			llmFieldRefs.current[refKey]?.focus();
		});
	}

	function bindRagFieldRef(field: RagFieldKey) {
		return (node: HTMLSelectElement | HTMLTextAreaElement | null) => {
			ragFieldRefs.current[field] = node;
		};
	}

	function focusRagField(field: RagFieldKey) {
		window.requestAnimationFrame(() => {
			ragFieldRefs.current[field]?.focus();
		});
	}

	function locateFirstLlmIssue() {
		const nextIssue = findFirstLlmIssue(llmSettings, builtinLlmTemplates);
		if (!nextIssue) {
			return;
		}

		setActiveSection("llm");
		setSelectedLlmProviderId(nextIssue.providerId);
		focusLlmField(nextIssue.providerId, nextIssue.fieldKey);
	}

	function locateFirstMcpIssue() {
		const nextIssue = findFirstMcpIssue(mcpServers);
		if (!nextIssue) {
			return;
		}

		setActiveSection("mcp");
		queueMcpFieldFocus(nextIssue.serverId, nextIssue.serverFieldKey);
	}

	function locateFirstRagIssue() {
		const ragFieldOrder: RagFieldKey[] = ["embeddingModelId", "sourceDirectories", "ignoreGlobs"];
		const nextField = ragFieldOrder.find((fieldKey) => ragValidation.fieldIssues[fieldKey]);
		if (!nextField) {
			return;
		}

		setActiveSection("rag");
		setSettingsError(
			ragValidation.fieldIssues[nextField] ?? ragValidation.issues[0] ?? "知识库配置不合法",
		);
		focusRagField(nextField);
	}

	function handleHeaderDragStart(event: ReactMouseEvent<HTMLDivElement>) {
		if (event.button !== 0) {
			return;
		}

		event.preventDefault();

		void getCurrentWindow()
			.startDragging()
			.catch((dragError: unknown) => {
				console.warn("failed to start settings window dragging", dragError);
			});
	}

	function handleAddLlmProvider() {
		const nextProvider = createLlmProviderDraft();
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: [...current.providers, nextProvider],
			}),
		);
		setSelectedLlmProviderId(nextProvider.id);
		setSettingsError(null);
	}

	function handleSelectLlmProvider(providerId: string) {
		setSelectedLlmProviderId(providerId);
		setSettingsError(null);
	}

	function handleRemoveLlmProvider(providerId: string) {
		const nextSelectedProviderId =
			selectedLlmProviderId === providerId
				? (llmSettings.providers.find((provider) => provider.id !== providerId)?.id ?? null)
				: selectedLlmProviderId;
		setLlmSettings((current) => {
			return reconcileLlmSettings({
				...current,
				providers: current.providers.filter((provider) => provider.id !== providerId),
			});
		});
		setSelectedLlmProviderId(nextSelectedProviderId);
		clearLlmModelCatalog(providerId);
		setSettingsError(null);
	}

	function handleSelectLlmProviderModel(providerId: string, modelId: string) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					const nextModel =
						provider.models.find((model) => model.id === modelId) ?? provider.models[0];
					return nextModel
						? {
								...provider,
								modelConfig: nextModel,
							}
						: provider;
				}),
			}),
		);
		setSettingsError(null);
	}

	function handleAddLlmProviderModel(providerId: string) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					const seedModel = getSelectedProviderModel(provider);
					const nextModel = createLlmModelDraft(seedModel.modelType);
					return {
						...provider,
						models: [...provider.models, nextModel],
						modelConfig: nextModel,
					};
				}),
			}),
		);
		setSettingsError(null);
	}

	function handleRemoveLlmProviderModel(providerId: string, modelId: string) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId || provider.models.length <= 1) {
						return provider;
					}

					const nextModels = provider.models.filter((model) => model.id !== modelId);
					const nextSelectedModel =
						nextModels.find((model) => model.id === provider.modelConfig?.id) ?? nextModels[0];
					return {
						...provider,
						models: nextModels,
						modelConfig: nextSelectedModel,
					};
				}),
			}),
		);
		setSettingsError(null);
	}

	function handleLlmProviderFieldChange(
		providerId: string,
		key: LlmEditableFieldKey,
		value: string | boolean,
	) {
		if (key === "baseUrl" || key === "apiKey") {
			clearLlmModelCatalog(providerId);
		}

		setLlmSettings((current) => {
			const nextSettings = reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					let nextProvider = { ...provider };
					if (key === "baseUrl" || key === "apiKey") {
						nextProvider = clearProviderModelIdentityHints({
							...nextProvider,
							[key]: value,
						});
					} else if (key === "model") {
						nextProvider = patchSelectedProviderModel(nextProvider, (model) => ({
							...model,
							model: String(value),
							modelIdentityHint:
								findLlmModelOption(providerId, String(value))?.identityHint ?? null,
						}));
						const template = providerUsesBuiltinTemplate(nextProvider)
							? findBuiltinTemplate(builtinLlmTemplates, nextProvider.builtinPresetId)
							: null;
						nextProvider = applyBuiltinTemplateModelMetadata(
							nextProvider,
							findBuiltinTemplateModelByModelName(template, String(value)),
							getSelectedProviderModel(nextProvider).id,
						);
					} else if (key === "name") {
						nextProvider = {
							...nextProvider,
							name: String(value),
						};
					} else {
						nextProvider = patchSelectedProviderModel(nextProvider, (model) => ({
							...model,
							supportsMultimodal: Boolean(value),
						}));
					}

					return nextProvider;
				}),
			});
			return nextSettings;
		});
		setSettingsError(null);
	}

	function handleLlmProviderTemplateChange(providerId: string, templateId: string) {
		clearLlmModelCatalog(providerId);
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					if (!templateId) {
						return clearProviderModelIdentityHints(detachBuiltinTemplateFromProvider(provider));
					}

					const template = findBuiltinTemplate(builtinLlmTemplates, templateId);
					if (!template) {
						return provider;
					}

					return clearProviderModelIdentityHints(
						applyBuiltinTemplateModelMetadata(
							applyBuiltinTemplateToProvider(provider, template),
							findBuiltinTemplateModelByModelName(
								template,
								getSelectedProviderModel(provider).model,
							),
							getSelectedProviderModel(provider).id,
						),
					);
				}),
			}),
		);
		setSettingsError(null);
	}

	function handleBuiltinProviderManagedBaseUrlChange(providerId: string, managed: boolean) {
		clearLlmModelCatalog(providerId);
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					const template = findBuiltinTemplate(builtinLlmTemplates, provider.builtinPresetId);
					if (!template) {
						return {
							...provider,
							managedBaseUrl: managed,
						};
					}

					const nextProvider = {
						...provider,
						managedBaseUrl: managed,
						baseUrl: managed ? template.defaultBaseUrl : provider.baseUrl,
					};
					return clearProviderModelIdentityHints(nextProvider);
				}),
			}),
		);
		setSettingsError(null);
	}

	function handleLlmProviderKindChange(providerId: string, kind: LlmProviderKind) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					const nextProvider = applyLlmProviderKind(
						provider,
						kind,
						getSelectedProviderModel(provider).id,
					);
					return patchSelectedProviderModel(nextProvider, (model) => ({
						...model,
						builtinPresetModelId: null,
					}));
				}),
			}),
		);
		setSettingsError(null);
	}

	function handleLlmRouteProviderChange(
		key: "translationModelId" | "questionAnswerModelId",
		providerId: string | null,
	) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				[key]: providerId,
			}),
		);
		setSettingsError(null);
	}

	async function handleAppendRagSourceDirectory() {
		const selectedPath = await chooseDirectory(ragDirectoryPickerDefaultPath);
		if (!selectedPath) {
			return;
		}

		setRagSettings((current) => ({
			...current,
			sourceDirectories: current.sourceDirectories.includes(selectedPath)
				? current.sourceDirectories
				: [...current.sourceDirectories, selectedPath],
		}));
		setSettingsError(null);
	}

	async function handleScanRag() {
		if (ragValidation.totalIssues > 0) {
			locateFirstRagIssue();
			return;
		}

		setScanningRag(true);
		setSettingsError(null);
		try {
			const result = await scanRagSources(ragSettings, llmSettings);
			setRagScanResult(result);
		} catch (error: unknown) {
			setSettingsError(getErrorMessage(error, "知识库扫描失败"));
		} finally {
			setScanningRag(false);
		}
	}

	function handleAddMcpServer(transport: McpTransport) {
		setMcpNotice(null);
		const nextServer = createMcpServerDraft(transport);
		const transportMeta = getMcpTransportMeta(transport);
		setMcpNotice({
			tone: "info",
			text: `已新建 ${transportMeta.label} 服务，继续填写当前表单即可。`,
		});
		applyMcpServerDrafts([...mcpServers, nextServer], nextServer.id);
		setMcpPanelMode("edit");
		queueMcpFieldFocus(nextServer.id, "name");
	}

	function handleRemoveMcpServer(serverId: string) {
		setMcpNotice(null);
		const nextServers = mcpServers.filter((server) => server.id !== serverId);
		const nextSelectedServerId =
			selectedMcpServerId === serverId ? (nextServers[0]?.id ?? null) : selectedMcpServerId;
		applyMcpServerDrafts(nextServers, nextSelectedServerId);
		setMcpPanelMode(nextSelectedServerId ? "edit" : "create");
	}

	function handleMcpServerFieldChange(
		serverId: string,
		key: keyof Omit<AcpMcpServerDraft, "id">,
		value: string,
	) {
		setMcpNotice(null);
		setMcpServersState((current) =>
			current.map((server) => (server.id === serverId ? { ...server, [key]: value } : server)),
		);
	}

	function handleApplyMcpTransportSelection() {
		handleAddMcpServer(selectedMcpTransport);
	}

	function handleToggleBuiltinMcp(enabled: boolean) {
		setMcpNotice(null);
		setBuiltinMcpConfig((current) => ({
			...current,
			enabled,
		}));
	}

	function handleToggleBuiltinMcpModule(moduleKey: BuiltinMcpConfig["enabledModules"][number]) {
		setMcpNotice(null);
		setBuiltinMcpConfig((current) => {
			const nextEnabledModules = current.enabledModules.includes(moduleKey)
				? current.enabledModules.filter((key) => key !== moduleKey)
				: [...current.enabledModules, moduleKey];
			nextEnabledModules.sort();
			return {
				...current,
				enabledModules: nextEnabledModules,
			};
		});
	}

	return (
		<main className="settings-shell" ref={shellRef}>
			<section
				className="settings-frame"
				style={{
					width: `${frameSize.width}px`,
					maxWidth: `${frameSize.width}px`,
					height: `${frameSize.height}px`,
					maxHeight: `${frameSize.height}px`,
				}}
			>
				<header className="settings-header">
					<button className="settings-back-button" onClick={onBack} type="button">
						← 返回
					</button>
					<div
						aria-hidden="true"
						className="settings-header-drag-zone"
						onMouseDown={handleHeaderDragStart}
					/>
					<h1 className="settings-title">设置</h1>
					<div
						aria-hidden="true"
						className="settings-header-drag-zone"
						onMouseDown={handleHeaderDragStart}
					/>
					{onLauncherPinnedChange ? (
						<LauncherPinButton
							className="settings-pin-button"
							pinned={launcherPinned}
							onToggle={() => void onLauncherPinnedChange(!launcherPinned)}
						/>
					) : null}
					<button
						className="settings-close-button"
						onClick={() => void hideLauncherWindow()}
						type="button"
					>
						关闭
					</button>
				</header>

				<div className="settings-content">
					<div className="settings-layout">
						<aside className="settings-sidebar">
							<div className="settings-sidebar-inner">
								<nav
									aria-label="设置分组"
									className="settings-nav settings-nav-rail"
									role="tablist"
								>
									{settingsSections.map((section) => (
										<button
											aria-controls={getSettingsPanelId(section.id)}
											aria-selected={activeSection === section.id}
											className={`settings-nav-button ${activeSection === section.id ? "settings-nav-button-active" : ""}`}
											id={getSettingsTabId(section.id)}
											key={section.id}
											onKeyDown={(event) => handleSectionTabKeyDown(event, section.id)}
											onClick={() => handleSelectSection(section.id)}
											ref={(element) => {
												sectionTabRefs.current[section.id] = element;
											}}
											role="tab"
											tabIndex={activeSection === section.id ? 0 : -1}
											type="button"
										>
											<span className="settings-nav-button-label">{section.label}</span>
											<span className="settings-nav-button-meta">
												{sectionSummaryText[section.id]}
											</span>
										</button>
									))}
								</nav>
							</div>
						</aside>

						<div className="settings-main" ref={contentRef}>
							{settingsError ? (
								<div className="settings-banner settings-banner-error">{settingsError}</div>
							) : null}
							<div className="settings-main-header">
								<div className="settings-main-header-copy">
									<h2 className="settings-main-title">{activeSectionLabel}</h2>
									<span className="settings-help-text settings-help-text-tight">
										{sectionDescriptionText[activeSection]}
									</span>
								</div>
								{activeQuickLinks.length > 0 ? (
									<SettingsQuickJumpList
										activeBlockId={activeSectionBlockId}
										compact
										links={activeQuickLinks}
										onSelect={scrollToSectionBlock}
									/>
								) : null}
							</div>

							{activeSection === "general" ? (
								<GeneralSettingsSection
									appearanceSettings={appearanceSettings}
									bindSectionBlockRef={bindSectionBlockRef}
									generalSettings={generalSettings}
									isRecording={isRecording}
									notificationSettings={notificationSettings}
									onSaveAppSettings={saveAppSettings}
									onSaveNotificationSettings={saveNotificationSettings}
									onShortcutClick={handleShortcutClick}
									savingSettings={savingSettings}
									savingShortcutKey={savingShortcutKey}
									shortcutSettings={shortcutSettings}
								/>
							) : null}

							{activeSection === "prompts" ? (
								<PromptsSettingsSection
									bindSectionBlockRef={bindSectionBlockRef}
									defaultQuestionAnswerPrompt={defaultPromptsSettings.ragAnswerSystemPrompt}
									defaultTranslationPrompt={defaultPromptsSettings.translationPrompt}
									dependencyHealthItems={promptsDependencyHealthItems}
									eligibleAiTaskProviders={eligibleAiTaskProviders}
									eligibleOcrProviders={eligibleOcrProviders}
									llmSettings={llmSettings}
									onDiscardQuestionAnswerDraft={handleDiscardQuestionAnswerDraft}
									onDiscardTranslationDraft={handleDiscardTranslationDraft}
									onLlmRouteProviderChange={handleLlmRouteProviderChange}
									onSaveQuestionAnswerConfig={handleSaveQuestionAnswerConfig}
									onSaveOcr={handleSaveOcr}
									onSaveTranslationConfig={handleSaveTranslationConfig}
									onSelectSection={handleSelectSection}
									ocrSettings={ocrSettings}
									promptsSettings={promptsSettings}
									questionAnswerHasUnsavedChanges={questionAnswerHasUnsavedChanges}
									questionAnswerPromptExpanded={questionAnswerPromptExpanded}
									questionAnswerPromptSummary={questionAnswerPromptSummary}
									savingAiTaskConfig={savingAiTaskConfig}
									savingOcr={savingOcr}
									savingQuestionAnswerConfig={savingQuestionAnswerConfig}
									savingTranslationConfig={savingTranslationConfig}
									selectedQuestionAnswerProvider={selectedQuestionAnswerProvider}
									selectedTranslationProvider={selectedTranslationProvider}
									setPromptsSettings={setPromptsSettings}
									setOcrSettings={setOcrSettings}
									setQuestionAnswerPromptExpanded={setQuestionAnswerPromptExpanded}
									setTranslationPromptExpanded={setTranslationPromptExpanded}
									showLlmOcrFields={showLlmOcrFields}
									summarizeLlmProviderProfile={summarizeLlmProviderProfile}
									translationHasUnsavedChanges={translationHasUnsavedChanges}
									translationPromptExpanded={translationPromptExpanded}
									translationPromptSummary={translationPromptSummary}
								/>
							) : null}

							{activeSection === "llm" ? (
								<LlmSettingsSection
									bindLlmFieldRef={bindLlmFieldRef}
									bindSectionBlockRef={bindSectionBlockRef}
									buildLlmModelPickerId={buildLlmModelPickerId}
									builtinLlmTemplates={builtinLlmTemplates}
									getLlmProviderKind={(provider) =>
										getLlmProviderKind(provider, getSelectedProviderModel(provider).id)
									}
									getLlmProviderKindLabel={getLlmProviderKindLabel}
									getLlmProviderModelPlaceholder={(provider) =>
										getLlmProviderModelPlaceholder(provider, getSelectedProviderModel(provider).id)
									}
									isLoadingSelectedLlmProviderModels={isLoadingSelectedLlmProviderModels}
									llmHasUnsavedChanges={llmHasUnsavedChanges}
									llmModelMenuRef={llmModelMenuRef}
									llmSettings={llmSettings}
									llmValidation={llmValidation}
									onAddLlmProvider={handleAddLlmProvider}
									onAddLlmProviderModel={handleAddLlmProviderModel}
									onBuiltinProviderManagedBaseUrlChange={handleBuiltinProviderManagedBaseUrlChange}
									onDiscardLlmDraft={handleDiscardLlmDraft}
									onFetchLlmProviderModels={handleFetchLlmProviderModels}
									onLlmModelInputKeyDown={handleLlmModelInputKeyDown}
									onLlmModelOptionClick={handleLlmModelOptionClick}
									onLlmProviderFieldChange={handleLlmProviderFieldChange}
									onLlmProviderKindChange={handleLlmProviderKindChange}
									onLlmProviderTemplateChange={handleLlmProviderTemplateChange}
									onLocateFirstLlmIssue={locateFirstLlmIssue}
									onOpenUrl={(url) => void openUrl(url)}
									onRemoveLlmProviderModel={handleRemoveLlmProviderModel}
									onRemoveLlmProvider={handleRemoveLlmProvider}
									onSaveLlm={handleSaveLlm}
									onSelectLlmProviderModel={handleSelectLlmProviderModel}
									onSelectLlmProvider={handleSelectLlmProvider}
									onToggleLlmModelMenu={handleToggleLlmModelMenu}
									openLlmModelPickerId={openLlmModelPickerId}
									providerCanHandleOcr={(provider) =>
										providerCanHandleOcr(provider, getSelectedProviderModel(provider).id)
									}
									providerHasResponsesModel={(provider) =>
										providerHasResponsesModel(provider, getSelectedProviderModel(provider).id)
									}
									providerIsLlmModel={(provider) =>
										providerIsLlmModel(provider, getSelectedProviderModel(provider).id)
									}
									savingLlm={savingLlm}
									selectedBuiltinLlmTemplate={selectedBuiltinLlmTemplate}
									selectedBuiltinLlmTemplateModel={selectedBuiltinLlmTemplateModel}
									selectedLlmFieldIssues={selectedLlmFieldIssues}
									selectedLlmProvider={selectedLlmProvider}
									selectedLlmProviderId={selectedLlmProviderId}
									selectedLlmProviderIsBuiltin={selectedLlmProviderIsBuiltin}
									selectedLlmProviderKind={selectedLlmProviderKind}
									selectedLlmProviderModels={selectedLlmProviderModels}
									selectedLlmProviderModelsError={selectedLlmProviderModelsError}
									summarizeLlmProviderProfile={summarizeLlmProviderProfile}
								/>
							) : null}

							{activeSection === "rag" ? (
								<RagSettingsSection
									bindRagFieldRef={bindRagFieldRef}
									bindSectionBlockRef={bindSectionBlockRef}
									defaultRagIgnoreGlobs={defaultRagIgnoreGlobs}
									dependencyHealthItems={ragDependencyHealthItems}
									eligibleRagEmbeddingProviders={eligibleRagEmbeddingProviders}
									extraRagIgnoreGlobPlaceholder={extraRagIgnoreGlobPlaceholder}
									formatTextLines={formatTextLines}
									normalizeRagIgnoreGlobs={normalizeRagIgnoreGlobs}
									onAppendRagSourceDirectory={handleAppendRagSourceDirectory}
									onDiscardRagDraft={handleDiscardRagDraft}
									onLocateFirstRagIssue={locateFirstRagIssue}
									onSaveRag={handleSaveRag}
									onScanRag={handleScanRag}
									parseTextLines={parseTextLines}
									ragExtraIgnoreGlobs={ragExtraIgnoreGlobs}
									ragHasUnsavedChanges={ragHasUnsavedChanges}
									ragScanResult={ragScanResult}
									ragScanSummaryItems={ragScanSummaryItems}
									ragSettings={ragSettings}
									ragSourceDirectoryPlaceholder={ragSourceDirectoryPlaceholder}
									ragStatusDescription={ragStatusDescription}
									ragStatusTitle={ragStatusTitle}
									ragSummaryItems={ragSummaryItems}
									ragSupportedExtensionsLabel={ragSupportedExtensionsLabel}
									ragValidation={ragValidation}
									savingRag={savingRag}
									scanningRag={scanningRag}
									selectedRagEmbeddingProvider={selectedRagEmbeddingProvider}
									selectedRagEmbeddingProviderLabel={selectedRagEmbeddingProviderLabel}
									setRagSettings={setRagSettings}
								/>
							) : null}

							{activeSection === "mcp" ? (
								<McpSettingsSection
									agentHealthItems={agentDependencyHealthItems}
									bindMcpFieldRef={bindMcpFieldRef}
									bindMcpListOptionRef={bindMcpListOptionRef}
									bindSectionBlockRef={bindSectionBlockRef}
									builtinMcpConfig={builtinMcpConfig}
									builtinAgentToolStatus={builtinAgentToolStatus}
									isMcpCreateMode={isMcpCreateMode}
									isMcpEditMode={isMcpEditMode}
									mcpHasUnsavedChanges={mcpHasUnsavedChanges}
									mcpNotice={mcpNotice}
									mcpValidation={mcpValidation}
									onApplyMcpTransportSelection={handleApplyMcpTransportSelection}
									onDiscardMcpDraft={handleDiscardMcpDraft}
									onEnterMcpCreateMode={handleEnterMcpCreateMode}
									onLocateFirstMcpIssue={locateFirstMcpIssue}
									onMcpCatalogKeyDown={handleMcpCatalogKeyDown}
									onMcpServerFieldChange={handleMcpServerFieldChange}
									onRemoveMcpServer={handleRemoveMcpServer}
									onReturnToCurrentMcp={handleReturnToCurrentMcp}
									onSaveMcp={handleSaveMcp}
									onSelectMcpServer={selectMcpServer}
									onToggleBuiltinMcp={handleToggleBuiltinMcp}
									onToggleBuiltinMcpModule={handleToggleBuiltinMcpModule}
									regularMcpServers={regularMcpServers}
									savingMcp={savingMcp}
									selectedMcpFieldIssues={selectedMcpFieldIssues}
									selectedMcpIssueCount={selectedMcpIssueCount}
									selectedMcpServer={selectedMcpServer}
									selectedMcpServerId={selectedMcpServerId}
									selectedMcpTransport={selectedMcpTransport}
									setSelectedMcpTransport={setSelectedMcpTransport}
								/>
							) : null}

							{activeSection === "about" ? (
								<AboutSettingsSection
									bindSectionBlockRef={bindSectionBlockRef}
									onOpenUrl={(url) => void openUrl(url)}
								/>
							) : null}
						</div>
					</div>
				</div>
			</section>
		</main>
	);
}
