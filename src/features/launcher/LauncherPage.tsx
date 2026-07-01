import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent, MouseEvent as ReactMouseEvent } from "react";
import { flushSync } from "react-dom";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
	activateAcpSession,
	armLauncherBlurAutoHideSuppression,
	cancelAcpSession,
	cancelScreenshotReview,
	chooseWorkspaceDirectory,
	closeAcpSession,
	confirmScreenshotReview,
	createAcpSession,
	deleteClipboardHistoryEntry,
	dismissClipboardHistoryPanel as dismissClipboardHistoryWindow,
	executeAction,
	getClipboardHistory,
	getScreenshotReviewPreview,
	getAcpSessionDetail,
	getWorkspace,
	hideLauncherWindow,
	insertClipboardHistoryTextIntoLauncher,
	isDesktopRuntimeAvailable,
	launchApp,
	onClipboardHistoryUpdated,
	onExecutionProgress,
	onInsertClipboardHistoryTextIntoLauncher,
	onOpenClipboardHistoryPanel,
	onRevealLauncherMainPanel,
	listAcpSessions,
	onOcrTranslationStream,
	onOcrTranslationStarted,
	onOcrTranslationResult,
	onLauncherFailure,
	onScreenshotReviewStarted,
	retryScreenshotReview,
	subscribeAcpSessionRemovals,
	subscribeAcpSessionUpdates,
	openDocumentReference,
	onWorkspaceUpdated,
	sendAcpPrompt,
	setLauncherBlurAutoHideEnabled,
	setWorkspace,
	toggleClipboardHistoryEntryPin,
	takeAcpRestoreNotices,
	pasteClipboardHistoryEntry,
	setAcpSessionConfigOption,
	setAcpSessionMode,
} from "../../lib/tauri/client";
import { useAutoResizeWindow } from "../../lib/tauri/useAutoResizeWindow";
import type {
	AcpRestoreNotice,
	AcpSessionMessage,
	AcpSessionDetail,
	AcpSessionSummary,
	ClipboardHistoryEntry,
	ClipboardHistorySnapshot,
	ClipboardHistorySelectionMode,
	ShortcutRuntimeStatus,
	ScreenshotReviewPayload,
	WorkspaceState,
} from "../../lib/tauri/types";
import type {
	ActionDescriptor,
	ExecutionConversationState,
	ExecutionConversationTurn,
	ExecutionResult,
	FloatingPanelOffset,
	InputMode,
	RagCitation,
	RagRetrievalSummary,
} from "./types";
import {
	killProcessActionDescriptor,
	ragAnswerActionDescriptor,
	translateActionDescriptor,
} from "./actionCatalog";
import {
	buildQuery,
	deriveActionCompletion,
	deriveJsonPrettyPreview,
	deriveSelectedSlashActionPayload,
	deriveSlashActionPayload,
	deriveFileCompletion,
	findSlashCommandPrefixRange,
	findSlashCommandTokenRange,
	findActiveFileToken,
	getErrorMessage,
	isActionQuery,
	isAppSearchReady,
	isFileSearchReady,
	isKillSearchReady,
	parseSlashActionInput,
	type SuggestionMode,
	replaceTextRange,
} from "./query";
import { deriveShortcutTranslationInputState } from "./shortcutTranslation";
import { usesInlineInputControl, usesTextareaInputControl } from "./inputMode";
import {
	clamp,
	clampCaretIndex,
	completionPanelMaxWidth,
	completionPanelVerticalGap,
	launcherFrameMaxWidth,
	launcherFrameMinWidth,
	longestVisibleLine,
	measureCaretPosition,
	measureTextWidth,
	multilineInputMinHeight,
	readPx,
	resolveLauncherScrollableHeights,
} from "./layout";
import {
	buildSessionTriggerSummary,
	upsertSessionDetailRecord,
	upsertSessionSummary,
	visibleSessionDotsLimit,
} from "./sessions";
import { buildWorkspaceBreadcrumbs } from "./workspace";
import { useDismissOnPointerDownOutside, useFloatingPanelOffset } from "./useFloatingPanel";
import { useLauncherSuggestions } from "./useLauncherSuggestions";
import { useLauncherSelectionEffects } from "./useLauncherSelectionEffects";
import { useRagRuntimeStatus } from "./useRagRuntimeStatus";
import { LauncherHeader } from "./components/LauncherHeader";
import { LauncherComposer } from "./components/LauncherComposer";
import { LauncherFeedback } from "./components/LauncherFeedback";
import { RestoreNoticeList } from "./components/RestoreNoticeList";
import { WorkspacePickerPanel } from "./components/WorkspacePickerPanel";
import { LauncherSuggestionsSection } from "./components/LauncherSuggestionsSection";
import { LauncherSessionSection } from "./components/LauncherSessionSection";
import { LauncherClipboardSection } from "./components/LauncherClipboardSection";
import { ScreenshotReviewPanel } from "./components/ScreenshotReviewPanel";
import "./launcher.css";
import { isRagAnswerStructuredPayload } from "./types";
import {
	applyClientEffect,
	buildLauncherStatusBarState,
	buildQaAssistantMessageBlocks,
	buildRagRuntimeStatusText,
	defaultInputMode,
	deriveKillCompletion,
	derivePrimaryActionState,
	extractQaPrompt,
	getPinnedClipboardHotkeyIndex,
	getRecentClipboardHotkeyIndex,
	launcherCompletionOptionIdPrefix,
	launcherCompletionPopupId,
	QA_RESULT_BLUR_AUTO_HIDE_RESTORE_AFTER_SETTLE_MS,
	QA_RESULT_BLUR_AUTO_HIDE_SUPPRESSION_MS,
	resolveLauncherOperationStatus,
	type PrimaryActionState,
} from "./launcherPageModel";

interface LauncherPageProps {
	active?: boolean;
	launcherPinned?: boolean;
	onLauncherPinnedChange?: (pinned: boolean) => void | Promise<void>;
	onOpenSettings?: () => void;
	shortcutRuntimeStatus: ShortcutRuntimeStatus;
	windowKind: "main" | "clipboard_history";
}

function aggregateScreenshotReviewBlockText(
	review: ScreenshotReviewPayload | null,
	selectedBlockIds: string[],
): string {
	if (!review || selectedBlockIds.length === 0) {
		return "";
	}

	const blocksById = new Map(review.ocr.blocks.map((block) => [block.id, block]));
	return selectedBlockIds
		.map((blockId) => blocksById.get(blockId)?.text.trim() ?? "")
		.filter(Boolean)
		.join("\n");
}

