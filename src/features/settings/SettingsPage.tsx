import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
	chooseDirectory,
	getAppSettings,
	getAcpAgents,
	getAcpMcpServers,
	getBuiltinMcpServerStatus,
	getPublicSkillCatalog,
	getShortcut,
	getShortcutRuntimeStatus,
	getWorkspace,
	hideLauncherWindow,
	listBuiltinLlmProviderTemplates,
	listLlmProviderModels,
	onShortcutRuntimeStatusChanged,
	onShortcutUpdated,
	scanRagSources,
	setShortcut,
	defaultRagIgnoreGlobs,
} from "../../lib/tauri/client";
import type {
	AppSettings,
	AppearanceSettings,
	BuiltinMcpConfig,
	BuiltinMcpServerStatus,
	BuiltinLlmProviderTemplate,
	GeneralSettings,
	LlmProviderConfig,
	LlmProviderModelEntry,
	LlmSettings,
	NotificationSettings,
	OcrSettings,
	PromptsSettings,
	PublicSkillCatalog,
	RagScanResult,
	RagSettings,
	ShortcutConfig,
	ShortcutKey,
	ShortcutRuntimeStatus,
	WorkspaceState,
} from "../../lib/tauri/types";
import { useSettingsWindowFrame } from "./useSettingsWindowFrame";
import { applyAppearanceSettings, defaultAppearanceSettings } from "../../app/appearance";
import {
	AboutSettingsSection,
	AcpSettingsSection,
	GeneralSettingsSection,
	LlmSettingsSection,
	McpSettingsSection,
	PromptsSettingsSection,
	RagSettingsSection,
	SkillsSettingsSection,
} from "./SettingsSectionViews";
import {
	SettingsQuickJumpList,
	ShortcutSummaryCard,
	acpAgentOptions,
	getMcpTransportMeta,
	getSettingsPanelId,
	getSettingsTabId,
	settingsSections,
} from "./settingsShared";
import {
	AcpAgentDraft,
	AcpFieldKey,
	AcpInlineNotice,
	AcpMcpServerDraft,
	LlmFieldKey,
	LlmModelFieldKey,
	LlmProviderKind,
	McpFieldKey,
	McpPanelMode,
	McpTransport,
	PendingMcpFocusTarget,
	RagFieldKey,
	SavedAcpDraftState,
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
	buildAcpDraftSnapshot,
	buildAgentCommand,
	buildLlmDraftSnapshot,
	buildMcpDraftSnapshot,
	buildPromptsDraftSnapshot,
	buildQuestionAnswerTaskDraftSnapshot,
	buildRagDraftSnapshot,
	buildSavedAcpDraftState,
	buildSavedLlmDraftState,
	buildSavedMcpDraftState,
	buildSavedRagDraftState,
	buildTranslationTaskDraftSnapshot,
	createAgentDraft,
	createDefaultAppSettings,
	createDefaultGeneralSettings,
	createDefaultLlmSettings,
	createDefaultNotificationSettings,
	createDefaultOcrSettings,
	createDefaultRagSettings,
	createDefaultShortcutSettings,
	createDefaultWorkspaceState,
	createLlmProviderDraft,
	createMcpServerDraft,
	createMcpServerDraftFromConfig,
	detachBuiltinTemplateFromProvider,
	extraRagIgnoreGlobPlaceholder,
	findBuiltinTemplate,
	findBuiltinTemplateModelByModelName,
	findFirstAcpIssue,
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
	validateAcpAgents,
	validateLlmSettings,
	validateMcpServers,
	validateRagSettings,
} from "./settingsState";
import { useSettingsPersistence } from "./useSettingsPersistence";
import { useSettingsSectionNavigation } from "./useSettingsSectionNavigation";
import "./settings.css";

interface SettingsPageProps {
	onBack: () => void;
	onAppearanceChange?: (appearance: AppearanceSettings) => void;
	shortcutRuntimeStatus?: ShortcutRuntimeStatus;
}

