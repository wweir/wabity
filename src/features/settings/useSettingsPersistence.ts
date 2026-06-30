import { useState } from "react";
import type { Dispatch, SetStateAction } from "react";

import {
	getErrorMessage,
	reconcileLlmSettings,
	reconcileOcrSettings,
	reconcileRagSettings,
} from "./settingsState";
import type {
	AcpAgentDraft,
	AcpInlineNotice,
	AcpMcpServerDraft,
	SavedAcpDraftState,
	SavedMcpDraftState,
} from "./settingsTypes";
import {
	buildAcpDraftSnapshot,
	buildAgentCommand,
	buildLlmDraftSnapshot,
	buildRagDraftSnapshot,
	buildMcpDraftSnapshot,
	buildSavedAcpDraftState,
	buildSavedLlmDraftState,
	buildSavedMcpDraftState,
	buildSavedRagDraftState,
	cloneLlmProviderDraft,
	cloneSavedAcpDraftState,
	cloneSavedMcpDraftState,
	createMcpServerDraftFromConfig,
	deriveProgramFromCommand,
	serializeMcpServerDraft,
} from "./settingsState";
import type {
	AppSettings,
	AppearanceSettings,
	BuiltinMcpConfig,
	GeneralSettings,
	LlmSettings,
	LlmProviderModelEntry,
	NotificationSettings,
	OcrSettings,
	PromptsSettings,
	RagSettings,
} from "../../lib/tauri/types";
import { setAcpAgents, setAcpMcpServers, setAppSettings } from "../../lib/tauri/client";

interface UseSettingsPersistenceArgs {
	acpAgents: AcpAgentDraft[];
	acpValidationIssueCount: number;
	applyAgentDrafts: (nextAgents: AcpAgentDraft[], nextSelectedAgentId: string | null) => void;
	applyMcpServerDrafts: (
		nextServers: AcpMcpServerDraft[],
		nextSelectedServerId: string | null,
	) => void;
	builtinMcpConfig: BuiltinMcpConfig;
	defaultAgentId: string | null;
	generalSettings: GeneralSettings;
	llmSettings: LlmSettings;
	mcpServers: AcpMcpServerDraft[];
	ocrSettings: OcrSettings;
	appearanceSettings: AppearanceSettings;
	persistedAppSettings: AppSettings;
	promptsSettings: PromptsSettings;
	ragSettings: RagSettings;
	savedAcpSnapshot: string;
	savedAcpState: SavedAcpDraftState;
	savedLlmSnapshot: string;
	savedMcpSnapshot: string;
	savedMcpState: SavedMcpDraftState;
	selectedAgentId: string | null;
	selectedLlmProviderId: string | null;
	selectedMcpServerId: string | null;
	setAcpNotice: (notice: AcpInlineNotice | null) => void;
	setBuiltinMcpConfig: (
		value: BuiltinMcpConfig | ((current: BuiltinMcpConfig) => BuiltinMcpConfig),
	) => void;
	setDefaultAgentId: (value: string | null) => void;
	setGeneralSettings: (settings: GeneralSettings) => void;
	setAppearanceSettings: (settings: AppearanceSettings) => void;
	setLlmModelErrors: (value: Record<string, string>) => void;
	setLlmModelOptions: Dispatch<SetStateAction<Record<string, LlmProviderModelEntry[]>>>;
	setLlmSettings: (value: LlmSettings | ((current: LlmSettings) => LlmSettings)) => void;
	setLoadingLlmModelProviderId: (value: string | null) => void;
	setMcpNotice: (notice: AcpInlineNotice | null) => void;
	setMcpPanelMode: (mode: "create" | "edit") => void;
	setNotificationSettings: (settings: NotificationSettings) => void;
	setOcrSettings: (value: OcrSettings | ((current: OcrSettings) => OcrSettings)) => void;
	setOpenLlmModelPickerId: (value: string | null) => void;
	setPersistedAppSettings: (settings: AppSettings) => void;
	setPromptsSettings: (
		value: PromptsSettings | ((current: PromptsSettings) => PromptsSettings),
	) => void;
	setRagSettings: (value: RagSettings | ((current: RagSettings) => RagSettings)) => void;
	setSavedAcpSnapshot: (snapshot: string) => void;
	setSavedAcpState: (state: SavedAcpDraftState) => void;
	setSavedLlmSnapshot: (snapshot: string) => void;
	setSavedLlmState: (state: ReturnType<typeof buildSavedLlmDraftState>) => void;
	setSavedMcpSnapshot: (snapshot: string) => void;
	setSavedMcpState: (state: SavedMcpDraftState) => void;
	setSavedRagSnapshot: (snapshot: string) => void;
	setSavedRagState: (state: ReturnType<typeof buildSavedRagDraftState>) => void;
	setSelectedLlmProviderId: (value: string | null) => void;
	setSettingsError: (value: string | null) => void;
	syncAppearanceState: (appearance: AppearanceSettings) => void;
	llmValidationIssueCount: number;
	locateFirstLlmIssue: () => void;
	locateFirstAcpIssue: () => void;
	locateFirstMcpIssue: () => void;
	ragValidationIssueCount: number;
	locateFirstRagIssue: () => void;
	mcpValidationIssueCount: number;
}