export function LauncherPage({
	active = true,
	launcherPinned = false,
	onLauncherPinnedChange,
	onOpenSettings,
	shortcutRuntimeStatus,
	windowKind,
}: LauncherPageProps) {
	const isClipboardWindow = windowKind === "clipboard_history";
	const launcherViewActive = isClipboardWindow || active;
	const [inputMode, setInputMode] = useState<InputMode>(defaultInputMode);
	const [workspace, setWorkspaceState] = useState<WorkspaceState>({
		rootPath: "",
		recentRoots: [],
		homePath: null,
		displayHomeAsTilde: false,
	});
	const [rawText, setRawText] = useState("");
	const [sessionSummaries, setSessionSummaries] = useState<AcpSessionSummary[]>([]);
	const [sessionDetails, setSessionDetails] = useState<Record<string, AcpSessionDetail>>({});
	const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
	const [activeSlashAction, setActiveSlashAction] = useState<ActionDescriptor | null>(null);
	const [result, setResult] = useState<ExecutionResult | null>(null);
	const [qaMessages, setQaMessages] = useState<AcpSessionMessage[]>([]);
	const [qaAutoResizeFrozen, setQaAutoResizeFrozen] = useState(false);
	const [qaConversationState, setQaConversationState] = useState<ExecutionConversationState | null>(
		null,
	);
	const [ragConversation, setRagConversation] = useState<ExecutionConversationTurn[]>([]);
	const [qaRetrieval, setQaRetrieval] = useState<RagRetrievalSummary | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [operationPending, setOperationPending] = useState(false);
	const [agentActionPending, setAgentActionPending] = useState(false);
	const [shortcutTranslationPending, setShortcutTranslationPending] = useState(false);
	const [screenshotReview, setScreenshotReview] = useState<ScreenshotReviewPayload | null>(null);
	const [screenshotReviewPreview, setScreenshotReviewPreview] = useState<string | null>(null);
	const [screenshotReviewText, setScreenshotReviewText] = useState("");
	const [selectedScreenshotReviewBlockIds, setSelectedScreenshotReviewBlockIds] = useState<
		string[]
	>([]);
	const [screenshotReviewBusy, setScreenshotReviewBusy] = useState(false);
	const [screenshotReviewCopyFeedback, setScreenshotReviewCopyFeedback] = useState<string | null>(
		null,
	);
	const [frameWidth, setFrameWidth] = useState(launcherFrameMaxWidth);
	const [caretIndex, setCaretIndex] = useState(0);
	const [completionOffset, setCompletionOffset] = useState<FloatingPanelOffset>({
		x: 0,
		y: 0,
		width: completionPanelMaxWidth,
	});
	const [creatingSession, setCreatingSession] = useState(false);
	const [runtimeControlPendingKey, setRuntimeControlPendingKey] = useState<string | null>(null);
	const [sessionPanelOpen, setSessionPanelOpen] = useState(false);
	const [workspacePickerOpen, setWorkspacePickerOpen] = useState(false);
	const [restoreNotices, setRestoreNotices] = useState<AcpRestoreNotice[]>([]);
	const [clipboardHistory, setClipboardHistory] = useState<ClipboardHistorySnapshot>({
		pinnedEntries: [],
		recentEntries: [],
	});
	const [clipboardPanelOpen, setClipboardPanelOpen] = useState(windowKind === "clipboard_history");
	const [clipboardSelectionMode, setClipboardSelectionMode] =
		useState<ClipboardHistorySelectionMode>("paste_externally");
	const [selectedClipboardEntryId, setSelectedClipboardEntryId] = useState<string | null>(null);
	const [latestSubmittedText, setLatestSubmittedText] = useState<string | null>(null);
	const [operationStatusText, setOperationStatusText] = useState<string | null>(null);
	const ragRuntimeStatus = useRagRuntimeStatus();
	const frameRef = useRef<HTMLElement | null>(null);
	const shellRef = useRef<HTMLElement | null>(null);
	const sessionPanelRef = useRef<HTMLElement | null>(null);
	const sessionPanelTriggerRef = useRef<HTMLButtonElement | null>(null);
	const clipboardPanelRef = useRef<HTMLElement | null>(null);
	const workspacePickerTriggerRef = useRef<HTMLButtonElement | null>(null);
	const workspacePickerPanelRef = useRef<HTMLDivElement | null>(null);
	const inputAnchorRef = useRef<HTMLDivElement | null>(null);
	const inputRef = useRef<HTMLInputElement | HTMLTextAreaElement | null>(null);
	const pendingFocusAnimationFrameRef = useRef<number | null>(null);
	const pendingFocusTimeoutRef = useRef<number | null>(null);
	const qaBlurAutoHideRestoreTimeoutRef = useRef<number | null>(null);
	const launcherBlurAutoHideEnabledRef = useRef(true);
	const suspendReactiveLauncherInputFocusRef = useRef(false);
	const launcherResetEpochRef = useRef(0);
	const activeTrackedRequestEpochRef = useRef<number | null>(null);
	const shortcutTranslationEpochRef = useRef<number | null>(null);
	const completionListRef = useRef<HTMLUListElement | null>(null);
	const sessionLogRef = useRef<HTMLDivElement | null>(null);
	const screenshotReviewSessionIdRef = useRef<string | null>(null);
	const screenshotReviewTextManuallyEditedRef = useRef(false);
	const pendingSelectionRef = useRef<number | null>(null);
	const boundedCaretIndex = useMemo(
		() => clampCaretIndex(rawText, caretIndex),
		[caretIndex, rawText],
	);
	const textBeforeCaret = useMemo(
		() => rawText.slice(0, boundedCaretIndex),
		[boundedCaretIndex, rawText],
	);
	const activeFileToken = useMemo(
		() => findActiveFileToken(rawText, boundedCaretIndex),
		[boundedCaretIndex, rawText],
	);
	const fullQuery = useMemo(() => buildQuery(inputMode, rawText), [inputMode, rawText]);
	const suggestionQuery = useMemo(
		() => buildQuery(inputMode, textBeforeCaret),
		[inputMode, textBeforeCaret],
	);
	const jsonPreview = useMemo(
		() => (activeSessionId ? null : deriveJsonPrettyPreview(rawText)),
		[activeSessionId, rawText],
	);
	const desktopRuntimeAvailable = isDesktopRuntimeAvailable();
	const clipboardPanelVisible = isClipboardWindow || clipboardPanelOpen;
	const screenshotReviewOpen = screenshotReview !== null;
	const fileMode = activeFileToken !== null;
	const launcherMode = activeSessionId === null;
	const textStartsWithSlash = textBeforeCaret.trimStart().startsWith("/");
	const pendingSlashAction =
		launcherMode && activeSlashAction && !textStartsWithSlash ? activeSlashAction : null;
	const killSlashInputMatch = useMemo(
		() =>
			launcherMode
				? parseSlashActionInput(textBeforeCaret, killProcessActionDescriptor.aliases)
				: null,
		[launcherMode, textBeforeCaret],
	);
	const killSlashCommandCommitted = Boolean(
		killSlashInputMatch &&
		(killSlashInputMatch.commandToken === killSlashInputMatch.alias.toLowerCase() ||
			killSlashInputMatch.content.length > 0),
	);
	const killSlashPayload = useMemo(
		() =>
			launcherMode
				? deriveSelectedSlashActionPayload(
						rawText,
						boundedCaretIndex,
						killProcessActionDescriptor.aliases,
					)
				: null,
		[boundedCaretIndex, launcherMode, rawText],
	);
	const killSearchQuery =
		pendingSlashAction?.id === "kill_process" ? textBeforeCaret : (killSlashPayload?.value ?? "");
	const markdownPreview = useMemo(() => {
		if (activeSessionId || pendingSlashAction?.id !== "markdown_render") {
			return null;
		}

		return rawText.length > 0 ? rawText : null;
	}, [activeSessionId, pendingSlashAction, rawText]);
	const currentFileNeedle = activeFileToken?.needle ?? "";
	const suggestionMode: SuggestionMode = fileMode
		? "file"
		: shortcutTranslationPending || screenshotReviewOpen
			? "none"
			: !launcherMode
				? "none"
				: pendingSlashAction
					? pendingSlashAction.id === "kill_process"
						? "kill"
						: "none"
					: killSlashCommandCommitted
						? "kill"
						: isActionQuery(textBeforeCaret)
							? "action"
							: isAppSearchReady(textBeforeCaret)
								? "app"
								: "none";
	const {
		actionMatches,
		appMatches,
		fileMatches,
		killMatches,
		resetSuggestions,
		selectedIndex,
		setSelectedIndex,
		suggestionLoading,
		suggestionsHidden,
		setSuggestionsHidden,
	} = useLauncherSuggestions({
		currentFileNeedle,
		killSearchQuery,
		launcherMode,
		setError,
		suggestionMode,
		suggestionQuery,
		textBeforeCaret,
	});
	const visibleActionMatches = useMemo(
		() => (launcherMode ? actionMatches : []),
		[actionMatches, launcherMode],
	);
	const visibleFileMatches = fileMatches;
	const visibleAppMatches = useMemo(
		() => (suggestionMode === "app" ? appMatches : []),
		[appMatches, suggestionMode],
	);
	const visibleKillMatches = useMemo(
		() => (suggestionMode === "kill" ? killMatches : []),
		[killMatches, suggestionMode],
	);
	const suggestionCount =
		suggestionMode === "file"
			? visibleFileMatches.length
			: suggestionMode === "action"
				? visibleActionMatches.length
				: suggestionMode === "app"
					? visibleAppMatches.length
					: suggestionMode === "kill"
						? visibleKillMatches.length
						: 0;
	const canTextMatchCompletions =
		suggestionMode === "file"
			? isFileSearchReady(currentFileNeedle)
			: suggestionMode === "kill"
				? isKillSearchReady(killSearchQuery)
				: suggestionMode !== "none";
	const hasSuggestions = !suggestionsHidden && suggestionCount > 0 && canTextMatchCompletions;
	const activeCompletionOptionId = hasSuggestions
		? `${launcherCompletionOptionIdPrefix}-${suggestionMode}-${selectedIndex}`
		: undefined;
	const visibleCount = hasSuggestions ? suggestionCount : 0;
	const selectedActionMatch = visibleActionMatches[selectedIndex];
	const selectedFileMatch = visibleFileMatches[selectedIndex];
	const selectedAppMatch = visibleAppMatches[selectedIndex];
	const selectedKillMatch = visibleKillMatches[selectedIndex];
	const completionText =
		suggestionMode === "file"
			? deriveFileCompletion(textBeforeCaret, selectedFileMatch)
			: suggestionMode === "action"
				? deriveActionCompletion(textBeforeCaret, selectedActionMatch)
				: suggestionMode === "kill"
					? deriveKillCompletion(selectedKillMatch)
					: null;
	const hasCompletion = completionText !== null && completionText !== textBeforeCaret;
	const activeSession = activeSessionId ? (sessionDetails[activeSessionId] ?? null) : null;
	const activeSessionStatus = activeSession?.session.status ?? null;
	const activeSessionBusy =
		Boolean(activeSessionId) && (activeSessionStatus === "running" || agentActionPending);
	const activeSessionScrollAnchor =
		activeSession?.session.sessionId ?? (qaMessages.length > 0 ? `qa:${qaMessages.length}` : null);
	const activeSessionSummary =
		sessionSummaries.find((session) => session.sessionId === activeSessionId) ??
		activeSession?.session ??
		null;
	const workspaceBreadcrumbs = useMemo(() => buildWorkspaceBreadcrumbs(workspace), [workspace]);
	const flattenedClipboardEntries = useMemo(
		() => [...clipboardHistory.pinnedEntries, ...clipboardHistory.recentEntries],
		[clipboardHistory],
	);
	const visibleSessionDots = sessionSummaries.slice(0, visibleSessionDotsLimit);
	const overflowSessionCount = Math.max(0, sessionSummaries.length - visibleSessionDotsLimit);
	const sessionRunningCount = sessionSummaries.filter(
		(session) => session.status === "running",
	).length;
	const sessionAttentionCount = sessionSummaries.filter((session) => session.attention).length;
	const sessionTriggerSummary = buildSessionTriggerSummary(
		sessionSummaries,
		activeSessionSummary,
		sessionRunningCount,
		sessionAttentionCount,
	);
	const recentWorkspaceRoots = workspace.recentRoots.filter((path) => path !== workspace.rootPath);
	const workspacePickerOffset = useFloatingPanelOffset({
		open: workspacePickerOpen,
		triggerRef: workspacePickerTriggerRef,
		shellRef,
		horizontalAlign: "left",
		gap: 8,
		minWidth: 240,
		extraWidth: 148,
		initialWidth: 280,
		watchKey: `${frameWidth}:${recentWorkspaceRoots.length}`,
	});
	const sessionPanelOffset = useFloatingPanelOffset({
		open: sessionPanelOpen,
		triggerRef: sessionPanelTriggerRef,
		shellRef,
		horizontalAlign: "right",
		gap: 10,
		minWidth: 400,
		maxWidth: 480,
		extraWidth: 176,
		initialWidth: 400,
		watchKey: `${frameWidth}:${sessionSummaries.length}:${activeSessionId ?? ""}`,
	});
	const sessionCanSend = activeSession ? activeSession.session.status === "idle" : false;
	const agentConfigured = true;
	const appSearchActive = suggestionMode === "app";
	const appSearchPending =
		appSearchActive &&
		isAppSearchReady(textBeforeCaret) &&
		visibleAppMatches.length === 0 &&
		suggestionLoading;
	const hasAppMatches = visibleAppMatches.length > 0;
	const shouldFallbackToRagAnswer =
		launcherMode &&
		!fileMode &&
		!activeSessionId &&
		!pendingSlashAction &&
		!textStartsWithSlash &&
		!isActionQuery(textBeforeCaret) &&
		rawText.trim().length > 0 &&
		(!appSearchActive || (!appSearchPending && !hasAppMatches));
	const primaryActionShortcutLabel = "Enter";
	const agentActionShortcutLabel = "Alt+Enter";
	const primaryActionState: PrimaryActionState = useMemo(
		() =>
			derivePrimaryActionState({
				activeSessionId,
				appSearchActive,
				fileMode,
				killSearchQuery,
				pendingSlashAction,
				rawText,
				selectedActionMatch,
				selectedAppMatch,
				selectedFileMatch,
				sessionCanSend,
				shouldFallbackToRagAnswer,
				suggestionMode,
			}),
		[
			activeSessionId,
			appSearchActive,
			fileMode,
			killSearchQuery,
			pendingSlashAction,
			rawText,
			selectedActionMatch,
			selectedAppMatch,
			selectedFileMatch,
			sessionCanSend,
			shouldFallbackToRagAnswer,
			suggestionMode,
		],
	);
	const ragRuntimeBar = useMemo(
		() => buildRagRuntimeStatusText(ragRuntimeStatus),
		[ragRuntimeStatus],
	);
	const visibleQaMessages = useMemo(
		() => qaMessages.filter((message) => message.role !== "system"),
		[qaMessages],
	);
	const shouldShowQaMessages =
		!activeSession && !jsonPreview && !markdownPreview && !result && visibleQaMessages.length > 0;
	const showShortcutHint =
		rawText.trim().length === 0 &&
		!clipboardPanelVisible &&
		!operationPending &&
		!shortcutTranslationPending &&
		!screenshotReviewOpen &&
		result === null &&
		activeSessionId === null &&
		!pendingSlashAction;
	const statusBarState = useMemo(
		() =>
			buildLauncherStatusBarState({
				activeSession,
				activeSessionBusy,
				clipboardHistoryShortcut: shortcutRuntimeStatus.open_clipboard_history.configuredShortcut,
				creatingSession,
				error,
				hasSettingsAction: Boolean(onOpenSettings),
				latestSubmittedText,
				operationStatusText,
				ragRuntimeBar,
				shouldShowQaMessages,
				showShortcutHint,
				workspace,
			}),
		[
			activeSession,
			activeSessionBusy,
			creatingSession,
			error,
			latestSubmittedText,
			onOpenSettings,
			operationStatusText,
			ragRuntimeBar,
			showShortcutHint,
			shouldShowQaMessages,
			shortcutRuntimeStatus.open_clipboard_history.configuredShortcut,
			workspace,
		],
	);
	const showTranslateAction = launcherMode;
	const showCancelActiveSession = Boolean(activeSessionId) && activeSessionBusy;
	const primaryActionDependsOnSuggestions =
		primaryActionState.kind === "insert_path" ||
		primaryActionState.kind === "search_path" ||
		primaryActionState.kind === "run_action" ||
		primaryActionState.kind === "launch_app";
	const canTriggerAgentAction =
		agentConfigured &&
		!activeSessionId &&
		!operationPending &&
		!shortcutTranslationPending &&
		!screenshotReviewOpen &&
		!creatingSession &&
		rawText.trim().length > 0;
	const canRunPrimaryAction =
		!operationPending &&
		!shortcutTranslationPending &&
		!screenshotReviewOpen &&
		!creatingSession &&
		!(suggestionLoading && primaryActionDependsOnSuggestions) &&
		primaryActionState.enabled &&
		!(primaryActionState.kind === "launch_app" && appSearchPending && !hasAppMatches);
	const canRunTranslateAction =
		launcherMode &&
		!operationPending &&
		!shortcutTranslationPending &&
		!screenshotReviewOpen &&
		!creatingSession &&
		!fileMode &&
		rawText.trim().length > 0;
	const showAgentActionButton = launcherMode;
	const canRunAgentAction = canTriggerAgentAction;
	const agentActionLabel = "Agent";
	const agentActionTitle = `${agentActionLabel} (${agentActionShortcutLabel})`;
	const showAgentActionShortcut = true;

	const clearScheduledLauncherInputFocus = useCallback(() => {
		if (typeof window !== "undefined" && pendingFocusAnimationFrameRef.current !== null) {
			window.cancelAnimationFrame(pendingFocusAnimationFrameRef.current);
			pendingFocusAnimationFrameRef.current = null;
		}

		if (typeof window !== "undefined" && pendingFocusTimeoutRef.current !== null) {
			window.clearTimeout(pendingFocusTimeoutRef.current);
			pendingFocusTimeoutRef.current = null;
		}
	}, []);

	const clearScheduledQaBlurAutoHideRestore = useCallback(() => {
		if (typeof window === "undefined" || qaBlurAutoHideRestoreTimeoutRef.current === null) {
			return;
		}

		window.clearTimeout(qaBlurAutoHideRestoreTimeoutRef.current);
		qaBlurAutoHideRestoreTimeoutRef.current = null;
	}, []);

	const restoreLauncherBlurAutoHide = useCallback(
		(errorMessage: string) => {
			clearScheduledQaBlurAutoHideRestore();
			if (!desktopRuntimeAvailable || launcherBlurAutoHideEnabledRef.current) {
				suspendReactiveLauncherInputFocusRef.current = false;
				return;
			}

			suspendReactiveLauncherInputFocusRef.current = false;
			launcherBlurAutoHideEnabledRef.current = true;
			void setLauncherBlurAutoHideEnabled(true).catch((error: unknown) => {
				console.warn(errorMessage, error);
			});
		},
		[clearScheduledQaBlurAutoHideRestore, desktopRuntimeAvailable],
	);

	const scheduleLauncherBlurAutoHideRestore = useCallback(
		(delayMs: number) => {
			if (
				typeof window === "undefined" ||
				!desktopRuntimeAvailable ||
				launcherBlurAutoHideEnabledRef.current
			) {
				return;
			}

			clearScheduledQaBlurAutoHideRestore();
			qaBlurAutoHideRestoreTimeoutRef.current = window.setTimeout(
				() => {
					qaBlurAutoHideRestoreTimeoutRef.current = null;
					restoreLauncherBlurAutoHide(
						"failed to re-enable launcher blur auto-hide after QA result settled",
					);
				},
				Math.max(1, Math.ceil(delayMs)),
			);
		},
		[clearScheduledQaBlurAutoHideRestore, desktopRuntimeAvailable, restoreLauncherBlurAutoHide],
	);

	const updateRawText = useCallback(
		(
			nextValue: string,
			nextCaretIndex?: number,
			options?: {
				preserveQaPresentation?: boolean;
				resumeReactiveLauncherInputFocus?: boolean;
			},
		) => {
			const preserveQaPresentation = options?.preserveQaPresentation ?? false;
			const resumeReactiveLauncherInputFocus = options?.resumeReactiveLauncherInputFocus ?? true;
			if (resumeReactiveLauncherInputFocus) {
				suspendReactiveLauncherInputFocusRef.current = false;
			}
			if (!preserveQaPresentation) {
				setQaAutoResizeFrozen(false);
				restoreLauncherBlurAutoHide("failed to re-enable launcher blur auto-hide");
			}
			setRawText(nextValue);
			setSuggestionsHidden(false);
			if (typeof nextCaretIndex === "number") {
				setCaretIndex(nextCaretIndex);
			}
			setError(null);
			if (!activeSessionId) {
				setResult(null);
			}
		},
		[activeSessionId, restoreLauncherBlurAutoHide, setSuggestionsHidden],
	);

	const focusLauncherInput = useCallback(() => {
		const inputElement = inputRef.current;
		if (!inputElement) {
			return;
		}

		const nextSelection = clampCaretIndex(
			inputElement.value,
			pendingSelectionRef.current ?? inputElement.selectionEnd ?? inputElement.value.length,
		);

		inputElement.focus({ preventScroll: true });
		inputElement.setSelectionRange(nextSelection, nextSelection);
		pendingSelectionRef.current = null;
	}, []);

	const scheduleLauncherInputFocus = useCallback(() => {
		if (typeof window === "undefined") {
			return;
		}

		clearScheduledLauncherInputFocus();
		pendingFocusAnimationFrameRef.current = window.requestAnimationFrame(() => {
			pendingFocusAnimationFrameRef.current = null;
			focusLauncherInput();
			pendingFocusTimeoutRef.current = window.setTimeout(() => {
				pendingFocusTimeoutRef.current = null;
				focusLauncherInput();
			}, 60);
		});
	}, [clearScheduledLauncherInputFocus, focusLauncherInput]);

	const beginTrackedLauncherRequest = useCallback(() => {
		const requestEpoch = launcherResetEpochRef.current;
		activeTrackedRequestEpochRef.current = requestEpoch;
		return requestEpoch;
	}, []);

	const isTrackedLauncherRequestCurrent = useCallback(
		(requestEpoch: number) => launcherResetEpochRef.current === requestEpoch,
		[],
	);

	const endTrackedLauncherRequest = useCallback((requestEpoch: number) => {
		if (activeTrackedRequestEpochRef.current === requestEpoch) {
			activeTrackedRequestEpochRef.current = null;
		}
	}, []);

	const resetQaConversation = useCallback(() => {
		setQaAutoResizeFrozen(false);
		restoreLauncherBlurAutoHide(
			"failed to re-enable launcher blur auto-hide while resetting QA state",
		);
		setQaMessages([]);
		setQaConversationState(null);
		setRagConversation([]);
		setQaRetrieval(null);
	}, [restoreLauncherBlurAutoHide]);

	const resetLauncherStateForExplicitDismiss = useCallback(() => {
		const resetEpoch = launcherResetEpochRef.current + 1;
		launcherResetEpochRef.current = resetEpoch;
		activeTrackedRequestEpochRef.current = null;
		shortcutTranslationEpochRef.current = resetEpoch - 1;
		const screenshotReviewSessionId = screenshotReviewSessionIdRef.current;
		if (screenshotReviewSessionId) {
			void cancelScreenshotReview(screenshotReviewSessionId).catch((error: unknown) => {
				console.warn("failed to cancel screenshot review while dismissing launcher", error);
			});
		}
		clearScheduledLauncherInputFocus();
		pendingSelectionRef.current = null;
		setInputMode(defaultInputMode);
		setRawText("");
		setCaretIndex(0);
		setResult(null);
		setError(null);
		setActiveSlashAction(null);
		setSuggestionsHidden(false);
		resetSuggestions(true);
		setLatestSubmittedText(null);
		setOperationStatusText(null);
		setOperationPending(false);
		setAgentActionPending(false);
		setShortcutTranslationPending(false);
		screenshotReviewSessionIdRef.current = null;
		screenshotReviewTextManuallyEditedRef.current = false;
		setScreenshotReview(null);
		setScreenshotReviewPreview(null);
		setScreenshotReviewText("");
		setSelectedScreenshotReviewBlockIds([]);
		setScreenshotReviewBusy(false);
		setScreenshotReviewCopyFeedback(null);
		setCreatingSession(false);
		setSessionPanelOpen(false);
		setClipboardPanelOpen(false);
		setClipboardSelectionMode("paste_externally");
		setWorkspacePickerOpen(false);
		setActiveSessionId(null);
		setSessionSummaries((current) =>
			current.map((session) => (session.isActive ? { ...session, isActive: false } : session)),
		);
		resetQaConversation();

		if (!activeSessionId) {
			return;
		}

		void activateAcpSession(null)
			.then((summaries) => {
				if (!isTrackedLauncherRequestCurrent(resetEpoch)) {
					return;
				}

				setSessionSummaries(summaries);
			})
			.catch((error: unknown) => {
				if (isTrackedLauncherRequestCurrent(resetEpoch)) {
					console.warn("failed to clear active Agent session while dismissing launcher", error);
				}
			});
	}, [
		activeSessionId,
		clearScheduledLauncherInputFocus,
		isTrackedLauncherRequestCurrent,
		resetSuggestions,
		resetQaConversation,
		setSuggestionsHidden,
	]);

	const dismissLauncher = useCallback(
		async (options?: { resetQaConversation?: boolean }) => {
			if (options?.resetQaConversation) {
				resetQaConversation();
			}
			setClipboardPanelOpen(false);
			setClipboardSelectionMode("paste_externally");
			await hideLauncherWindow();
		},
		[resetQaConversation],
	);

	const prepareShortcutTranslationView = useCallback(
		(sourceText: string, sourceMode: "ocr" | "selection") => {
			const nextInputState = deriveShortcutTranslationInputState(sourceText, sourceMode);
			setQaAutoResizeFrozen(false);
			pendingSelectionRef.current = nextInputState.caretIndex;
			setInputMode(nextInputState.inputMode);
			updateRawText(nextInputState.rawText, nextInputState.caretIndex, {
				preserveQaPresentation: false,
				resumeReactiveLauncherInputFocus: false,
			});
			setLatestSubmittedText(sourceText.trim() || "快捷翻译");
			setResult(null);
			setError(null);
			setActiveSlashAction(null);
			resetQaConversation();
			scheduleLauncherInputFocus();
		},
		[resetQaConversation, scheduleLauncherInputFocus, updateRawText],
	);

	const rawTextRef = useRef(rawText);
	const boundedCaretIndexRef = useRef(boundedCaretIndex);
	const clipboardPanelVisibleRef = useRef(clipboardPanelVisible);
	const resetQaConversationRef = useRef(resetQaConversation);
	const scheduleLauncherInputFocusRef = useRef(scheduleLauncherInputFocus);
	const updateRawTextRef = useRef(updateRawText);
	const prepareShortcutTranslationViewRef = useRef(prepareShortcutTranslationView);

	useLayoutEffect(() => {
		rawTextRef.current = rawText;
	}, [rawText]);

	useLayoutEffect(() => {
		boundedCaretIndexRef.current = boundedCaretIndex;
	}, [boundedCaretIndex]);

	useLayoutEffect(() => {
		clipboardPanelVisibleRef.current = clipboardPanelVisible;
	}, [clipboardPanelVisible]);

	useLayoutEffect(() => {
		resetQaConversationRef.current = resetQaConversation;
	}, [resetQaConversation]);

	useLayoutEffect(() => {
		scheduleLauncherInputFocusRef.current = scheduleLauncherInputFocus;
	}, [scheduleLauncherInputFocus]);

	useLayoutEffect(() => {
		updateRawTextRef.current = updateRawText;
	}, [updateRawText]);

	useLayoutEffect(() => {
		prepareShortcutTranslationViewRef.current = prepareShortcutTranslationView;
	}, [prepareShortcutTranslationView]);

	const syncScrollableLayoutCaps = useCallback(() => {
		const shellElement = shellRef.current;
		const inputElement = inputRef.current;
		const { inputMaxHeight, outputMaxHeight } = resolveLauncherScrollableHeights();

		shellElement?.style.setProperty("--launcher-input-max-height", `${inputMaxHeight}px`);
		shellElement?.style.setProperty("--launcher-output-max-height", `${outputMaxHeight}px`);

		if (!(inputElement instanceof HTMLTextAreaElement) || !usesTextareaInputControl(inputMode)) {
			return;
		}

		inputElement.style.height = "0px";
		inputElement.style.height = `${clamp(inputElement.scrollHeight, multilineInputMinHeight, inputMaxHeight)}px`;
		inputElement.style.overflowY = inputElement.scrollHeight > inputMaxHeight ? "auto" : "hidden";
	}, [inputMode]);

	useLayoutEffect(() => {
		syncScrollableLayoutCaps();
	}, [rawText, inputMode, syncScrollableLayoutCaps]);

	useLayoutEffect(() => {
		if (typeof window === "undefined") {
			return;
		}

		window.addEventListener("resize", syncScrollableLayoutCaps);

		return () => {
			window.removeEventListener("resize", syncScrollableLayoutCaps);
		};
	}, [syncScrollableLayoutCaps]);

	useLayoutEffect(() => {
		const inputElement = inputRef.current;
		if (!(inputElement instanceof HTMLTextAreaElement) || !usesTextareaInputControl(inputMode)) {
			return;
		}

		inputElement.style.overflowX = "hidden";
	}, [inputMode]);

	useLayoutEffect(() => {
		const frameElement = frameRef.current;
		const inputElement = inputRef.current;
		if (!frameElement || !inputElement) {
			return;
		}

		const frameStyles = getComputedStyle(frameElement);
		const inputStyles = getComputedStyle(inputElement);
		const frameBorderWidth =
			readPx(frameStyles.borderLeftWidth) + readPx(frameStyles.borderRightWidth);
		const framePaddingWidth = readPx(frameStyles.paddingLeft) + readPx(frameStyles.paddingRight);
		const inputPaddingWidth = readPx(inputStyles.paddingLeft) + readPx(inputStyles.paddingRight);
		const placeholder = inputElement.getAttribute("placeholder") ?? "";
		const inputContent = longestVisibleLine(rawText, placeholder);
		const inputOuterWidth =
			measureTextWidth(inputContent, inputStyles) + inputPaddingWidth + frameBorderWidth;

		const contentOuterWidth = Array.from(
			frameElement.querySelectorAll<HTMLElement>(
				".workspace-row, .control-row, .session-log, .status-line, .result-line",
			),
		).reduce(
			(maxWidth, element) =>
				Math.max(maxWidth, element.scrollWidth + framePaddingWidth + frameBorderWidth),
			0,
		);

		const nextFrameWidth = Math.ceil(
			clamp(
				Math.max(inputOuterWidth, contentOuterWidth),
				launcherFrameMinWidth,
				launcherFrameMaxWidth,
			),
		);

		setFrameWidth((currentWidth) =>
			currentWidth === nextFrameWidth ? currentWidth : nextFrameWidth,
		);
	}, [
		rawText,
		actionMatches,
		fileMatches,
		appMatches,
		sessionSummaries,
		sessionDetails,
		result,
		error,
		suggestionLoading,
		operationPending,
		agentActionPending,
		creatingSession,
		operationStatusText,
		inputMode,
	]);

	useLayoutEffect(() => {
		const inputElement = inputRef.current;
		if (!inputElement) {
			return;
		}

		const nextSelection = pendingSelectionRef.current;
		if (nextSelection === null) {
			return;
		}

		inputElement.focus({ preventScroll: true });
		inputElement.setSelectionRange(nextSelection, nextSelection);
		pendingSelectionRef.current = null;
	}, [rawText, inputMode]);

	useLayoutEffect(() => {
		if (typeof document === "undefined") {
			return;
		}

		if (document.visibilityState === "hidden") {
			return;
		}

		scheduleLauncherInputFocus();
	}, [inputMode, scheduleLauncherInputFocus]);

	useLayoutEffect(() => {
		const inputElement = inputRef.current;
		const frameElement = frameRef.current;
		const shellElement = shellRef.current;
		if (!inputElement || !frameElement || !shellElement) {
			return;
		}

		let nextX: number;
		let nextY: number;
		const nextWidth = Math.min(completionPanelMaxWidth, inputElement.clientWidth);

		if (inputElement instanceof HTMLTextAreaElement && usesTextareaInputControl(inputMode)) {
			const caretPosition = measureCaretPosition(inputElement, boundedCaretIndex);
			nextX = clamp(caretPosition.left, 0, Math.max(0, inputElement.clientWidth - nextWidth));
			const frameTop = frameElement.offsetTop;
			const inputTopInFrame = inputElement.offsetTop;
			nextY =
				frameTop +
				inputTopInFrame +
				caretPosition.top +
				caretPosition.lineHeight +
				completionPanelVerticalGap;
		} else {
			nextX = 0;
			const frameTop = frameElement.offsetTop;
			const inputTopInFrame = inputElement.offsetTop;
			const inputHeight = inputElement.offsetHeight;
			nextY = frameTop + inputTopInFrame + inputHeight + completionPanelVerticalGap;
		}

		setCompletionOffset((currentOffset) => {
			if (
				currentOffset.x === nextX &&
				currentOffset.y === nextY &&
				currentOffset.width === nextWidth
			) {
				return currentOffset;
			}

			return {
				x: nextX,
				y: nextY,
				width: nextWidth,
			};
		});
	}, [boundedCaretIndex, frameWidth, rawText, inputMode]);

	useLauncherSelectionEffects({
		completionListRef,
		flattenedClipboardEntries,
		hasSuggestions,
		selectedClipboardEntryId,
		selectedIndex,
		setSelectedClipboardEntryId,
		setSelectedIndex,
		setSuggestionsHidden,
		suggestionCount,
		suggestionMode,
		textBeforeCaret,
		visibleSuggestionKeys: [
			visibleActionMatches,
			visibleAppMatches,
			visibleFileMatches,
			visibleKillMatches,
		],
	});

	const handleQaResultResizeSettled = useCallback(() => {
		if (!qaAutoResizeFrozen || launcherBlurAutoHideEnabledRef.current) {
			return;
		}

		scheduleLauncherBlurAutoHideRestore(QA_RESULT_BLUR_AUTO_HIDE_RESTORE_AFTER_SETTLE_MS);
	}, [qaAutoResizeFrozen, scheduleLauncherBlurAutoHideRestore]);

	useEffect(() => {
		if (windowKind === "clipboard_history") {
			setClipboardPanelOpen(true);
		}
	}, [windowKind]);

	useAutoResizeWindow(shellRef, {
		enabled: launcherViewActive,
		allowShrink: !qaAutoResizeFrozen,
		onResizeSettled: handleQaResultResizeSettled,
		resetKey: clipboardPanelVisible
			? "clipboard-panel"
			: screenshotReviewOpen
				? "screenshot-review"
				: "launcher",
	});

	useEffect(
		() => () => clearScheduledQaBlurAutoHideRestore(),
		[clearScheduledQaBlurAutoHideRestore],
	);

	useEffect(() => {
		let active = true;
		const unlistenCallbacks: Array<() => void> = [];
		const registerUnlisten = (label: string, promise: Promise<(() => void) | null>) => {
			void promise
				.then((unlisten) => {
					if (!unlisten) {
						return;
					}

					if (!active) {
						unlisten();
						return;
					}

					unlistenCallbacks.push(unlisten);
				})
				.catch((error: unknown) => {
					console.warn(`failed to subscribe ${label}`, error);
				});
		};

		void getWorkspace().then((nextWorkspace) => {
			setWorkspaceState(nextWorkspace);
			resetQaConversationRef.current();
		});
		void getClipboardHistory().then((snapshot) => {
			setClipboardHistory(snapshot);
		});
		void takeAcpRestoreNotices().then((notices) => {
			setRestoreNotices(notices);
		});
		void listAcpSessions().then((summaries) => {
			setSessionSummaries(summaries);
			const activeSession = summaries.find((summary) => summary.isActive);
			setActiveSessionId(activeSession?.sessionId ?? null);
		});

		registerUnlisten(
			"workspace updates",
			onWorkspaceUpdated((nextWorkspace) => {
				setWorkspaceState(nextWorkspace);
				resetQaConversationRef.current();
			}),
		);
		registerUnlisten(
			"clipboard history updates",
			onClipboardHistoryUpdated((snapshot) => {
				setClipboardHistory(snapshot);
			}),
		);
		registerUnlisten(
			"clipboard history insert into launcher",
			onInsertClipboardHistoryTextIntoLauncher((payload) => {
				const currentRawText = rawTextRef.current;
				const currentCaretIndex = boundedCaretIndexRef.current;
				const nextRawText = replaceTextRange(
					currentRawText,
					currentCaretIndex,
					currentCaretIndex,
					payload.text,
				);
				pendingSelectionRef.current = nextRawText.caretIndex;
				updateRawTextRef.current(nextRawText.value, nextRawText.caretIndex);
				setClipboardPanelOpen(false);
				scheduleLauncherInputFocusRef.current();
				setError(null);
			}),
		);
		registerUnlisten(
			"open clipboard history panel",
			onOpenClipboardHistoryPanel((payload) => {
				setClipboardSelectionMode(payload.selectionMode);
				setClipboardPanelOpen(true);
				setWorkspacePickerOpen(false);
				setSessionPanelOpen(false);
				setSuggestionsHidden(true);
			}),
		);
		registerUnlisten(
			"reveal launcher main panel",
			onRevealLauncherMainPanel(() => {
				setClipboardPanelOpen(false);
				setClipboardSelectionMode("paste_externally");
				const sessionId = screenshotReviewSessionIdRef.current;
				if (sessionId) {
					void cancelScreenshotReview(sessionId).catch((error: unknown) => {
						console.warn("failed to cancel screenshot review while revealing launcher", error);
					});
				}
				screenshotReviewSessionIdRef.current = null;
				screenshotReviewTextManuallyEditedRef.current = false;
				setScreenshotReview(null);
				setScreenshotReviewPreview(null);
				setScreenshotReviewText("");
				setSelectedScreenshotReviewBlockIds([]);
				setScreenshotReviewBusy(false);
				setScreenshotReviewCopyFeedback(null);
			}),
		);
		registerUnlisten(
			"OCR translation started",
			onOcrTranslationStarted((payload) => {
				shortcutTranslationEpochRef.current = launcherResetEpochRef.current;
				setShortcutTranslationPending(true);
				screenshotReviewSessionIdRef.current = null;
				screenshotReviewTextManuallyEditedRef.current = false;
				setScreenshotReview(null);
				setScreenshotReviewPreview(null);
				setScreenshotReviewText("");
				setSelectedScreenshotReviewBlockIds([]);
				setScreenshotReviewBusy(false);
				setScreenshotReviewCopyFeedback(null);
				setOperationStatusText("模型请求中 · 正在翻译文本");
				setResult(null);
				prepareShortcutTranslationViewRef.current(payload.sourceText, payload.sourceMode);
				setLatestSubmittedText(payload.sourceText.trim() || "快捷翻译");
			}),
		);
		registerUnlisten(
			"OCR translation stream",
			onOcrTranslationStream((payload) => {
				if (
					shortcutTranslationEpochRef.current === null ||
					shortcutTranslationEpochRef.current !== launcherResetEpochRef.current
				) {
					return;
				}

				setOperationStatusText("模型响应中 · 正在输出译文");
				setLatestSubmittedText(payload.sourceText.trim() || "快捷翻译");
				setResult({
					status: "success",
					primaryText: payload.partialText,
					secondaryText: null,
					structuredPayload: null,
					nextActions: ["copy_text"],
					shouldCloseLauncher: false,
				});
			}),
		);
		registerUnlisten(
			"OCR translation result",
			onOcrTranslationResult((payload) => {
				if (
					shortcutTranslationEpochRef.current === null ||
					shortcutTranslationEpochRef.current !== launcherResetEpochRef.current
				) {
					return;
				}

				shortcutTranslationEpochRef.current = null;
				setShortcutTranslationPending(false);
				setOperationStatusText(null);
				setLatestSubmittedText(payload.sourceText.trim() || "快捷翻译");
				setResult(payload.result);
			}),
		);
		registerUnlisten(
			"screenshot review started",
			onScreenshotReviewStarted((payload) => {
				const selectedBlockIds = payload.ocr.blocks.map((block) => block.id);
				shortcutTranslationEpochRef.current = null;
				setShortcutTranslationPending(false);
				screenshotReviewSessionIdRef.current = payload.sessionId;
				screenshotReviewTextManuallyEditedRef.current = false;
				setScreenshotReview(payload);
				setScreenshotReviewPreview(null);
				setScreenshotReviewText(
					aggregateScreenshotReviewBlockText(payload, selectedBlockIds) || payload.ocr.text,
				);
				setSelectedScreenshotReviewBlockIds(selectedBlockIds);
				setScreenshotReviewBusy(false);
				setScreenshotReviewCopyFeedback(null);
				setOperationStatusText(null);
				setResult(null);
				setError(null);
				setClipboardPanelOpen(false);
				setWorkspacePickerOpen(false);
				setSessionPanelOpen(false);
				void getScreenshotReviewPreview(payload.sessionId)
					.then((preview) => {
						if (screenshotReviewSessionIdRef.current !== payload.sessionId) {
							return;
						}

						setScreenshotReviewPreview(preview || null);
					})
					.catch((previewError: unknown) => {
						if (screenshotReviewSessionIdRef.current !== payload.sessionId) {
							return;
						}

						setError(getErrorMessage(previewError, "加载截图预览失败"));
					});
			}),
		);
		registerUnlisten(
			"launcher failure",
			onLauncherFailure((payload) => {
				shortcutTranslationEpochRef.current = null;
				screenshotReviewSessionIdRef.current = null;
				screenshotReviewTextManuallyEditedRef.current = false;
				setScreenshotReview(null);
				setScreenshotReviewPreview(null);
				setScreenshotReviewText("");
				setSelectedScreenshotReviewBlockIds([]);
				setScreenshotReviewBusy(false);
				setScreenshotReviewCopyFeedback(null);
				setShortcutTranslationPending(false);
				setOperationStatusText(null);
				setError(payload.message);
				scheduleLauncherInputFocusRef.current();
			}),
		);
		registerUnlisten(
			"execution progress",
			onExecutionProgress((payload) => {
				if (
					activeTrackedRequestEpochRef.current === null ||
					activeTrackedRequestEpochRef.current !== launcherResetEpochRef.current
				) {
					return;
				}

				setOperationStatusText(payload.statusText);
				if (payload.actionId === "translate_text" && typeof payload.partialText === "string") {
					setResult({
						status: "success",
						primaryText: payload.partialText,
						secondaryText: null,
						structuredPayload: null,
						nextActions: ["copy_text"],
						shouldCloseLauncher: false,
					});
					return;
				}

				if (payload.actionId === "rag_answer" && typeof payload.partialText === "string") {
					if (payload.partialText.length === 0) {
						setResult(null);
						return;
					}

					setResult({
						status: "success",
						primaryText: payload.partialText,
						secondaryText: null,
						structuredPayload: {
							render: "markdown",
						},
						nextActions: ["copy_text"],
						shouldCloseLauncher: false,
					});
				}
			}),
		);
		return () => {
			active = false;
			unlistenCallbacks.forEach((unlisten) => unlisten());
		};
	}, [setSelectedIndex, setSuggestionsHidden]);

	useEffect(() => {
		let active = true;
		let updatesUnlisten: (() => void) | null = null;
		let removalsUnlisten: (() => void) | null = null;

		void subscribeAcpSessionUpdates((detail) => {
			setSessionDetails((current) => upsertSessionDetailRecord(current, detail));
			setSessionSummaries((current) => upsertSessionSummary(current, detail.session));
			if (detail.session.isActive) {
				setActiveSessionId(detail.session.sessionId);
			}
		})
			.then((unlisten) => {
				if (!active) {
					unlisten?.();
					return;
				}

				updatesUnlisten = unlisten;
			})
			.catch((error: unknown) => {
				console.warn("failed to subscribe Agent session updates", error);
			});

		void subscribeAcpSessionRemovals((sessionId) => {
			setSessionDetails((current) => {
				const next = { ...current };
				delete next[sessionId];
				return next;
			});
			setSessionSummaries((current) => current.filter((item) => item.sessionId !== sessionId));
			setActiveSessionId((current) => (current === sessionId ? null : current));
		})
			.then((unlisten) => {
				if (!active) {
					unlisten?.();
					return;
				}

				removalsUnlisten = unlisten;
			})
			.catch((error: unknown) => {
				console.warn("failed to subscribe Agent session removals", error);
			});

		return () => {
			active = false;
			updatesUnlisten?.();
			removalsUnlisten?.();
		};
	}, []);

	useEffect(() => {
		if (typeof window === "undefined" || typeof document === "undefined") {
			return;
		}

		if (!launcherViewActive) {
			return;
		}

		let active = true;
		let unlistenFocusChanged: (() => void) | null = null;
		const shouldSkipReactiveLauncherInputFocus = () =>
			(desktopRuntimeAvailable && suspendReactiveLauncherInputFocusRef.current) ||
			clipboardPanelVisibleRef.current;

		function handleDocumentVisibilityChange() {
			if (document.visibilityState === "visible" && !shouldSkipReactiveLauncherInputFocus()) {
				scheduleLauncherInputFocusRef.current();
			}
		}

		function handleWindowFocus() {
			if (shouldSkipReactiveLauncherInputFocus()) {
				return;
			}
			scheduleLauncherInputFocusRef.current();
		}

		document.addEventListener("visibilitychange", handleDocumentVisibilityChange);
		window.addEventListener("focus", handleWindowFocus);

		if (desktopRuntimeAvailable) {
			void getCurrentWindow()
				.onFocusChanged(({ payload: focused }) => {
					if (focused && !shouldSkipReactiveLauncherInputFocus()) {
						scheduleLauncherInputFocusRef.current();
					}
				})
				.then((unlisten) => {
					if (!active) {
						unlisten();
						return;
					}

					unlistenFocusChanged = unlisten;
				})
				.catch((error: unknown) => {
					console.warn("failed to subscribe launcher focus changes", error);
				});
		}

		return () => {
			active = false;
			clearScheduledLauncherInputFocus();
			document.removeEventListener("visibilitychange", handleDocumentVisibilityChange);
			window.removeEventListener("focus", handleWindowFocus);
			unlistenFocusChanged?.();
		};
	}, [clearScheduledLauncherInputFocus, desktopRuntimeAvailable, launcherViewActive]);

	useDismissOnPointerDownOutside({
		open: workspacePickerOpen,
		triggerRef: workspacePickerTriggerRef,
		panelRef: workspacePickerPanelRef,
		onDismiss: () => setWorkspacePickerOpen(false),
	});

	useDismissOnPointerDownOutside({
		open: sessionPanelOpen,
		triggerRef: sessionPanelTriggerRef,
		panelRef: sessionPanelRef,
		onDismiss: () => setSessionPanelOpen(false),
	});

	useLayoutEffect(() => {
		const logElement = sessionLogRef.current;
		if (!logElement || !activeSessionScrollAnchor) {
			return;
		}

		logElement.scrollTop = 0;
	}, [activeSessionScrollAnchor]);

	function applySessionDetail(detail: AcpSessionDetail) {
		setSessionDetails((current) => upsertSessionDetailRecord(current, detail));
		setSessionSummaries((current) => upsertSessionSummary(current, detail.session));
	}

	function resetComposer() {
		setRawText("");
		setCaretIndex(0);
		setResult(null);
		setError(null);
		setActiveSlashAction(null);
	}

	const updateClipboardHistorySnapshot = useCallback((snapshot: ClipboardHistorySnapshot) => {
		setClipboardHistory(snapshot);
	}, []);

	const dismissClipboardHistoryPanel = useCallback(() => {
		if (windowKind === "clipboard_history") {
			void dismissClipboardHistoryWindow();
			return;
		}

		setClipboardPanelOpen(false);
		if (clipboardSelectionMode === "insert_into_launcher") {
			scheduleLauncherInputFocus();
			return;
		}

		void dismissLauncher();
	}, [clipboardSelectionMode, dismissLauncher, scheduleLauncherInputFocus, windowKind]);

	const findClipboardEntry = useCallback(
		(entryId: string | null): ClipboardHistoryEntry | null => {
			if (!entryId) {
				return null;
			}

			return flattenedClipboardEntries.find((entry) => entry.id === entryId) ?? null;
		},
		[flattenedClipboardEntries],
	);

	const selectAdjacentClipboardEntry = useCallback(
		(direction: 1 | -1) => {
			if (flattenedClipboardEntries.length === 0) {
				return;
			}

			const currentIndex = flattenedClipboardEntries.findIndex(
				(entry) => entry.id === selectedClipboardEntryId,
			);
			const nextIndex =
				currentIndex === -1
					? 0
					: (currentIndex + direction + flattenedClipboardEntries.length) %
						flattenedClipboardEntries.length;
			setSelectedClipboardEntryId(flattenedClipboardEntries[nextIndex]?.id ?? null);
		},
		[flattenedClipboardEntries, selectedClipboardEntryId],
	);

	const handleClipboardEntryPaste = useCallback(
		async (entryId: string) => {
			const entry = findClipboardEntry(entryId);
			if (!entry) {
				return;
			}

			if (clipboardSelectionMode === "insert_into_launcher") {
				if (windowKind === "clipboard_history") {
					await insertClipboardHistoryTextIntoLauncher(entry.text);
					setError(null);
					return;
				}

				const nextRawText = replaceTextRange(
					rawText,
					boundedCaretIndex,
					boundedCaretIndex,
					entry.text,
				);
				pendingSelectionRef.current = nextRawText.caretIndex;
				updateRawText(nextRawText.value, nextRawText.caretIndex);
				setClipboardPanelOpen(false);
				scheduleLauncherInputFocus();
				setError(null);
				return;
			}

			try {
				const snapshot = await pasteClipboardHistoryEntry(entryId);
				updateClipboardHistorySnapshot(snapshot);
				setClipboardPanelOpen(false);
				setError(null);
			} catch (clipboardError) {
				setError(getErrorMessage(clipboardError, "回贴历史剪贴板失败"));
			}
		},
		[
			boundedCaretIndex,
			clipboardSelectionMode,
			findClipboardEntry,
			windowKind,
			rawText,
			scheduleLauncherInputFocus,
			updateClipboardHistorySnapshot,
			updateRawText,
		],
	);

	const handleSelectedClipboardEntryPaste = useCallback(async () => {
		const selectedEntry = findClipboardEntry(selectedClipboardEntryId);
		if (!selectedEntry) {
			return;
		}

		await handleClipboardEntryPaste(selectedEntry.id);
	}, [findClipboardEntry, handleClipboardEntryPaste, selectedClipboardEntryId]);

	const handleClipboardEntryTogglePin = useCallback(
		async (entryId: string) => {
			try {
				const snapshot = await toggleClipboardHistoryEntryPin(entryId);
				updateClipboardHistorySnapshot(snapshot);
				setSelectedClipboardEntryId(entryId);
				setError(null);
			} catch (clipboardError) {
				setError(getErrorMessage(clipboardError, "切换历史剪贴板固定状态失败"));
			}
		},
		[updateClipboardHistorySnapshot],
	);

	const handleSelectedClipboardEntryTogglePin = useCallback(async () => {
		const selectedEntry = findClipboardEntry(selectedClipboardEntryId);
		if (!selectedEntry) {
			return;
		}

		await handleClipboardEntryTogglePin(selectedEntry.id);
	}, [findClipboardEntry, handleClipboardEntryTogglePin, selectedClipboardEntryId]);

	const handleClipboardEntryDelete = useCallback(
		async (entryId: string) => {
			try {
				const snapshot = await deleteClipboardHistoryEntry(entryId);
				updateClipboardHistorySnapshot(snapshot);
				setError(null);
			} catch (clipboardError) {
				setError(getErrorMessage(clipboardError, "删除历史剪贴板失败"));
			}
		},
		[updateClipboardHistorySnapshot],
	);

	const handleSelectedClipboardEntryDelete = useCallback(async () => {
		const selectedEntry = findClipboardEntry(selectedClipboardEntryId);
		if (!selectedEntry) {
			return;
		}

		await handleClipboardEntryDelete(selectedEntry.id);
	}, [findClipboardEntry, handleClipboardEntryDelete, selectedClipboardEntryId]);

	useEffect(() => {
		if (typeof window === "undefined") {
			return;
		}

		if (!clipboardPanelVisible) {
			return;
		}

		const handleWindowKeyDown = (event: globalThis.KeyboardEvent) => {
			if (event.altKey && !event.metaKey && !event.ctrlKey) {
				const pinnedIndex = getPinnedClipboardHotkeyIndex(event.code);
				if (pinnedIndex !== null) {
					const pinnedEntry = clipboardHistory.pinnedEntries[pinnedIndex];
					if (pinnedEntry) {
						event.preventDefault();
						void handleClipboardEntryPaste(pinnedEntry.id);
					}
					return;
				}

				const recentIndex = getRecentClipboardHotkeyIndex(event.code);
				if (recentIndex !== null) {
					const recentEntry = clipboardHistory.recentEntries[recentIndex];
					if (recentEntry) {
						event.preventDefault();
						void handleClipboardEntryPaste(recentEntry.id);
					}
					return;
				}
			}

			if (
				(event.key === "Delete" || event.key === "Backspace") &&
				!event.metaKey &&
				!event.ctrlKey &&
				!event.altKey
			) {
				event.preventDefault();
				void handleSelectedClipboardEntryDelete();
				return;
			}

			if (event.key.toLowerCase() === "p" && (event.metaKey || event.ctrlKey) && !event.altKey) {
				event.preventDefault();
				void handleSelectedClipboardEntryTogglePin();
				return;
			}

			if (event.key === "ArrowDown") {
				event.preventDefault();
				selectAdjacentClipboardEntry(1);
				return;
			}

			if (event.key === "ArrowUp") {
				event.preventDefault();
				selectAdjacentClipboardEntry(-1);
				return;
			}

			if (event.key === "Escape") {
				event.preventDefault();
				dismissClipboardHistoryPanel();
				return;
			}

			if (event.key === "Enter" && !event.metaKey && !event.ctrlKey && !event.altKey) {
				event.preventDefault();
				void handleSelectedClipboardEntryPaste();
			}
		};

		window.addEventListener("keydown", handleWindowKeyDown, true);
		return () => {
			window.removeEventListener("keydown", handleWindowKeyDown, true);
		};
	}, [
		clipboardHistory.pinnedEntries,
		clipboardHistory.recentEntries,
		clipboardPanelVisible,
		dismissClipboardHistoryPanel,
		handleClipboardEntryPaste,
		handleSelectedClipboardEntryDelete,
		handleSelectedClipboardEntryPaste,
		handleSelectedClipboardEntryTogglePin,
		selectAdjacentClipboardEntry,
	]);

	useDismissOnPointerDownOutside({
		open: clipboardPanelVisible,
		triggerRef: inputAnchorRef,
		panelRef: clipboardPanelRef,
		onDismiss: dismissClipboardHistoryPanel,
	});

	const appendRagConversationTurn = useCallback((question: string, answer: string) => {
		const normalizedQuestion = question.trim();
		const normalizedAnswer = answer.trim();
		if (!normalizedQuestion || !normalizedAnswer) {
			return;
		}

		setRagConversation((current) =>
			[
				...current,
				{ role: "user" as const, content: normalizedQuestion },
				{ role: "assistant" as const, content: normalizedAnswer },
			].slice(-12),
		);
	}, []);

	const applyQaResult = useCallback(
		(prompt: string, executionResult: ExecutionResult) => {
			if (executionResult.status !== "success" || !executionResult.primaryText) {
				return false;
			}
			const payload = isRagAnswerStructuredPayload(executionResult.structuredPayload)
				? executionResult.structuredPayload
				: null;

			setQaAutoResizeFrozen(true);
			void armLauncherBlurAutoHideSuppression(QA_RESULT_BLUR_AUTO_HIDE_SUPPRESSION_MS).catch(
				(error: unknown) => {
					console.warn("failed to arm launcher blur suppression for QA result", error);
				},
			);
			if (desktopRuntimeAvailable && launcherBlurAutoHideEnabledRef.current) {
				suspendReactiveLauncherInputFocusRef.current = true;
				clearScheduledLauncherInputFocus();
				launcherBlurAutoHideEnabledRef.current = false;
				void setLauncherBlurAutoHideEnabled(false).catch((error: unknown) => {
					console.warn("failed to disable launcher blur auto-hide for QA result", error);
				});
			}
			scheduleLauncherBlurAutoHideRestore(QA_RESULT_BLUR_AUTO_HIDE_SUPPRESSION_MS);

			const timestamp = Date.now();
			const userMessage: AcpSessionMessage = {
				id: `${timestamp}-user`,
				role: "user",
				blocks: [
					{
						type: "content",
						text: prompt,
					},
				],
				pending: false,
			};
			const assistantMessage: AcpSessionMessage = {
				id: `${timestamp}-assistant`,
				role: "assistant",
				blocks: buildQaAssistantMessageBlocks(executionResult, payload),
				pending: false,
			};

			setQaMessages((current) =>
				current.length > 0
					? [...current, userMessage, assistantMessage]
					: [userMessage, assistantMessage],
			);
			setQaConversationState(
				payload?.conversationState ?? {
					previousResponseId: payload?.responseId ?? null,
					continuationScope: null,
					citations: payload?.citations ?? [],
					actions: payload?.actions ?? [],
					toolCalls: payload?.tools.calls ?? [],
				},
			);
			setQaRetrieval(payload?.retrieval ?? null);
			setResult(null);
			return true;
		},
		[
			clearScheduledLauncherInputFocus,
			desktopRuntimeAvailable,
			scheduleLauncherBlurAutoHideRestore,
		],
	);

	const activatePendingSlashAction = useCallback(
		(descriptor: ActionDescriptor) => {
			flushSync(() => {
				setActiveSlashAction(descriptor);
				pendingSelectionRef.current = 0;
				updateRawText("", 0);
				resetSuggestions(true);
			});
		},
		[resetSuggestions, updateRawText],
	);

	const executeLauncherAction = useCallback(
		async (
			descriptor: ActionDescriptor,
			query = fullQuery,
			options?: {
				onSuccess?: () => void;
			},
		) => {
			const requestEpoch = beginTrackedLauncherRequest();
			const nextOperationStatus = resolveLauncherOperationStatus(descriptor.id);
			if (nextOperationStatus) {
				setOperationStatusText(nextOperationStatus);
			}
			if (descriptor.id === "translate_text") {
				setResult(null);
			}
			setOperationPending(true);
			try {
				const executionResult = await executeAction({
					actionId: descriptor.id,
					query,
					conversation: descriptor.id === "rag_answer" ? ragConversation : undefined,
					conversationState: descriptor.id === "rag_answer" ? qaConversationState : undefined,
				});

				if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
					return false;
				}

				setLatestSubmittedText(query.rawText.trim() || descriptor.title);
				options?.onSuccess?.();
				const appliedQaResult =
					descriptor.id === "rag_answer" &&
					applyQaResult(extractQaPrompt(query.rawText), executionResult);
				if (!appliedQaResult) {
					setResult(executionResult);
				}
				setError(null);
				if (appliedQaResult && executionResult.primaryText) {
					appendRagConversationTurn(extractQaPrompt(query.rawText), executionResult.primaryText);
				} else if (descriptor.id !== "rag_answer") {
					resetQaConversation();
				}

				// Hide the always-on-top launcher first so opener targets can take focus.
				if (executionResult.shouldCloseLauncher) {
					await dismissLauncher();
				}
				await applyClientEffect(executionResult);

				return true;
			} catch (executionError) {
				if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
					return false;
				}

				if (descriptor.id === "rag_answer") {
					setResult(null);
				}
				setError(getErrorMessage(executionError, "动作执行失败"));
				return false;
			} finally {
				const requestStillCurrent = isTrackedLauncherRequestCurrent(requestEpoch);
				endTrackedLauncherRequest(requestEpoch);
				if (requestStillCurrent && nextOperationStatus) {
					setOperationStatusText(null);
				}
				if (requestStillCurrent) {
					setOperationPending(false);
				}
			}
		},
		[
			applyQaResult,
			appendRagConversationTurn,
			beginTrackedLauncherRequest,
			dismissLauncher,
			endTrackedLauncherRequest,
			fullQuery,
			isTrackedLauncherRequestCurrent,
			qaConversationState,
			ragConversation,
			resetQaConversation,
		],
	);

	async function runPendingSlashAction() {
		if (!pendingSlashAction || rawText.trim().length === 0) {
			return;
		}

		await executeLauncherAction(pendingSlashAction, fullQuery, {
			onSuccess: () => {
				setActiveSlashAction(null);
			},
		});
	}

	async function runFallbackRagAnswer() {
		if (!shouldFallbackToRagAnswer) {
			return;
		}

		await executeLauncherAction(ragAnswerActionDescriptor, fullQuery);
	}

	const runExplicitKillAction = useCallback(async () => {
		const payload = killSearchQuery.trim();
		if (!payload) {
			return;
		}

		await executeLauncherAction(killProcessActionDescriptor, buildQuery(inputMode, payload), {
			onSuccess: () => {
				pendingSelectionRef.current = payload.length;
				updateRawText(payload, payload.length);
			},
		});
	}, [executeLauncherAction, inputMode, killSearchQuery, updateRawText]);

	async function handleTranslateAction() {
		if (!canRunTranslateAction) {
			return;
		}

		await executeLauncherAction(translateActionDescriptor, fullQuery, {
			onSuccess: () => {
				setActiveSlashAction(null);
			},
		});
	}

	function selectFile(index: number) {
		const selected = visibleFileMatches[index];
		if (!selected || !activeFileToken) {
			return;
		}

		const nextRawText = replaceTextRange(
			rawText,
			activeFileToken.start,
			activeFileToken.end,
			selected.path,
		);
		pendingSelectionRef.current = nextRawText.caretIndex;
		updateRawText(nextRawText.value, nextRawText.caretIndex);
		resetSuggestions();
		setSelectedIndex(0);
		setError(null);
		setResult({
			status: "success",
			primaryText: selected.path,
			secondaryText: "已插入文件路径。",
			structuredPayload: null,
			nextActions: [],
			shouldCloseLauncher: false,
		});
	}

	const replaceKillPayload = useCallback(
		(value: string) => {
			if (pendingSlashAction?.id === "kill_process") {
				return replaceTextRange(rawText, 0, rawText.length, value);
			}

			const slashInputMatch = parseSlashActionInput(rawText, killProcessActionDescriptor.aliases);
			if (!slashInputMatch) {
				return replaceTextRange(rawText, 0, boundedCaretIndex, value);
			}

			const leadingWhitespaceLength = rawText.match(/^\s*/u)?.[0].length ?? 0;
			return replaceTextRange(
				rawText,
				leadingWhitespaceLength + slashInputMatch.contentStart,
				rawText.length,
				value,
			);
		},
		[boundedCaretIndex, pendingSlashAction, rawText],
	);

	const selectKill = useCallback(
		(index: number) => {
			const selected = visibleKillMatches[index];
			if (!selected) {
				return;
			}

			const nextValue = deriveKillCompletion(selected);
			if (!nextValue) {
				return;
			}

			const nextRawText = replaceKillPayload(nextValue);
			pendingSelectionRef.current = nextRawText.caretIndex;
			updateRawText(nextRawText.value, nextRawText.caretIndex);
			resetSuggestions();
			setSelectedIndex(0);
			setError(null);
			setResult({
				status: "success",
				primaryText: nextValue,
				secondaryText: `已选择 ${selected.displayName} (pid ${selected.pid})。`,
				structuredPayload: null,
				nextActions: [],
				shouldCloseLauncher: false,
			});
		},
		[replaceKillPayload, resetSuggestions, setSelectedIndex, updateRawText, visibleKillMatches],
	);

	function acceptCompletion() {
		if (!completionText) {
			return;
		}

		const nextRawText =
			suggestionMode === "file" && activeFileToken
				? replaceTextRange(rawText, activeFileToken.start, activeFileToken.end, completionText)
				: suggestionMode === "kill"
					? replaceKillPayload(completionText)
					: replaceTextRange(rawText, 0, boundedCaretIndex, completionText);
		pendingSelectionRef.current = nextRawText.caretIndex;
		updateRawText(nextRawText.value, nextRawText.caretIndex);

		if (suggestionMode === "file") {
			setResult({
				status: "success",
				primaryText: completionText,
				secondaryText: "已补全文件路径。",
				structuredPayload: null,
				nextActions: [],
				shouldCloseLauncher: false,
			});
			return;
		}

		if (suggestionMode === "kill" && selectedKillMatch) {
			setResult({
				status: "success",
				primaryText: completionText,
				secondaryText: `已填入 ${selectedKillMatch.displayName} 的 pid。`,
				structuredPayload: null,
				nextActions: [],
				shouldCloseLauncher: false,
			});
		}
	}

	function acceptSelectedSuggestion(index: number = selectedIndex) {
		if (suggestionMode === "file") {
			selectFile(index);
			return;
		}

		if (suggestionMode === "kill") {
			selectKill(index);
			return;
		}

		if (suggestionMode !== "action") {
			return;
		}

		const selected = visibleActionMatches[index];
		if (!selected) {
			return;
		}

		const insertedText =
			deriveActionCompletion(textBeforeCaret, selected) ?? selected.descriptor.title;
		const slashInputMatch = parseSlashActionInput(rawText, selected.descriptor.aliases);
		const commandTokenRange = slashInputMatch
			? findSlashCommandPrefixRange(rawText, slashInputMatch.commandToken)
			: findSlashCommandTokenRange(rawText);
		const nextRawText = commandTokenRange
			? replaceTextRange(rawText, commandTokenRange.start, commandTokenRange.end, insertedText)
			: replaceTextRange(rawText, 0, boundedCaretIndex, insertedText);
		pendingSelectionRef.current = nextRawText.caretIndex;
		setSelectedIndex(index);
		updateRawText(nextRawText.value, nextRawText.caretIndex);
	}

	const resolveSelectedAction = useCallback(
		(index: number = selectedIndex) => {
			const selected = visibleActionMatches[index];
			if (!selected) {
				return null;
			}

			const slashInputMatch = parseSlashActionInput(rawText, selected.descriptor.aliases);
			if (!slashInputMatch) {
				return {
					selected,
					slashInputMatch: null,
					resolvedRawText: rawText,
				};
			}

			const completedCommand =
				deriveActionCompletion(textBeforeCaret, selected) ??
				slashInputMatch.alias ??
				selected.descriptor.aliases[0] ??
				selected.descriptor.title;
			const commandTokenRange = findSlashCommandPrefixRange(rawText, slashInputMatch.commandToken);
			const nextRawText = commandTokenRange
				? replaceTextRange(
						rawText,
						commandTokenRange.start,
						commandTokenRange.end,
						completedCommand,
					)
				: replaceTextRange(rawText, 0, boundedCaretIndex, completedCommand);

			return {
				selected,
				slashInputMatch: parseSlashActionInput(nextRawText.value, selected.descriptor.aliases),
				resolvedRawText: nextRawText.value,
			};
		},
		[boundedCaretIndex, rawText, selectedIndex, textBeforeCaret, visibleActionMatches],
	);

	const runSelectedAction = useCallback(
		async (index: number = selectedIndex) => {
			const resolvedAction = resolveSelectedAction(index);
			if (!resolvedAction) {
				return;
			}

			const { selected, slashInputMatch, resolvedRawText } = resolvedAction;
			const explicitSlashSelection =
				rawText.trimStart().startsWith("/") &&
				selected.descriptor.aliases.some((alias) => alias.startsWith("/"));

			const payloadState = explicitSlashSelection
				? deriveSelectedSlashActionPayload(rawText, boundedCaretIndex, selected.descriptor.aliases)
				: slashInputMatch
					? deriveSlashActionPayload(rawText, boundedCaretIndex, selected.descriptor.aliases)
					: null;
			const actionRawText = payloadState?.value ?? slashInputMatch?.content ?? rawText;
			if ((explicitSlashSelection || slashInputMatch) && actionRawText.length === 0) {
				activatePendingSlashAction(selected.descriptor);
				return;
			}

			const actionQuery =
				explicitSlashSelection || slashInputMatch
					? buildQuery(inputMode, actionRawText)
					: fullQuery;
			if (explicitSlashSelection || slashInputMatch) {
				setActiveSlashAction(selected.descriptor);
				pendingSelectionRef.current = payloadState?.caretIndex ?? actionRawText.length;
				updateRawText(actionRawText, payloadState?.caretIndex ?? actionRawText.length);
				resetSuggestions(true);
				await executeLauncherAction(selected.descriptor, actionQuery, {
					onSuccess: () => {
						setActiveSlashAction(null);
					},
				});
				return;
			}

			await executeLauncherAction(selected.descriptor, actionQuery, {
				onSuccess: () => {
					if (resolvedRawText !== rawText) {
						pendingSelectionRef.current = resolvedRawText.length;
						updateRawText(resolvedRawText, resolvedRawText.length);
					}
				},
			});
		},
		[
			activatePendingSlashAction,
			boundedCaretIndex,
			executeLauncherAction,
			fullQuery,
			inputMode,
			rawText,
			resetSuggestions,
			resolveSelectedAction,
			selectedIndex,
			updateRawText,
		],
	);

	const runSelectedApp = useCallback(
		async (index: number = selectedIndex) => {
			const selected = visibleAppMatches[index];
			if (!selected) {
				return;
			}

			const requestEpoch = beginTrackedLauncherRequest();
			setOperationPending(true);
			try {
				const executionResult = await launchApp(selected.path);
				if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
					return;
				}
				setLatestSubmittedText(rawText.trim() || selected.name);
				setResult(executionResult);
				setError(null);

				if (executionResult.shouldCloseLauncher) {
					await dismissLauncher();
				}
			} catch (launchError) {
				if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
					return;
				}
				setError(getErrorMessage(launchError, "应用启动失败"));
			} finally {
				const requestStillCurrent = isTrackedLauncherRequestCurrent(requestEpoch);
				endTrackedLauncherRequest(requestEpoch);
				if (requestStillCurrent) {
					setOperationPending(false);
				}
			}
		},
		[
			beginTrackedLauncherRequest,
			dismissLauncher,
			endTrackedLauncherRequest,
			isTrackedLauncherRequestCurrent,
			rawText,
			selectedIndex,
			visibleAppMatches,
		],
	);

	async function runSessionPrompt() {
		if (!activeSessionId) {
			return;
		}

		const prompt = rawText.trim();
		if (!prompt) {
			return;
		}

		const requestEpoch = beginTrackedLauncherRequest();
		setOperationStatusText("Agent 执行中 · 正在等待响应");
		setOperationPending(true);
		setAgentActionPending(true);
		try {
			const detail = await sendAcpPrompt(activeSessionId, prompt);
			if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
				return;
			}
			setLatestSubmittedText(prompt);
			applySessionDetail(detail);
			resetComposer();
		} catch (promptError) {
			if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
				return;
			}
			setError(getErrorMessage(promptError, "Agent prompt 发送失败"));
		} finally {
			const requestStillCurrent = isTrackedLauncherRequestCurrent(requestEpoch);
			endTrackedLauncherRequest(requestEpoch);
			if (requestStillCurrent) {
				setOperationStatusText(null);
				setAgentActionPending(false);
				setOperationPending(false);
			}
		}
	}

	async function handleAgentExecute() {
		const prompt = rawText.trim();
		if (!prompt) {
			return;
		}

		const requestEpoch = beginTrackedLauncherRequest();
		setOperationStatusText("Agent 执行中 · 正在等待响应");
		setOperationPending(true);
		setAgentActionPending(true);
		try {
			let targetSessionId = activeSessionId;

			if (!targetSessionId) {
				const createdDetail = await createAndActivateSession(requestEpoch);
				targetSessionId = createdDetail.session.sessionId;
			}

			if (!targetSessionId) {
				throw new Error("Agent session 创建失败");
			}

			const detail = await sendAcpPrompt(targetSessionId, prompt);
			if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
				return;
			}
			setLatestSubmittedText(prompt);
			applySessionDetail(detail);
			resetComposer();
		} catch (agentError) {
			if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
				return;
			}
			setError(getErrorMessage(agentError, "Agent 执行失败"));
		} finally {
			const requestStillCurrent = isTrackedLauncherRequestCurrent(requestEpoch);
			endTrackedLauncherRequest(requestEpoch);
			if (requestStillCurrent) {
				setOperationStatusText(null);
				setAgentActionPending(false);
				setOperationPending(false);
			}
		}
	}

	async function runPrimaryAction(index: number = selectedIndex) {
		switch (primaryActionState.kind) {
			case "insert_path":
			case "search_path":
				selectFile(index);
				return;
			case "send":
				await runSessionPrompt();
				return;
			case "rag_answer":
				await runFallbackRagAnswer();
				return;
			case "launch_app":
				await runSelectedApp(index);
				return;
			case "run_kill":
				await runExplicitKillAction();
				return;
			case "pending_action":
				await runPendingSlashAction();
				return;
			case "run_action":
				await runSelectedAction(index);
				return;
			case "idle":
				return;
		}
	}

	async function handleOpenRagCitation(citation: RagCitation) {
		try {
			await dismissLauncher();
			await openDocumentReference(citation.absolutePath);
		} catch (openError) {
			if (desktopRuntimeAvailable) {
				void getCurrentWindow()
					.show()
					.catch(() => undefined);
			}
			setError(getErrorMessage(openError, "打开文档引用失败"));
			scheduleLauncherInputFocus();
		}
	}

	const applyScreenshotReviewBlockSelection = useCallback(
		(nextSelectedBlockIds: string[]) => {
			setSelectedScreenshotReviewBlockIds(nextSelectedBlockIds);
			if (!screenshotReviewTextManuallyEditedRef.current) {
				setScreenshotReviewText(
					aggregateScreenshotReviewBlockText(screenshotReview, nextSelectedBlockIds),
				);
			}
			setScreenshotReviewCopyFeedback(null);
		},
		[screenshotReview],
	);

	const handleScreenshotReviewBlockToggle = useCallback(
		(blockId: string) => {
			const nextSelectedBlockIds = selectedScreenshotReviewBlockIds.includes(blockId)
				? selectedScreenshotReviewBlockIds.filter((selectedId) => selectedId !== blockId)
				: [...selectedScreenshotReviewBlockIds, blockId];
			applyScreenshotReviewBlockSelection(nextSelectedBlockIds);
		},
		[applyScreenshotReviewBlockSelection, selectedScreenshotReviewBlockIds],
	);

	const handleScreenshotReviewSelectAll = useCallback(() => {
		applyScreenshotReviewBlockSelection(
			screenshotReview?.ocr.blocks.map((block) => block.id) ?? [],
		);
	}, [applyScreenshotReviewBlockSelection, screenshotReview]);

	const handleScreenshotReviewClearSelection = useCallback(() => {
		applyScreenshotReviewBlockSelection([]);
	}, [applyScreenshotReviewBlockSelection]);

	const handleScreenshotReviewTextChange = useCallback((value: string) => {
		screenshotReviewTextManuallyEditedRef.current = true;
		setScreenshotReviewText(value);
		setScreenshotReviewCopyFeedback(null);
	}, []);

	const handleScreenshotReviewUseSelectedBlocks = useCallback(() => {
		const nextText = aggregateScreenshotReviewBlockText(
			screenshotReview,
			selectedScreenshotReviewBlockIds,
		);
		screenshotReviewTextManuallyEditedRef.current = false;
		setScreenshotReviewText(nextText);
		setScreenshotReviewCopyFeedback(null);
	}, [screenshotReview, selectedScreenshotReviewBlockIds]);

	const clearScreenshotReviewState = useCallback(() => {
		screenshotReviewSessionIdRef.current = null;
		screenshotReviewTextManuallyEditedRef.current = false;
		setScreenshotReview(null);
		setScreenshotReviewPreview(null);
		setScreenshotReviewText("");
		setSelectedScreenshotReviewBlockIds([]);
		setScreenshotReviewBusy(false);
		setScreenshotReviewCopyFeedback(null);
	}, []);

	const handleScreenshotReviewCancel = useCallback(async () => {
		if (!screenshotReview || screenshotReviewBusy) {
			return;
		}

		const sessionId = screenshotReview.sessionId;
		setScreenshotReviewBusy(true);
		try {
			await cancelScreenshotReview(sessionId);
			if (screenshotReviewSessionIdRef.current === sessionId) {
				clearScreenshotReviewState();
				setError(null);
				scheduleLauncherInputFocus();
			}
		} catch (cancelError) {
			if (screenshotReviewSessionIdRef.current === sessionId) {
				setScreenshotReviewBusy(false);
				setError(getErrorMessage(cancelError, "取消截图 Review 失败"));
			}
		}
	}, [
		clearScreenshotReviewState,
		scheduleLauncherInputFocus,
		screenshotReview,
		screenshotReviewBusy,
	]);

	const handleScreenshotReviewRetry = useCallback(async () => {
		if (!screenshotReview) {
			return;
		}

		const sessionId = screenshotReview.sessionId;
		setScreenshotReviewBusy(true);
		setScreenshotReviewCopyFeedback(null);
		setError(null);
		try {
			await retryScreenshotReview(sessionId);
			if (screenshotReviewSessionIdRef.current === sessionId) {
				clearScreenshotReviewState();
				scheduleLauncherInputFocus();
			}
		} catch (retryError) {
			if (screenshotReviewSessionIdRef.current === sessionId) {
				clearScreenshotReviewState();
				setError(getErrorMessage(retryError, "重新截图失败"));
				scheduleLauncherInputFocus();
			}
		}
	}, [clearScreenshotReviewState, scheduleLauncherInputFocus, screenshotReview]);

	const handleScreenshotReviewCopy = useCallback(async () => {
		if (!screenshotReview) {
			return;
		}

		const sessionId = screenshotReview.sessionId;
		setScreenshotReviewBusy(true);
		try {
			await confirmScreenshotReview(sessionId, "copy_selected_text", screenshotReviewText);
			if (screenshotReviewSessionIdRef.current === sessionId) {
				setScreenshotReviewCopyFeedback("已复制");
				setError(null);
			}
		} catch (copyError) {
			if (screenshotReviewSessionIdRef.current === sessionId) {
				setError(getErrorMessage(copyError, "复制截图文字失败"));
			}
		} finally {
			if (screenshotReviewSessionIdRef.current === sessionId) {
				setScreenshotReviewBusy(false);
			}
		}
	}, [screenshotReview, screenshotReviewText]);

	const handleScreenshotReviewTranslate = useCallback(async () => {
		if (!screenshotReview) {
			return;
		}

		const sessionId = screenshotReview.sessionId;
		setScreenshotReviewBusy(true);
		try {
			await confirmScreenshotReview(sessionId, "translate_selected_text", screenshotReviewText);
			if (screenshotReviewSessionIdRef.current === sessionId) {
				clearScreenshotReviewState();
				setError(null);
			}
		} catch (translateError) {
			if (screenshotReviewSessionIdRef.current === sessionId) {
				setScreenshotReviewBusy(false);
			} else {
				clearScreenshotReviewState();
				setShortcutTranslationPending(false);
				setOperationStatusText(null);
				scheduleLauncherInputFocus();
			}
			setError(getErrorMessage(translateError, "确认翻译失败"));
		}
	}, [
		clearScreenshotReviewState,
		scheduleLauncherInputFocus,
		screenshotReview,
		screenshotReviewText,
	]);

	useEffect(() => {
		if (typeof document === "undefined" || !screenshotReviewOpen) {
			return;
		}

		function handleScreenshotReviewDocumentKeyDown(event: globalThis.KeyboardEvent) {
			if (event.key !== "Escape") {
				return;
			}

			event.preventDefault();
			void handleScreenshotReviewCancel();
		}

		document.addEventListener("keydown", handleScreenshotReviewDocumentKeyDown);
		return () => {
			document.removeEventListener("keydown", handleScreenshotReviewDocumentKeyDown);
		};
	}, [handleScreenshotReviewCancel, screenshotReviewOpen]);

	async function handleKeyDown(
		event: KeyboardEvent<HTMLInputElement> | KeyboardEvent<HTMLTextAreaElement>,
	) {
		if (screenshotReviewOpen) {
			if (event.key === "Escape") {
				event.preventDefault();
				await handleScreenshotReviewCancel();
			}
			return;
		}

		if (event.key === "Backspace" && pendingSlashAction && rawText.length === 0) {
			setActiveSlashAction(null);
			setResult(null);
			setError(null);
			return;
		}

		if (event.key === "Tab") {
			if (hasCompletion) {
				event.preventDefault();
				acceptCompletion();
			}
			return;
		}

		if (event.key === "ArrowDown") {
			event.preventDefault();
			setSelectedIndex((current) => (visibleCount === 0 ? 0 : (current + 1) % visibleCount));
			return;
		}

		if (event.key === "ArrowUp") {
			event.preventDefault();
			setSelectedIndex((current) =>
				visibleCount === 0 ? 0 : (current - 1 + visibleCount) % visibleCount,
			);
			return;
		}

		if (event.key === "Escape") {
			event.preventDefault();
			if (workspacePickerOpen) {
				setWorkspacePickerOpen(false);
				return;
			}

			if (sessionPanelOpen) {
				setSessionPanelOpen(false);
				return;
			}

			if (hasSuggestions) {
				setSuggestionsHidden(true);
				return;
			}

			if (pendingSlashAction && rawText.length === 0) {
				setActiveSlashAction(null);
				return;
			}

			resetLauncherStateForExplicitDismiss();
			await dismissLauncher();
			return;
		}

		if (event.key !== "Enter") {
			return;
		}

		if (event.altKey && canTriggerAgentAction) {
			event.preventDefault();
			await handleAgentExecute();
			return;
		}

		if ((event.metaKey || event.ctrlKey) && usesInlineInputControl(inputMode)) {
			event.preventDefault();
			setInputMode("multiline");
			const newText = rawText.slice(0, boundedCaretIndex) + "\n" + rawText.slice(boundedCaretIndex);
			pendingSelectionRef.current = boundedCaretIndex + 1;
			updateRawText(newText, boundedCaretIndex + 1);
			return;
		}

		if ((event.metaKey || event.ctrlKey) && usesTextareaInputControl(inputMode)) {
			return;
		}

		if (usesInlineInputControl(inputMode) && !event.shiftKey) {
			event.preventDefault();
			if (hasSuggestions && suggestionMode === "kill") {
				acceptSelectedSuggestion();
				return;
			}
			await runPrimaryAction();
			return;
		}

		if (usesTextareaInputControl(inputMode)) {
			event.preventDefault();
			if (hasSuggestions) {
				if (suggestionMode === "action") {
					await runSelectedAction();
					return;
				}

				if (suggestionMode === "file") {
					acceptSelectedSuggestion();
					return;
				}

				if (suggestionMode === "kill") {
					acceptSelectedSuggestion();
					return;
				}
			}

			await runPrimaryAction();
			return;
		}
	}

	function syncCaretIndex(element: HTMLInputElement | HTMLTextAreaElement) {
		setCaretIndex(element.selectionEnd ?? 0);
	}

	const applyWorkspaceSelection = useCallback(
		async (path: string, fallbackMessage: string) => {
			try {
				const nextWorkspace = await setWorkspace(path);
				setWorkspaceState(nextWorkspace);
				resetQaConversation();
				setError(null);
			} catch (workspaceError) {
				setError(getErrorMessage(workspaceError, fallbackMessage));
			}
		},
		[resetQaConversation],
	);

	async function handleWorkspacePick() {
		setWorkspacePickerOpen(false);
		try {
			const selectedPath = await chooseWorkspaceDirectory(workspace.rootPath || undefined);
			if (!selectedPath) {
				return;
			}

			await applyWorkspaceSelection(selectedPath, "选择工作目录失败");
		} catch (workspaceError) {
			setError(getErrorMessage(workspaceError, "选择工作目录失败"));
		}
	}

	const handleWorkspaceCrumbClick = useCallback(
		async (path: string) => {
			setWorkspacePickerOpen(false);
			await applyWorkspaceSelection(path, "切换工作目录失败");
		},
		[applyWorkspaceSelection],
	);

	async function handleRecentWorkspaceClick(path: string) {
		setWorkspacePickerOpen(false);
		await applyWorkspaceSelection(path, "切换最近目录失败");
	}

	const createAndActivateSession = useCallback(
		async (requestEpoch: number = launcherResetEpochRef.current) => {
			const detail = await createAcpSession(null);
			if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
				throw new Error("launcher reset while creating Agent session");
			}
			setSessionDetails((current) => upsertSessionDetailRecord(current, detail));
			const summaries = await activateAcpSession(detail.session.sessionId);
			if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
				throw new Error("launcher reset while activating Agent session");
			}
			setSessionSummaries(summaries);
			setActiveSessionId(detail.session.sessionId);
			return detail;
		},
		[isTrackedLauncherRequestCurrent],
	);

	const handleCreateSession = useCallback(async () => {
		const requestEpoch = beginTrackedLauncherRequest();
		setCreatingSession(true);
		try {
			await createAndActivateSession(requestEpoch);
			if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
				return;
			}
			setSessionPanelOpen(true);
			setResult(null);
			setError(null);
		} catch (sessionError) {
			if (!isTrackedLauncherRequestCurrent(requestEpoch)) {
				return;
			}
			setError(getErrorMessage(sessionError, "创建 Agent session 失败"));
		} finally {
			const requestStillCurrent = isTrackedLauncherRequestCurrent(requestEpoch);
			endTrackedLauncherRequest(requestEpoch);
			if (requestStillCurrent) {
				setCreatingSession(false);
			}
		}
	}, [
		beginTrackedLauncherRequest,
		createAndActivateSession,
		endTrackedLauncherRequest,
		isTrackedLauncherRequestCurrent,
	]);

	const handleSessionDotClick = useCallback(
		async (sessionId: string) => {
			const nextSessionId = activeSessionId === sessionId ? null : sessionId;
			try {
				const summaries = await activateAcpSession(nextSessionId);
				setSessionSummaries(summaries);
				setActiveSessionId(nextSessionId);
				if (nextSessionId && !sessionDetails[nextSessionId]) {
					const detail = await getAcpSessionDetail(nextSessionId);
					if (detail) {
						setSessionDetails((current) => upsertSessionDetailRecord(current, detail));
					}
				}
			} catch (sessionError) {
				setError(getErrorMessage(sessionError, "切换 session 失败"));
			}
		},
		[activeSessionId, sessionDetails],
	);

	const handleSessionPanelSelect = useCallback(
		async (sessionId: string) => {
			await handleSessionDotClick(sessionId);
			setSessionPanelOpen(false);
		},
		[handleSessionDotClick],
	);

	const closeSessionById = useCallback(
		async (sessionId: string, collapsePanelWhenLast: boolean) => {
			try {
				await closeAcpSession(sessionId);
				if (activeSessionId === sessionId) {
					setActiveSessionId(null);
				}
				if (collapsePanelWhenLast && sessionSummaries.length <= 1) {
					setSessionPanelOpen(false);
				}
			} catch (sessionError) {
				setError(getErrorMessage(sessionError, "关闭 session 失败"));
			}
		},
		[activeSessionId, sessionSummaries.length],
	);

	const handleCloseSession = useCallback(
		async (sessionId: string) => {
			await closeSessionById(sessionId, true);
		},
		[closeSessionById],
	);

	async function handleCancelActiveSession() {
		if (!activeSessionId) {
			return;
		}

		try {
			await cancelAcpSession(activeSessionId);
			setError(null);
		} catch (sessionError) {
			setError(getErrorMessage(sessionError, "取消 session 失败"));
		}
	}

	async function handleSetActiveSessionMode(modeId: string) {
		if (!activeSessionId) {
			return;
		}

		setRuntimeControlPendingKey("mode");
		try {
			const detail = await setAcpSessionMode(activeSessionId, modeId);
			applySessionDetail(detail);
			setError(null);
		} catch (sessionError) {
			setError(getErrorMessage(sessionError, "切换 Agent session 模式失败"));
		} finally {
			setRuntimeControlPendingKey(null);
		}
	}

	async function handleSetActiveSessionConfigOption(configId: string, valueId: string) {
		if (!activeSessionId) {
			return;
		}

		setRuntimeControlPendingKey(`config:${configId}`);
		try {
			const detail = await setAcpSessionConfigOption(activeSessionId, configId, valueId);
			applySessionDetail(detail);
			setError(null);
		} catch (sessionError) {
			setError(getErrorMessage(sessionError, "更新 Agent session 配置失败"));
		} finally {
			setRuntimeControlPendingKey(null);
		}
	}

	function handleWorkspaceDragStart(event: ReactMouseEvent<HTMLDivElement>) {
		if (event.button !== 0) {
			return;
		}

		event.preventDefault();

		if (!desktopRuntimeAvailable) {
			return;
		}

		void getCurrentWindow()
			.startDragging()
			.catch((dragError: unknown) => {
				console.warn("failed to start launcher window dragging", dragError);
			});
	}

	const handleToggleWorkspacePicker = useCallback(() => {
		setWorkspacePickerOpen((current) => !current);
		setClipboardPanelOpen(false);
	}, []);

	const handleSelectWorkspaceCrumb = useCallback(
		(path: string) => {
			void handleWorkspaceCrumbClick(path);
		},
		[handleWorkspaceCrumbClick],
	);

	const handleCreateSessionClick = useCallback(() => {
		void handleCreateSession();
	}, [handleCreateSession]);

	const handleToggleSessionPanel = useCallback(() => {
		setSessionPanelOpen((current) => !current);
		setClipboardPanelOpen(false);
	}, []);

	const handleSelectSessionDot = useCallback(
		(sessionId: string) => {
			void handleSessionDotClick(sessionId);
		},
		[handleSessionDotClick],
	);

	const handleOpenSessionPanel = useCallback(() => {
		setSessionPanelOpen(true);
		setClipboardPanelOpen(false);
	}, []);

	const handleRunSelectedActionFromPopup = useCallback(
		(index: number) => {
			void runSelectedAction(index);
		},
		[runSelectedAction],
	);

	const handleRunSelectedAppFromPopup = useCallback(
		(index: number) => {
			void runSelectedApp(index);
		},
		[runSelectedApp],
	);

	const handleSelectKillFromPopup = useCallback(
		(index: number) => {
			selectKill(index);
		},
		[selectKill],
	);

	const handleSelectSessionFromPanel = useCallback(
		(sessionId: string) => {
			void handleSessionPanelSelect(sessionId);
		},
		[handleSessionPanelSelect],
	);

	const handleCloseSessionFromPanel = useCallback(
		(sessionId: string) => {
			void handleCloseSession(sessionId);
		},
		[handleCloseSession],
	);

	const handlePasteClipboardEntryFromPanel = useCallback(
		(entryId: string) => {
			void handleClipboardEntryPaste(entryId);
		},
		[handleClipboardEntryPaste],
	);

	const handleToggleClipboardPinFromPanel = useCallback(
		(entryId: string) => {
			void handleClipboardEntryTogglePin(entryId);
		},
		[handleClipboardEntryTogglePin],
	);

	const handleDeleteClipboardEntryFromPanel = useCallback(
		(entryId: string) => {
			void handleClipboardEntryDelete(entryId);
		},
		[handleClipboardEntryDelete],
	);

	const inputPlaceholder = activeSessionId
		? "向当前 Agent session 发送消息，或用 @ 插入当前 workspace 文件路径"
		: pendingSlashAction
			? `已选择 ${pendingSlashAction.aliases[0] ?? pendingSlashAction.title}，输入待处理文本后按 Enter`
			: "输入应用名，或用 / 执行动作、@ 搜索当前 workspace 文件";
	const launcherShortcutError = shortcutRuntimeStatus.toggle_launcher.registered
		? null
		: (shortcutRuntimeStatus.toggle_launcher.message ??
			`启动器快捷键 ${shortcutRuntimeStatus.toggle_launcher.configuredShortcut} 当前不可用。`);

	return (
		<main
			className={["launcher-shell", clipboardPanelVisible ? "launcher-shell-clipboard-only" : null]
				.filter(Boolean)
				.join(" ")}
			ref={shellRef}
		>
			{!clipboardPanelVisible ? (
				<section className="launcher-frame" ref={frameRef} style={{ width: `${frameWidth}px` }}>
					<LauncherHeader
						workspacePickerOpen={workspacePickerOpen}
						workspacePickerTriggerRef={workspacePickerTriggerRef}
						workspaceBreadcrumbs={workspaceBreadcrumbs}
						onToggleWorkspacePicker={handleToggleWorkspacePicker}
						onSelectWorkspaceCrumb={handleSelectWorkspaceCrumb}
						onWorkspaceDragStart={handleWorkspaceDragStart}
						agentConfigured={agentConfigured}
						creatingSession={creatingSession}
						onCreateSession={handleCreateSessionClick}
						sessionPanelOpen={sessionPanelOpen}
						sessionPanelTriggerRef={sessionPanelTriggerRef}
						sessionTriggerSummary={sessionTriggerSummary}
						sessionCount={sessionSummaries.length}
						visibleSessionDots={visibleSessionDots}
						activeSessionId={activeSessionId}
						overflowSessionCount={overflowSessionCount}
						onToggleSessionPanel={handleToggleSessionPanel}
						onSelectSessionDot={handleSelectSessionDot}
						onOpenSessionPanel={handleOpenSessionPanel}
					/>

					<RestoreNoticeList restoreNotices={restoreNotices} />
					{launcherShortcutError ? (
						<div className="launcher-inline-banner launcher-inline-banner-error">
							<span>{launcherShortcutError}</span>
							{onOpenSettings ? (
								<button
									className="launcher-inline-banner-action"
									onClick={onOpenSettings}
									type="button"
								>
									打开设置
								</button>
							) : null}
						</div>
					) : null}

					{screenshotReview ? (
						<ScreenshotReviewPanel
							review={screenshotReview}
							previewDataUrl={screenshotReviewPreview}
							editedText={screenshotReviewText}
							selectedBlockIds={selectedScreenshotReviewBlockIds}
							busy={screenshotReviewBusy}
							copyFeedback={screenshotReviewCopyFeedback}
							onEditedTextChange={handleScreenshotReviewTextChange}
							onToggleBlock={handleScreenshotReviewBlockToggle}
							onSelectAllBlocks={handleScreenshotReviewSelectAll}
							onClearBlockSelection={handleScreenshotReviewClearSelection}
							onUseSelectedBlocks={handleScreenshotReviewUseSelectedBlocks}
							onTranslate={() => void handleScreenshotReviewTranslate()}
							onCopy={() => void handleScreenshotReviewCopy()}
							onRetry={() => void handleScreenshotReviewRetry()}
							onCancel={() => void handleScreenshotReviewCancel()}
						/>
					) : (
						<LauncherComposer
							inputAnchorRef={inputAnchorRef}
							inputMode={inputMode}
							inputRef={inputRef}
							rawText={rawText}
							inputPlaceholder={inputPlaceholder}
							inputLabel="输入动作、问题或文件路径"
							inputDescriptionId="launcher-status-region"
							announceStatus={statusBarState.announce}
							statusLabel={statusBarState.label}
							statusItems={statusBarState.items}
							statusTone={statusBarState.tone}
							onUpdateRawText={updateRawText}
							onSyncCaretIndex={syncCaretIndex}
							onKeyDown={handleKeyDown}
							onOpenSettings={onOpenSettings}
							hasCompletion={hasCompletion}
							hasSuggestions={hasSuggestions}
							completionPopupId={launcherCompletionPopupId}
							activeCompletionOptionId={activeCompletionOptionId}
							onAcceptCompletion={acceptCompletion}
							agentActionPending={agentActionPending}
							showAgentAction={showAgentActionButton}
							agentActionLabel={agentActionLabel}
							agentActionTitle={agentActionTitle}
							canRunAgentAction={canRunAgentAction}
							showAgentActionShortcut={showAgentActionShortcut}
							showTranslateAction={showTranslateAction}
							primaryActionShortcutLabel={primaryActionShortcutLabel}
							primaryActionLabel={primaryActionState.label}
							primaryActionTone={primaryActionState.tone}
							canRunPrimaryAction={canRunPrimaryAction}
							canRunTranslateAction={canRunTranslateAction}
							agentActionShortcutLabel={agentActionShortcutLabel}
							onAgentExecute={() => {
								void handleAgentExecute();
							}}
							onRunTranslateAction={() => void handleTranslateAction()}
							showCancelActiveSession={showCancelActiveSession}
							onCancelActiveSession={() => void handleCancelActiveSession()}
							onRunPrimaryAction={() => void runPrimaryAction()}
						/>
					)}

					{!screenshotReview ? (
						<LauncherFeedback
							activeSession={activeSession}
							launcherPinned={launcherPinned}
							runtimeControlPendingKey={runtimeControlPendingKey}
							qaCitations={qaConversationState?.citations ?? []}
							qaRetrieval={qaRetrieval}
							qaMessages={qaMessages}
							sessionLogRef={sessionLogRef}
							result={result}
							resultPending={operationPending || shortcutTranslationPending}
							jsonPreview={jsonPreview}
							markdownPreview={markdownPreview}
							onSetAcpSessionConfigOption={(configId, valueId) =>
								void handleSetActiveSessionConfigOption(configId, valueId)
							}
							onSetAcpSessionMode={(modeId) => void handleSetActiveSessionMode(modeId)}
							onLauncherPinnedChange={onLauncherPinnedChange}
							onOpenRagCitation={(citation) => void handleOpenRagCitation(citation)}
						/>
					) : null}
				</section>
			) : null}

			{!clipboardPanelVisible && !screenshotReview ? (
				<>
					<LauncherSuggestionsSection
						hasSuggestions={hasSuggestions}
						suggestionMode={suggestionMode}
						popupId={launcherCompletionPopupId}
						optionIdPrefix={launcherCompletionOptionIdPrefix}
						completionOffset={completionOffset}
						completionListRef={completionListRef}
						selectedIndex={selectedIndex}
						visibleFileMatches={visibleFileMatches}
						visibleActionMatches={visibleActionMatches}
						visibleAppMatches={visibleAppMatches}
						visibleKillMatches={visibleKillMatches}
						onSelectIndex={setSelectedIndex}
						onSelectFile={selectFile}
						onSelectKill={handleSelectKillFromPopup}
						onRunSelectedAction={handleRunSelectedActionFromPopup}
						onRunSelectedApp={handleRunSelectedAppFromPopup}
					/>

					<LauncherSessionSection
						open={sessionPanelOpen}
						offset={sessionPanelOffset}
						panelRef={sessionPanelRef}
						agentConfigured={agentConfigured}
						sessionSummaries={sessionSummaries}
						sessionRunningCount={sessionRunningCount}
						sessionAttentionCount={sessionAttentionCount}
						activeSessionId={activeSessionId}
						workspace={workspace}
						onSelectSession={handleSelectSessionFromPanel}
						onCloseSession={handleCloseSessionFromPanel}
					/>
				</>
			) : null}

			<LauncherClipboardSection
				open={clipboardPanelVisible}
				panelRef={clipboardPanelRef}
				selectionMode={clipboardSelectionMode}
				pinnedEntries={clipboardHistory.pinnedEntries}
				recentEntries={clipboardHistory.recentEntries}
				selectedEntryId={selectedClipboardEntryId}
				onSelectEntry={setSelectedClipboardEntryId}
				onPasteEntry={handlePasteClipboardEntryFromPanel}
				onTogglePin={handleToggleClipboardPinFromPanel}
				onDeleteEntry={handleDeleteClipboardEntryFromPanel}
			/>

			{!clipboardPanelVisible && !screenshotReview ? (
				<>
					<WorkspacePickerPanel
						open={workspacePickerOpen}
						offset={workspacePickerOffset}
						panelRef={workspacePickerPanelRef}
						recentWorkspaceRoots={recentWorkspaceRoots}
						workspace={workspace}
						onPickWorkspace={() => void handleWorkspacePick()}
						onSelectRecentWorkspace={(path) => void handleRecentWorkspaceClick(path)}
					/>
				</>
			) : null}
		</main>
	);
}