export function SettingsPage({
	onBack,
	onAppearanceChange,
	shortcutRuntimeStatus: shortcutRuntimeStatusProp,
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
	const [shortcutRuntimeStatus, setShortcutRuntimeStatus] = useState<ShortcutRuntimeStatus>(
		shortcutRuntimeStatusProp ?? {
			toggle_launcher: {
				configuredShortcut: "Alt+Space",
				registered: true,
				message: null,
			},
			ocr_translate: {
				configuredShortcut: "Alt+D",
				registered: true,
				message: null,
			},
			open_clipboard_history: {
				configuredShortcut: "Alt+V",
				registered: true,
				message: null,
			},
		},
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
	const [acpAgents, setAcpAgentsState] = useState<AcpAgentDraft[]>([]);
	const [defaultAgentId, setDefaultAgentId] = useState<string | null>(null);
	const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
	const [savedAcpSnapshot, setSavedAcpSnapshot] = useState(() => buildAcpDraftSnapshot([]));
	const [savedAcpState, setSavedAcpState] = useState<SavedAcpDraftState>(() =>
		buildSavedAcpDraftState([], null),
	);
	const [acpNotice, setAcpNotice] = useState<AcpInlineNotice | null>(null);
	const [selectedPresetOptionId, setSelectedPresetOptionId] = useState("__custom__");
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
	const [builtinMcpServerStatus, setBuiltinMcpServerStatus] =
		useState<BuiltinMcpServerStatus | null>(null);
	const [skillCatalog, setSkillCatalog] = useState<PublicSkillCatalog>({
		rootPath: "~/.agents/skills",
		exists: false,
		skills: [],
	});
	const [workspaceContext, setWorkspaceContext] = useState<WorkspaceState>(
		createDefaultWorkspaceState,
	);
	const [selectedSkillId, setSelectedSkillId] = useState<string | null>(null);
	const [skillError, setSkillError] = useState<string | null>(null);

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
	const acpFieldRefs = useRef<
		Record<string, HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | null>
	>({});
	const mcpListOptionRefs = useRef<Record<string, HTMLButtonElement | null>>({});
	const pendingMcpFocusRef = useRef<PendingMcpFocusTarget | null>(null);
	const llmFieldRefs = useRef<Record<string, HTMLInputElement | null>>({});
	const ragFieldRefs = useRef<Record<RagFieldKey, HTMLSelectElement | HTMLTextAreaElement | null>>({
		embeddingProviderId: null,
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
		handleViewSkill,
		scrollToSectionBlock,
		sectionTabRefs,
	} = useSettingsSectionNavigation({
		activeSection,
		setActiveSection,
		setSelectedSkillId,
	});

	function applyAgentDrafts(nextAgents: AcpAgentDraft[], nextSelectedAgentId: string | null) {
		setAcpAgentsState(nextAgents);
		setSelectedAgentId(nextSelectedAgentId);
	}

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
	}

	function queueLlmModelOptionFocus(providerId: string, target: "selected" | "first" | "last") {
		pendingLlmModelOptionFocusRef.current = { providerId, target };
	}

	function findLlmModelOption(providerId: string, model: string) {
		return (llmModelOptions[providerId] ?? []).find((option) => option.id === model) ?? null;
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
			const matchedOption = models.find((option) => option.id === provider.model) ?? null;
			if (matchedOption) {
				setLlmSettings((current) =>
					reconcileLlmSettings({
						...current,
						providers: current.providers.map((candidate) =>
							candidate.id === provider.id
								? {
										...candidate,
										modelIdentityHint: matchedOption.identityHint,
									}
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

					const nextProvider = {
						...provider,
						[fieldKey]: option.id,
						modelIdentityHint: option.identityHint,
					};
					const template = providerUsesBuiltinTemplate(nextProvider)
						? findBuiltinTemplate(builtinLlmTemplates, nextProvider.builtinPresetId)
						: null;
					return applyBuiltinTemplateModelMetadata(
						nextProvider,
						findBuiltinTemplateModelByModelName(template, option.id),
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
		void listBuiltinLlmProviderTemplates()
			.then((templates) => {
				setBuiltinLlmTemplates(templates);
			})
			.catch(() => {
				setBuiltinLlmTemplates([]);
			});
		void getAppSettings().then((settings) => {
			setGeneralSettings(settings.general);
			setNotificationSettings(settings.notification);
			syncAppearanceState(settings.appearance);
			setPromptsSettings(settings.prompts);
			setLlmSettings(settings.llm);
			setSelectedLlmProviderId(settings.llm.providers[0]?.id ?? null);
			setSavedLlmSnapshot(buildLlmDraftSnapshot(settings.llm));
			setSavedLlmState(buildSavedLlmDraftState(settings.llm.providers));
			setLlmModelOptions({});
			setLlmModelErrors({});
			setLoadingLlmModelProviderId(null);
			setOpenLlmModelPickerId(null);
			setOcrSettings(settings.ocr);
			const nextRagSettings = reconcileRagSettings(settings.rag, settings.llm.providers);
			setRagSettings(nextRagSettings);
			setSavedRagSnapshot(buildRagDraftSnapshot(nextRagSettings));
			setSavedRagState(buildSavedRagDraftState(nextRagSettings));
			setRagScanResult(null);
			setPersistedAppSettings(settings);
		});
		void getShortcut().then((config) => {
			setShortcutSettings(config);
		});
		void getShortcutRuntimeStatus().then((status) => {
			setShortcutRuntimeStatus(status);
		});
		void getAcpAgents().then((catalog) => {
			const draftAgents = catalog.agents.map((agent) => ({
				id: agent.id,
				name: agent.name,
				command: buildAgentCommand(agent),
				launchMode: agent.launchMode,
			}));
			const nextDefaultAgentId = catalog.defaultAgentId ?? catalog.agents[0]?.id ?? null;
			applyAgentDrafts(draftAgents, draftAgents[0]?.id ?? null);
			setDefaultAgentId(nextDefaultAgentId);
			setSavedAcpSnapshot(buildAcpDraftSnapshot(draftAgents));
			setSavedAcpState(buildSavedAcpDraftState(draftAgents, nextDefaultAgentId));
		});
		void getAcpMcpServers().then((catalog) => {
			const draftServers = catalog.servers.map(createMcpServerDraftFromConfig);
			applyMcpServerDrafts(draftServers, draftServers[0]?.id ?? null);
			setMcpPanelMode(draftServers.length > 0 ? "edit" : "create");
			setBuiltinMcpConfig(catalog.builtin);
			setSavedMcpSnapshot(buildMcpDraftSnapshot(draftServers, catalog.builtin));
			setSavedMcpState(buildSavedMcpDraftState(draftServers, catalog.builtin));
		});
		void getBuiltinMcpServerStatus()
			.then((status) => {
				setBuiltinMcpServerStatus(status);
			})
			.catch((error: unknown) => {
				setBuiltinMcpServerStatus({
					server: {
						transport: "http",
						name: "Wabity Built-in MCP",
						url: "http://127.0.0.1:43189/internal/mcp",
						headers: [],
					},
					running: false,
					lastError: getErrorMessage(error, "内置 MCP server 状态读取失败"),
					availableModules: [],
				});
			});
		void getPublicSkillCatalog()
			.then((catalog) => {
				setSkillError(null);
				setSkillCatalog(catalog);
				setSelectedSkillId(catalog.skills[0]?.id ?? null);
			})
			.catch((error: unknown) => {
				setSkillError(getErrorMessage(error, "公共 skill 读取失败"));
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
		const unlistenRuntimeStatusPromise = onShortcutRuntimeStatusChanged((status) => {
			setShortcutRuntimeStatus(status);
		});

		return () => {
			void unlistenPromise.then((unlisten) => unlisten?.());
			void unlistenRuntimeStatusPromise.then((unlisten) => unlisten?.());
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
		if (shortcutRuntimeStatusProp) {
			setShortcutRuntimeStatus(shortcutRuntimeStatusProp);
		}
	}, [shortcutRuntimeStatusProp]);

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
	}, [llmSettings.providers, ocrSettings.llmProviderId, ocrSettings.provider]);

	useEffect(() => {
		setRagSettings((current) => reconcileRagSettings(current, llmSettings.providers));
	}, [llmSettings.providers, ragSettings.embeddingProviderId]);

	useEffect(() => {
		const nextSelectedAgentId = selectExistingIdOrFirst(acpAgents, selectedAgentId);
		if (nextSelectedAgentId !== selectedAgentId) {
			setSelectedAgentId(nextSelectedAgentId);
		}
	}, [acpAgents, selectedAgentId]);

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

		const refKey = buildAcpFieldRefKey("server", pendingTarget.serverId, pendingTarget.fieldKey);
		const focusTimer = window.setTimeout(
			() => {
				acpFieldRefs.current[refKey]?.focus();
			},
			pendingTarget.scrollToForm ? 180 : 0,
		);
		pendingMcpFocusRef.current = null;

		return () => {
			window.clearTimeout(focusTimer);
		};
	}, [mcpServers, scrollToSectionBlock, selectedMcpServerId]);

	useEffect(() => {
		const nextSelectedSkillId = selectExistingIdOrFirst(skillCatalog.skills, selectedSkillId);
		if (nextSelectedSkillId !== selectedSkillId) {
			setSelectedSkillId(nextSelectedSkillId);
		}
	}, [selectedSkillId, skillCatalog.skills]);

	const handleShortcutClick = (key: keyof ShortcutConfig) => {
		setEditingShortcut(key);
	};

	const handleJumpToShortcutSettings = useCallback(
		(key?: ShortcutKey) => {
			scrollToSectionBlock("general-shortcuts");
			if (key) {
				const triggerIdByKey: Record<ShortcutKey, string> = {
					toggle_launcher: "shortcut-toggle-launcher-trigger",
					ocr_translate: "shortcut-ocr-translate-trigger",
					open_clipboard_history: "shortcut-open-clipboard-history-trigger",
				};
				window.setTimeout(() => {
					document.getElementById(triggerIdByKey[key])?.focus();
				}, 180);
			}
		},
		[scrollToSectionBlock],
	);

	const isRecording = (key: keyof ShortcutConfig) => editingShortcut === key;
	const llmValidation = validateLlmSettings(llmSettings, builtinLlmTemplates);
	const ragValidation = validateRagSettings(ragSettings, llmSettings);
	const acpValidation = validateAcpAgents(acpAgents);
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
		handleDiscardAcpDraft,
		handleDiscardLlmDraft,
		handleDiscardMcpDraft,
		handleDiscardQuestionAnswerDraft,
		handleDiscardRagDraft,
		handleDiscardTranslationDraft,
		handleSaveAgent,
		handleSaveLlm,
		handleSaveMcp,
		handleSaveOcr,
		handleSaveQuestionAnswerConfig,
		handleSaveRag,
		handleSaveTranslationConfig,
		saveAppSettings,
		saveNotificationSettings,
		savingAgent,
		savingLlm,
		savingMcp,
		savingOcr,
		savingQuestionAnswerConfig,
		savingRag,
		savingSettings,
		savingTranslationConfig,
	} = useSettingsPersistence({
		acpAgents,
		acpValidationIssueCount: acpValidation.totalIssues,
		appearanceSettings,
		applyAgentDrafts,
		applyMcpServerDrafts,
		builtinMcpConfig,
		defaultAgentId,
		generalSettings,
		llmSettings,
		llmValidationIssueCount: llmValidation.totalIssues,
		locateFirstAcpIssue,
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
		savedAcpSnapshot,
		savedAcpState,
		savedLlmSnapshot,
		savedMcpSnapshot,
		savedMcpState,
		selectedAgentId,
		selectedLlmProviderId,
		selectedMcpServerId,
		setAcpNotice,
		setAppearanceSettings,
		setBuiltinMcpConfig,
		setDefaultAgentId,
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
		setSavedAcpSnapshot,
		setSavedAcpState,
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
	const acpDraftSnapshot = buildAcpDraftSnapshot(acpAgents);
	const acpHasUnsavedChanges = acpDraftSnapshot !== savedAcpSnapshot;
	const mcpDraftSnapshot = buildMcpDraftSnapshot(mcpServers, builtinMcpConfig);
	const mcpHasUnsavedChanges = mcpDraftSnapshot !== savedMcpSnapshot;
	const selectedLlmProvider =
		llmSettings.providers.find((provider) => provider.id === selectedLlmProviderId) ?? null;
	const selectedLlmProviderIsBuiltin =
		selectedLlmProvider !== null && providerUsesBuiltinTemplate(selectedLlmProvider);
	const selectedBuiltinLlmTemplate = findBuiltinTemplate(
		builtinLlmTemplates,
		selectedLlmProvider?.builtinPresetId,
	);
	const selectedBuiltinLlmTemplateModel = findBuiltinTemplateModelByModelName(
		selectedBuiltinLlmTemplate,
		selectedLlmProvider?.model,
	);
	const selectedLlmProviderKind = selectedLlmProvider
		? getLlmProviderKind(selectedLlmProvider)
		: null;
	const selectedLlmProviderModels = selectedLlmProvider
		? (llmModelOptions[selectedLlmProvider.id] ?? [])
		: [];
	const selectedLlmProviderModelsError = selectedLlmProvider
		? (llmModelErrors[selectedLlmProvider.id] ?? null)
		: null;
	const isLoadingSelectedLlmProviderModels =
		selectedLlmProvider !== null && loadingLlmModelProviderId === selectedLlmProvider.id;
	const eligibleOcrProviders = llmSettings.providers.filter((provider) =>
		providerCanHandleOcr(provider),
	);
	const eligibleAiTaskProviders = llmSettings.providers.filter((provider) =>
		providerCanHandleAiTask(provider),
	);
	const selectedTranslationProvider =
		eligibleAiTaskProviders.find((provider) => provider.id === llmSettings.translationProviderId) ??
		null;
	const selectedQuestionAnswerProvider =
		eligibleAiTaskProviders.find(
			(provider) => provider.id === llmSettings.questionAnswerProviderId,
		) ?? null;
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
	const eligibleRagEmbeddingProviders = llmSettings.providers.filter((provider) =>
		providerCanHandleRagEmbedding(provider),
	);
	const selectedRagEmbeddingProvider =
		eligibleRagEmbeddingProviders.find(
			(provider) => provider.id === ragSettings.embeddingProviderId,
		) ?? null;
	const selectedRagEmbeddingProviderLabel =
		selectedRagEmbeddingProvider?.name ||
		selectedRagEmbeddingProvider?.model ||
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
	const selectedLlmProviderUsedByPersistedRag =
		selectedLlmProvider !== null &&
		persistedAppSettings.rag.embeddingProviderId === selectedLlmProvider.id;
	const selectedLlmProviderUsedByRagDraft =
		selectedLlmProvider !== null && ragSettings.embeddingProviderId === selectedLlmProvider.id;
	const selectedLlmProviderTriggersRagReindex =
		selectedLlmProvider !== null &&
		providerCanHandleRagEmbedding(selectedLlmProvider) &&
		(selectedLlmProviderUsedByPersistedRag || selectedLlmProviderUsedByRagDraft);
	const selectedAgent = acpAgents.find((agent) => agent.id === selectedAgentId) ?? null;
	const selectedMcpServer = mcpServers.find((server) => server.id === selectedMcpServerId) ?? null;
	const sectionSummaryText: Record<SettingsSectionId, string> = {
		general: "即时生效",
		prompts: promptsHaveUnsavedChanges ? "有草稿" : "已同步",
		llm: llmHasUnsavedChanges ? "有草稿" : `${llmSettings.providers.length} 条`,
		rag: ragHasUnsavedChanges ? "有草稿" : `目录 ${ragSourceDirectoryCount}`,
		acp: acpHasUnsavedChanges ? "有草稿" : `${acpAgents.length} 个 Agent`,
		mcp: mcpHasUnsavedChanges
			? "有草稿"
			: `${mcpServers.length} 个服务 / ${builtinMcpConfig.enabledModules.length} 个内置模块`,
		skills: skillCatalog.exists ? `${skillCatalog.skills.length} 个 skill` : "只读",
		about: "只读",
	};
	const sectionDescriptionText: Record<SettingsSectionId, string> = {
		general: "快捷键、通知、外观和 OCR。",
		prompts: "翻译与文档问答。",
		llm: "维护可复用的模型接入条目。",
		rag: "索引输入、目录与重建。",
		acp: "本地 Agent 启动命令与模式。",
		mcp: "全局 MCP 服务清单。",
		skills: "浏览公共 skill 目录。",
		about: "版本与产品定位。",
	};
	const activeSectionLabel =
		settingsSections.find((section) => section.id === activeSection)?.label ?? "设置";
	const selectedAgentIssueCount = selectedAgent
		? (acpValidation.agentIssues[selectedAgent.id]?.length ?? 0)
		: 0;
	const selectedMcpIssueCount = selectedMcpServer
		? (mcpValidation.serverIssues[selectedMcpServer.id]?.length ?? 0)
		: 0;
	const regularMcpServers = mcpServers;
	const builtinMcpTransportMeta =
		builtinMcpServerStatus !== null
			? getMcpTransportMeta(builtinMcpServerStatus.server.transport)
			: getMcpTransportMeta("http");
	const builtinMcpToggleDisabled =
		!builtinMcpConfig.enabled &&
		(!builtinMcpServerStatus?.running || builtinMcpServerStatus.server.transport !== "http");
	const selectedSkill = skillCatalog.skills.find((skill) => skill.id === selectedSkillId) ?? null;
	const selectedLlmFieldIssues = selectedLlmProvider
		? (llmValidation.providerFieldIssues[selectedLlmProvider.id] ?? {})
		: {};
	const selectedAgentFieldIssues = selectedAgent
		? (acpValidation.agentFieldIssues[selectedAgent.id] ?? {})
		: {};
	const selectedMcpFieldIssues = selectedMcpServer
		? (mcpValidation.serverFieldIssues[selectedMcpServer.id] ?? {})
		: {};
	const isMcpCreateMode =
		mcpPanelMode === "create" || (!selectedMcpServer && mcpServers.length === 0);
	const isMcpEditMode = !isMcpCreateMode && selectedMcpServer !== null;

	function buildAcpFieldRefKey(
		scope: "agent" | "server",
		id: string,
		field: McpFieldKey | AcpFieldKey,
	) {
		return `${scope}:${id}:${field}`;
	}

	function bindAcpFieldRef(
		scope: "agent" | "server",
		id: string,
		field: McpFieldKey | AcpFieldKey,
	) {
		const refKey = buildAcpFieldRefKey(scope, id, field);
		return (node: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | null) => {
			acpFieldRefs.current[refKey] = node;
		};
	}

	function focusAcpField(scope: "agent" | "server", id: string, field: McpFieldKey | AcpFieldKey) {
		const refKey = buildAcpFieldRefKey(scope, id, field);
		window.requestAnimationFrame(() => {
			acpFieldRefs.current[refKey]?.focus();
		});
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

	function locateFirstAcpIssue() {
		const nextIssue = findFirstAcpIssue(acpAgents);
		if (!nextIssue) {
			return;
		}

		setActiveSection("acp");
		setSelectedAgentId(nextIssue.agentId);
		focusAcpField("agent", nextIssue.agentId, nextIssue.fieldKey);
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
		const ragFieldOrder: RagFieldKey[] = [
			"embeddingProviderId",
			"sourceDirectories",
			"ignoreGlobs",
		];
		const nextField = ragFieldOrder.find((fieldKey) => ragValidation.fieldIssues[fieldKey]);
		if (!nextField) {
			return;
		}

		setActiveSection("rag");
		setSettingsError(
			ragValidation.fieldIssues[nextField] ?? ragValidation.issues[0] ?? "RAG 配置不合法",
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

	function handleAgentFieldChange(agentId: string, key: AcpFieldKey, value: string) {
		setAcpNotice(null);
		setAcpAgentsState((current) =>
			current.map((agent) => (agent.id === agentId ? { ...agent, [key]: value } : agent)),
		);
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

	function handleLlmProviderFieldChange<K extends keyof LlmProviderConfig>(
		providerId: string,
		key: K,
		value: LlmProviderConfig[K],
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

					let nextProvider = { ...provider, [key]: value };
					if (key === "baseUrl" || key === "apiKey") {
						nextProvider.modelIdentityHint = null;
					} else if (key === "model") {
						nextProvider.modelIdentityHint =
							findLlmModelOption(providerId, String(value))?.identityHint ?? null;
						const template = providerUsesBuiltinTemplate(nextProvider)
							? findBuiltinTemplate(builtinLlmTemplates, nextProvider.builtinPresetId)
							: null;
						nextProvider = applyBuiltinTemplateModelMetadata(
							nextProvider,
							findBuiltinTemplateModelByModelName(template, String(value)),
						);
					}

					return nextProvider;
				}),
			});
			return nextSettings;
		});
		setSettingsError(null);
	}

	function handleLlmProviderTemplateChange(providerId: string, templateId: string) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) => {
					if (provider.id !== providerId) {
						return provider;
					}

					if (!templateId) {
						return detachBuiltinTemplateFromProvider(provider);
					}

					const template = findBuiltinTemplate(builtinLlmTemplates, templateId);
					if (!template) {
						return provider;
					}

					return applyBuiltinTemplateModelMetadata(
						applyBuiltinTemplateToProvider(provider, template),
						findBuiltinTemplateModelByModelName(template, provider.model),
					);
				}),
			}),
		);
		setSettingsError(null);
	}

	function handleBuiltinProviderManagedBaseUrlChange(providerId: string, managed: boolean) {
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

					return {
						...provider,
						managedBaseUrl: managed,
						baseUrl: managed ? template.defaultBaseUrl : provider.baseUrl,
					};
				}),
			}),
		);
		setSettingsError(null);
	}

	function handleLlmProviderKindChange(providerId: string, kind: LlmProviderKind) {
		setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: current.providers.map((provider) =>
					provider.id === providerId
						? {
								...applyLlmProviderKind(provider, kind),
								builtinPresetModelId: null,
							}
						: provider,
				),
			}),
		);
		setSettingsError(null);
	}

	function handleLlmRouteProviderChange(
		key: "translationProviderId" | "questionAnswerProviderId",
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
			setSettingsError(getErrorMessage(error, "RAG 扫描失败"));
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
		if (enabled && !builtinMcpServerStatus?.running) {
			setMcpNotice({
				tone: "warn",
				text: builtinMcpServerStatus?.lastError || "内置 MCP server 还没运行成功，先修复运行状态。",
			});
			return;
		}

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

	function handleAddPresetAgent(option: (typeof acpAgentOptions)[number]) {
		const existingAgent = acpAgents.find(
			(agent) => agent.command.trim() === option.command && agent.launchMode === option.launchMode,
		);
		if (existingAgent) {
			setAcpNotice({
				tone: "warn",
				text: `${option.label} 已存在。相同启动命令和启动模式不需要重复添加。`,
			});
			setSelectedAgentId(existingAgent.id);
			return;
		}

		const nextAgent = createAgentDraft(option.label, option.command, option.launchMode);
		const nextAgents = [...acpAgents, nextAgent];
		setAcpNotice({
			tone: "info",
			text: `${option.label} 已加入 ACP Agent 草稿，还没写回 config.toml。`,
		});
		applyAgentDrafts(nextAgents, nextAgent.id);
	}

	function handleAddCustomAgent() {
		const nextAgent = createAgentDraft("ACP Agent", "");
		const nextAgents = [...acpAgents, nextAgent];
		setAcpNotice({
			tone: "info",
			text: "已创建新的自定义 ACP Agent 草稿。先补全名称和启动命令，再保存。",
		});
		applyAgentDrafts(nextAgents, nextAgent.id);
	}

	function handleApplyPresetSelection() {
		if (!selectedPresetOptionId) {
			return;
		}

		if (selectedPresetOptionId === "__custom__") {
			handleAddCustomAgent();
			setSelectedPresetOptionId("__custom__");
			return;
		}

		const option = acpAgentOptions.find((item) => item.id === selectedPresetOptionId);
		if (!option) {
			return;
		}

		handleAddPresetAgent(option);
		setSelectedPresetOptionId("__custom__");
	}

	function handleRemoveAgent(agentId: string) {
		setAcpNotice(null);
		const nextAgents = acpAgents.filter((agent) => agent.id !== agentId);
		applyAgentDrafts(
			nextAgents,
			selectedAgentId === agentId ? (nextAgents[0]?.id ?? null) : selectedAgentId,
		);
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
								<ShortcutSummaryCard
									onJumpToShortcuts={handleJumpToShortcutSettings}
									runtimeStatus={shortcutRuntimeStatus}
								/>
							) : null}

							{activeSection === "general" ? (
								<GeneralSettingsSection
									appearanceSettings={appearanceSettings}
									bindSectionBlockRef={bindSectionBlockRef}
									eligibleOcrProviders={eligibleOcrProviders}
									generalSettings={generalSettings}
									isRecording={isRecording}
									notificationSettings={notificationSettings}
									ocrSettings={ocrSettings}
									onSaveAppSettings={saveAppSettings}
									onSaveNotificationSettings={saveNotificationSettings}
									onSaveOcr={handleSaveOcr}
									onShortcutClick={handleShortcutClick}
									savingOcr={savingOcr}
									savingSettings={savingSettings}
									savingShortcutKey={savingShortcutKey}
									setOcrSettings={setOcrSettings}
									shortcutSettings={shortcutSettings}
									showLlmOcrFields={showLlmOcrFields}
									summarizeLlmProviderProfile={summarizeLlmProviderProfile}
								/>
							) : null}

							{activeSection === "prompts" ? (
								<PromptsSettingsSection
									bindSectionBlockRef={bindSectionBlockRef}
									defaultQuestionAnswerPrompt={defaultPromptsSettings.ragAnswerSystemPrompt}
									defaultTranslationPrompt={defaultPromptsSettings.translationPrompt}
									eligibleAiTaskProviders={eligibleAiTaskProviders}
									llmSettings={llmSettings}
									onDiscardQuestionAnswerDraft={handleDiscardQuestionAnswerDraft}
									onDiscardTranslationDraft={handleDiscardTranslationDraft}
									onLlmRouteProviderChange={handleLlmRouteProviderChange}
									onSaveQuestionAnswerConfig={handleSaveQuestionAnswerConfig}
									onSaveTranslationConfig={handleSaveTranslationConfig}
									onSelectSection={handleSelectSection}
									promptsSettings={promptsSettings}
									questionAnswerHasUnsavedChanges={questionAnswerHasUnsavedChanges}
									questionAnswerPromptExpanded={questionAnswerPromptExpanded}
									questionAnswerPromptSummary={questionAnswerPromptSummary}
									savingAiTaskConfig={savingAiTaskConfig}
									savingQuestionAnswerConfig={savingQuestionAnswerConfig}
									savingTranslationConfig={savingTranslationConfig}
									selectedQuestionAnswerProvider={selectedQuestionAnswerProvider}
									selectedTranslationProvider={selectedTranslationProvider}
									setPromptsSettings={setPromptsSettings}
									setQuestionAnswerPromptExpanded={setQuestionAnswerPromptExpanded}
									setTranslationPromptExpanded={setTranslationPromptExpanded}
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
									getLlmProviderKind={getLlmProviderKind}
									getLlmProviderKindLabel={getLlmProviderKindLabel}
									getLlmProviderModelPlaceholder={getLlmProviderModelPlaceholder}
									isLoadingSelectedLlmProviderModels={isLoadingSelectedLlmProviderModels}
									llmHasUnsavedChanges={llmHasUnsavedChanges}
									llmModelMenuRef={llmModelMenuRef}
									llmSettings={llmSettings}
									llmValidation={llmValidation}
									onAddLlmProvider={handleAddLlmProvider}
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
									onRemoveLlmProvider={handleRemoveLlmProvider}
									onSaveLlm={handleSaveLlm}
									onSelectLlmProvider={handleSelectLlmProvider}
									onToggleLlmModelMenu={handleToggleLlmModelMenu}
									openLlmModelPickerId={openLlmModelPickerId}
									providerCanHandleOcr={providerCanHandleOcr}
									providerHasResponsesModel={providerHasResponsesModel}
									providerIsLlmModel={providerIsLlmModel}
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
									selectedLlmProviderTriggersRagReindex={selectedLlmProviderTriggersRagReindex}
									selectedLlmProviderUsedByPersistedRag={selectedLlmProviderUsedByPersistedRag}
									summarizeLlmProviderProfile={summarizeLlmProviderProfile}
								/>
							) : null}

							{activeSection === "rag" ? (
								<RagSettingsSection
									bindRagFieldRef={bindRagFieldRef}
									bindSectionBlockRef={bindSectionBlockRef}
									defaultRagIgnoreGlobs={defaultRagIgnoreGlobs}
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

							{activeSection === "acp" ? (
								<AcpSettingsSection
									acpAgents={acpAgents}
									acpHasUnsavedChanges={acpHasUnsavedChanges}
									acpNotice={acpNotice}
									acpValidation={acpValidation}
									bindAcpFieldRef={bindAcpFieldRef}
									bindSectionBlockRef={bindSectionBlockRef}
									onAddCustomAgent={handleAddCustomAgent}
									onAgentFieldChange={handleAgentFieldChange}
									onApplyPresetSelection={handleApplyPresetSelection}
									onDiscardAcpDraft={handleDiscardAcpDraft}
									onLocateFirstAcpIssue={locateFirstAcpIssue}
									onRemoveAgent={handleRemoveAgent}
									onSaveAgent={handleSaveAgent}
									savingAgent={savingAgent}
									selectedAgent={selectedAgent}
									selectedAgentFieldIssues={selectedAgentFieldIssues}
									selectedAgentId={selectedAgentId}
									selectedAgentIssueCount={selectedAgentIssueCount}
									selectedPresetOptionId={selectedPresetOptionId}
									setSelectedAgentId={setSelectedAgentId}
									setSelectedPresetOptionId={setSelectedPresetOptionId}
								/>
							) : null}

							{activeSection === "mcp" ? (
								<McpSettingsSection
									bindAcpFieldRef={bindAcpFieldRef}
									bindMcpListOptionRef={bindMcpListOptionRef}
									bindSectionBlockRef={bindSectionBlockRef}
									builtinMcpConfig={builtinMcpConfig}
									builtinMcpServerStatus={builtinMcpServerStatus}
									builtinMcpToggleDisabled={builtinMcpToggleDisabled}
									builtinMcpTransportMeta={builtinMcpTransportMeta}
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

							{activeSection === "skills" ? (
								<SkillsSettingsSection
									bindSectionBlockRef={bindSectionBlockRef}
									onViewSkill={handleViewSkill}
									selectedSkill={selectedSkill}
									selectedSkillId={selectedSkillId}
									skillCatalog={skillCatalog}
									skillError={skillError}
								/>
							) : null}

							{activeSection === "about" ? (
								<AboutSettingsSection bindSectionBlockRef={bindSectionBlockRef} />
							) : null}
						</div>
					</div>
				</div>
			</section>
		</main>
	);
}