export function useSettingsPersistence(args: UseSettingsPersistenceArgs) {
	const [savingSettings, setSavingSettings] = useState(false);
	const [savingTranslationConfig, setSavingTranslationConfig] = useState(false);
	const [savingQuestionAnswerConfig, setSavingQuestionAnswerConfig] = useState(false);
	const [savingLlm, setSavingLlm] = useState(false);
	const [savingOcr, setSavingOcr] = useState(false);
	const [savingRag, setSavingRag] = useState(false);
	const [savingAgent, setSavingAgent] = useState(false);
	const [savingMcp, setSavingMcp] = useState(false);

	async function persistAppSettings(
		nextSettings: AppSettings,
		setSaving: (value: boolean) => void,
		fallbackMessage: string,
		options: {
			adoptPromptKeys: Array<keyof PromptsSettings>;
			adoptLlmProviders: boolean;
			adoptLlmRouteKeys: Array<"translationModelId" | "questionAnswerModelId">;
			adoptProviderDependencies: boolean;
			adoptOcr: boolean;
			adoptRag: boolean;
		},
	) {
		setSaving(true);
		args.setSettingsError(null);
		try {
			const saved = await setAppSettings(nextSettings);
			const nextSavedLlmSettings = reconcileLlmSettings(saved.llm);
			args.setGeneralSettings(saved.general);
			args.setNotificationSettings(saved.notification);
			args.setAppearanceSettings(saved.appearance);
			args.syncAppearanceState(saved.appearance);
			if (options.adoptPromptKeys.length > 0) {
				args.setPromptsSettings((current) => ({
					...current,
					...Object.fromEntries(options.adoptPromptKeys.map((key) => [key, saved.prompts[key]])),
				}));
			}
			if (
				options.adoptLlmProviders ||
				options.adoptLlmRouteKeys.length > 0 ||
				options.adoptProviderDependencies
			) {
				args.setLlmSettings((current) =>
					reconcileLlmSettings({
						providers: options.adoptLlmProviders
							? nextSavedLlmSettings.providers
							: current.providers,
						translationModelId:
							options.adoptProviderDependencies ||
							options.adoptLlmRouteKeys.includes("translationModelId")
								? nextSavedLlmSettings.translationModelId
								: current.translationModelId,
						questionAnswerModelId:
							options.adoptProviderDependencies ||
							options.adoptLlmRouteKeys.includes("questionAnswerModelId")
								? nextSavedLlmSettings.questionAnswerModelId
								: current.questionAnswerModelId,
					}),
				);
			}
			if (options.adoptProviderDependencies) {
				args.setOcrSettings((current) =>
					reconcileOcrSettings(
						{
							...current,
							llmModelId: saved.ocr.llmModelId,
						},
						saved.llm.providers,
					),
				);
				args.setRagSettings((current) =>
					reconcileRagSettings(
						{
							...current,
							embeddingModelId: saved.rag.embeddingModelId,
						},
						saved.llm.providers,
					),
				);
			}
			if (options.adoptLlmProviders) {
				args.setSelectedLlmProviderId(
					nextSavedLlmSettings.providers.some(
						(provider) => provider.id === args.selectedLlmProviderId,
					)
						? args.selectedLlmProviderId
						: (nextSavedLlmSettings.providers[0]?.id ?? null),
				);
				args.setSavedLlmSnapshot(buildLlmDraftSnapshot(nextSavedLlmSettings));
				args.setSavedLlmState(buildSavedLlmDraftState(nextSavedLlmSettings.providers));
			}
			if (options.adoptOcr) {
				args.setOcrSettings(saved.ocr);
			}
			if (options.adoptRag) {
				const nextRagSettings = reconcileRagSettings(saved.rag, nextSavedLlmSettings.providers);
				args.setRagSettings(nextRagSettings);
				args.setSavedRagSnapshot(buildRagDraftSnapshot(nextRagSettings));
				args.setSavedRagState(buildSavedRagDraftState(nextRagSettings));
			}
			args.setPersistedAppSettings({
				...saved,
				llm: nextSavedLlmSettings,
			});
		} catch (error: unknown) {
			args.setSettingsError(getErrorMessage(error, fallbackMessage));
		} finally {
			setSaving(false);
		}
	}

	async function saveAppSettings(
		nextGeneral: GeneralSettings,
		nextNotification: NotificationSettings,
		nextAppearance: AppearanceSettings,
	) {
		await persistAppSettings(
			{
				general: nextGeneral,
				notification: nextNotification,
				appearance: nextAppearance,
				prompts: args.persistedAppSettings.prompts,
				llm: args.persistedAppSettings.llm,
				ocr: args.persistedAppSettings.ocr,
				rag: args.persistedAppSettings.rag,
			},
			setSavingSettings,
			"设置保存失败",
			{
				adoptPromptKeys: [],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: [],
				adoptProviderDependencies: false,
				adoptOcr: false,
				adoptRag: false,
			},
		);
	}

	async function saveNotificationSettings(nextNotification: NotificationSettings) {
		await saveAppSettings(args.generalSettings, nextNotification, args.appearanceSettings);
	}

	async function handleSaveTranslationConfig() {
		await persistAppSettings(
			{
				general: args.persistedAppSettings.general,
				notification: args.persistedAppSettings.notification,
				appearance: args.persistedAppSettings.appearance,
				prompts: {
					translationPrompt: args.promptsSettings.translationPrompt,
					ragAnswerSystemPrompt: args.persistedAppSettings.prompts.ragAnswerSystemPrompt,
				},
				llm: {
					...args.persistedAppSettings.llm,
					translationModelId: args.llmSettings.translationModelId,
				},
				ocr: args.persistedAppSettings.ocr,
				rag: args.persistedAppSettings.rag,
			},
			setSavingTranslationConfig,
			"翻译配置保存失败",
			{
				adoptPromptKeys: ["translationPrompt"],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: ["translationModelId"],
				adoptProviderDependencies: false,
				adoptOcr: false,
				adoptRag: false,
			},
		);
	}

	async function handleSaveQuestionAnswerConfig() {
		await persistAppSettings(
			{
				general: args.persistedAppSettings.general,
				notification: args.persistedAppSettings.notification,
				appearance: args.persistedAppSettings.appearance,
				prompts: {
					translationPrompt: args.persistedAppSettings.prompts.translationPrompt,
					ragAnswerSystemPrompt: args.promptsSettings.ragAnswerSystemPrompt,
				},
				llm: {
					...args.persistedAppSettings.llm,
					questionAnswerModelId: args.llmSettings.questionAnswerModelId,
				},
				ocr: args.persistedAppSettings.ocr,
				rag: args.persistedAppSettings.rag,
			},
			setSavingQuestionAnswerConfig,
			"文档问答配置保存失败",
			{
				adoptPromptKeys: ["ragAnswerSystemPrompt"],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: ["questionAnswerModelId"],
				adoptProviderDependencies: false,
				adoptOcr: false,
				adoptRag: false,
			},
		);
	}

	async function handleSaveLlm() {
		if (args.llmValidationIssueCount > 0) {
			args.locateFirstLlmIssue();
			return;
		}

		await persistAppSettings(
			{
				general: args.persistedAppSettings.general,
				notification: args.persistedAppSettings.notification,
				appearance: args.persistedAppSettings.appearance,
				prompts: args.persistedAppSettings.prompts,
				llm: {
					providers: args.llmSettings.providers,
					translationModelId: args.persistedAppSettings.llm.translationModelId,
					questionAnswerModelId: args.persistedAppSettings.llm.questionAnswerModelId,
				},
				ocr: args.persistedAppSettings.ocr,
				rag: args.persistedAppSettings.rag,
			},
			setSavingLlm,
			"模型接入配置保存失败",
			{
				adoptPromptKeys: [],
				adoptLlmProviders: true,
				adoptLlmRouteKeys: [],
				adoptProviderDependencies: true,
				adoptOcr: false,
				adoptRag: false,
			},
		);
	}

	async function handleSaveOcr() {
		await persistAppSettings(
			{
				general: args.persistedAppSettings.general,
				notification: args.persistedAppSettings.notification,
				appearance: args.persistedAppSettings.appearance,
				prompts: args.persistedAppSettings.prompts,
				llm: args.persistedAppSettings.llm,
				ocr: args.ocrSettings,
				rag: args.persistedAppSettings.rag,
			},
			setSavingOcr,
			"OCR 配置保存失败",
			{
				adoptPromptKeys: [],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: [],
				adoptProviderDependencies: false,
				adoptOcr: true,
				adoptRag: false,
			},
		);
	}

	async function handleSaveRag() {
		if (args.ragValidationIssueCount > 0) {
			args.locateFirstRagIssue();
			return;
		}

		await persistAppSettings(
			{
				general: args.persistedAppSettings.general,
				notification: args.persistedAppSettings.notification,
				appearance: args.persistedAppSettings.appearance,
				prompts: args.persistedAppSettings.prompts,
				llm: args.persistedAppSettings.llm,
				ocr: args.persistedAppSettings.ocr,
				rag: args.ragSettings,
			},
			setSavingRag,
			"RAG 配置保存失败",
			{
				adoptPromptKeys: [],
				adoptLlmProviders: false,
				adoptLlmRouteKeys: [],
				adoptProviderDependencies: false,
				adoptOcr: false,
				adoptRag: true,
			},
		);
	}

	function handleDiscardTranslationDraft() {
		args.setPromptsSettings((current) => ({
			...current,
			translationPrompt: args.persistedAppSettings.prompts.translationPrompt,
		}));
		args.setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				translationModelId: args.persistedAppSettings.llm.translationModelId,
			}),
		);
		args.setSettingsError(null);
	}

	function handleDiscardQuestionAnswerDraft() {
		args.setPromptsSettings((current) => ({
			...current,
			ragAnswerSystemPrompt: args.persistedAppSettings.prompts.ragAnswerSystemPrompt,
		}));
		args.setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				questionAnswerModelId: args.persistedAppSettings.llm.questionAnswerModelId,
			}),
		);
		args.setSettingsError(null);
	}

	function handleDiscardLlmDraft() {
		args.setLlmSettings((current) =>
			reconcileLlmSettings({
				...current,
				providers: args.persistedAppSettings.llm.providers.map(cloneLlmProviderDraft),
			}),
		);
		args.setSelectedLlmProviderId(
			args.persistedAppSettings.llm.providers.some(
				(provider) => provider.id === args.selectedLlmProviderId,
			)
				? args.selectedLlmProviderId
				: (args.persistedAppSettings.llm.providers[0]?.id ?? null),
		);
		args.setLlmModelOptions({});
		args.setLlmModelErrors({});
		args.setLoadingLlmModelProviderId(null);
		args.setOpenLlmModelPickerId(null);
		args.setSettingsError(null);
	}

	function handleDiscardRagDraft() {
		args.setRagSettings(
			reconcileRagSettings(
				{
					sourceDirectories: [...args.persistedAppSettings.rag.sourceDirectories],
					ignoreGlobs: [...args.persistedAppSettings.rag.ignoreGlobs],
					embeddingModelId: args.persistedAppSettings.rag.embeddingModelId,
				},
				args.llmSettings.providers,
			),
		);
		args.setSettingsError(null);
	}

	function handleDiscardAcpDraft() {
		const restored = cloneSavedAcpDraftState(args.savedAcpState);
		args.setAcpNotice(null);
		args.setDefaultAgentId(restored.defaultAgentId);
		args.applyAgentDrafts(
			restored.agents,
			restored.agents.some((agent) => agent.id === args.selectedAgentId)
				? args.selectedAgentId
				: (restored.agents[0]?.id ?? null),
		);
		args.setSettingsError(null);
	}

	function handleDiscardMcpDraft() {
		const restored = cloneSavedMcpDraftState(args.savedMcpState);
		args.setMcpNotice(null);
		args.setBuiltinMcpConfig(restored.builtin);
		const nextSelectedServerId = restored.servers.some(
			(server) => server.id === args.selectedMcpServerId,
		)
			? args.selectedMcpServerId
			: (restored.servers[0]?.id ?? null);
		args.applyMcpServerDrafts(restored.servers, nextSelectedServerId);
		args.setMcpPanelMode(nextSelectedServerId ? "edit" : "create");
		args.setSettingsError(null);
	}

	async function handleSaveAgent() {
		if (args.acpValidationIssueCount > 0) {
			args.locateFirstAcpIssue();
			return;
		}

		setSavingAgent(true);
		args.setSettingsError(null);
		args.setAcpNotice(null);
		try {
			const preservedDefaultAgentId =
				args.defaultAgentId && args.acpAgents.some((agent) => agent.id === args.defaultAgentId)
					? args.defaultAgentId
					: null;
			const normalizedAgents = args.acpAgents.map((agent) => ({
				id: agent.id,
				name: agent.name.trim(),
				program: deriveProgramFromCommand(agent.command),
				args: [],
				shellCommand: agent.command.trim() || null,
				launchMode: agent.launchMode,
			}));
			const nextCatalog = await setAcpAgents(normalizedAgents, preservedDefaultAgentId);
			const nextDraftAgents = nextCatalog.agents.map((agent) => ({
				id: agent.id,
				name: agent.name,
				command: buildAgentCommand(agent),
				launchMode: agent.launchMode,
			}));
			const nextDefaultAgentId = nextCatalog.defaultAgentId ?? nextCatalog.agents[0]?.id ?? null;
			args.setDefaultAgentId(nextDefaultAgentId);
			args.applyAgentDrafts(
				nextDraftAgents,
				nextDraftAgents.some((agent) => agent.id === args.selectedAgentId)
					? args.selectedAgentId
					: (nextDraftAgents[0]?.id ?? null),
			);
			args.setSavedAcpSnapshot(buildAcpDraftSnapshot(nextDraftAgents));
			args.setSavedAcpState(buildSavedAcpDraftState(nextDraftAgents, nextDefaultAgentId));
		} catch (error: unknown) {
			args.setSettingsError(getErrorMessage(error, "Pi Agent 配置保存失败"));
		} finally {
			setSavingAgent(false);
		}
	}

	async function handleSaveMcp() {
		if (args.mcpValidationIssueCount > 0) {
			args.locateFirstMcpIssue();
			return;
		}

		setSavingMcp(true);
		args.setSettingsError(null);
		args.setMcpNotice(null);
		try {
			const nextCatalog = await setAcpMcpServers(
				args.mcpServers.map(serializeMcpServerDraft),
				args.builtinMcpConfig,
			);
			const nextDraftServers = nextCatalog.servers.map(createMcpServerDraftFromConfig);
			const nextSelectedServerId = nextDraftServers.some(
				(server) => server.id === args.selectedMcpServerId,
			)
				? args.selectedMcpServerId
				: (nextDraftServers[0]?.id ?? null);
			args.applyMcpServerDrafts(nextDraftServers, nextSelectedServerId);
			args.setMcpPanelMode(nextSelectedServerId ? "edit" : "create");
			args.setBuiltinMcpConfig(nextCatalog.builtin);
			args.setSavedMcpSnapshot(buildMcpDraftSnapshot(nextDraftServers, nextCatalog.builtin));
			args.setSavedMcpState(buildSavedMcpDraftState(nextDraftServers, nextCatalog.builtin));
		} catch (error: unknown) {
			args.setSettingsError(getErrorMessage(error, "MCP 配置保存失败"));
		} finally {
			setSavingMcp(false);
		}
	}

	return {
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
		persistAppSettings,
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
	};
}
