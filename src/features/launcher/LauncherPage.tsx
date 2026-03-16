import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent, MouseEvent as ReactMouseEvent } from "react";
import { flushSync } from "react-dom";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
	activateAcpSession,
	cancelAcpSession,
	chooseWorkspaceDirectory,
	closeAcpSession,
	createAcpSession,
	executeAction,
	getAcpAgents,
	getAcpSessionDetail,
	getWorkspace,
	hideLauncherWindow,
	isDesktopRuntimeAvailable,
	launchApp,
	listAcpSessions,
	matchActions,
	onOcrError,
	subscribeAcpSessionRemovals,
	subscribeAcpSessionUpdates,
	onSelectedText,
	onWorkspaceUpdated,
	searchApps,
	searchFiles,
	sendAcpPrompt,
	setWorkspace,
	takeAcpRestoreNotices,
} from "../../lib/tauri/client";
import { useAutoResizeWindow } from "../../lib/tauri/useAutoResizeWindow";
import type {
	AcpAgentCatalog,
	AcpAgentConfig,
	AcpRestoreNotice,
	AcpSessionDetail,
	AcpSessionSummary,
	WorkspaceState,
} from "../../lib/tauri/types";
import type {
	ActionMatch,
	ActionDescriptor,
	ExecutionResult,
	FileSearchMatch,
	FloatingPanelOffset,
	InputMode,
	InstalledAppMatch,
} from "./types";
import {
	appSearchDebounceMs,
	buildQuery,
	deriveActionCompletion,
	deriveJsonPrettyPreview,
	deriveSelectedSlashActionPayload,
	deriveSlashActionPayload,
	deriveFileCompletion,
	findSlashCommandPrefixRange,
	findSlashCommandTokenRange,
	fileSearchDebounceMs,
	findActiveFileToken,
	getErrorMessage,
	isActionQuery,
	isAppSearchReady,
	isFileSearchReady,
	parseSlashActionInput,
	type SuggestionMode,
	replaceTextRange,
} from "./query";
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
import { LauncherHeader } from "./components/LauncherHeader";
import { LauncherComposer } from "./components/LauncherComposer";
import { LauncherFeedback } from "./components/LauncherFeedback";
import { RestoreNoticeList } from "./components/RestoreNoticeList";
import { CompletionPopup } from "./components/CompletionPopup";
import { SessionPanel } from "./components/SessionPanel";
import { WorkspacePickerPanel } from "./components/WorkspacePickerPanel";
import { AgentPickerPanel } from "./components/AgentPickerPanel";
import "./launcher.css";

const defaultInputMode: InputMode = "inline";
async function applyClientEffect(result: ExecutionResult) {
	if (!result.structuredPayload) {
		return;
	}

	const effect = result.structuredPayload.effect;
	if (effect === "copy_to_clipboard" && typeof result.primaryText === "string") {
		await navigator.clipboard.writeText(result.primaryText);
		return;
	}

	if (effect === "open_url") {
		const url = result.structuredPayload.url;
		if (typeof url === "string") {
			await openUrl(url);
		}
	}
}

function getLatestUserPrompt(activeSession: AcpSessionDetail | null) {
	if (!activeSession) {
		return null;
	}

	const latestUserMessage = [...activeSession.messages]
		.reverse()
		.find((message) => message.role === "user");

	if (!latestUserMessage) {
		return null;
	}

	const prompt = latestUserMessage.blocks
		.filter((block): block is { type: "content"; text: string } => block.type === "content")
		.map((block) => block.text)
		.join("")
		.trim();

	return prompt || null;
}

interface LauncherPageProps {
	onOpenSettings?: () => void;
}

export function LauncherPage({ onOpenSettings }: LauncherPageProps) {
	const [inputMode, setInputMode] = useState<InputMode>(defaultInputMode);
	const [workspace, setWorkspaceState] = useState<WorkspaceState>({
		rootPath: "",
		recentRoots: [],
		homePath: null,
		displayHomeAsTilde: false,
	});
	const [rawText, setRawText] = useState("");
	const [actionMatches, setActionMatches] = useState<ActionMatch[]>([]);
	const [fileMatches, setFileMatches] = useState<FileSearchMatch[]>([]);
	const [appMatches, setAppMatches] = useState<InstalledAppMatch[]>([]);
	const [sessionSummaries, setSessionSummaries] = useState<AcpSessionSummary[]>([]);
	const [sessionDetails, setSessionDetails] = useState<Record<string, AcpSessionDetail>>({});
	const [agentCatalog, setAgentCatalog] = useState<AcpAgentCatalog>({
		agents: [],
		defaultAgentId: null,
	});
	const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
	const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
	const [activeSlashAction, setActiveSlashAction] = useState<ActionDescriptor | null>(null);
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [result, setResult] = useState<ExecutionResult | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);
	const [frameWidth, setFrameWidth] = useState(launcherFrameMaxWidth);
	const [caretIndex, setCaretIndex] = useState(0);
	const [suggestionsHidden, setSuggestionsHidden] = useState(false);
	const [completionOffset, setCompletionOffset] = useState<FloatingPanelOffset>({
		x: 0,
		y: 0,
		width: completionPanelMaxWidth,
	});
	const [workspacePickerOffset, setWorkspacePickerOffset] = useState<FloatingPanelOffset>({
		x: 0,
		y: 0,
		width: 280,
	});
	const [agentPickerOffset, setAgentPickerOffset] = useState<FloatingPanelOffset>({
		x: 0,
		y: 0,
		width: 220,
	});
	const [sessionPanelOffset, setSessionPanelOffset] = useState<FloatingPanelOffset>({
		x: 0,
		y: 0,
		width: 360,
	});
	const [creatingSession, setCreatingSession] = useState(false);
	const [sessionPanelOpen, setSessionPanelOpen] = useState(false);
	const [workspacePickerOpen, setWorkspacePickerOpen] = useState(false);
	const [agentPickerOpen, setAgentPickerOpen] = useState(false);
	const [restoreNotices, setRestoreNotices] = useState<AcpRestoreNotice[]>([]);
	const [latestSubmittedText, setLatestSubmittedText] = useState<string | null>(null);
	const frameRef = useRef<HTMLElement | null>(null);
	const shellRef = useRef<HTMLElement | null>(null);
	const sessionPanelRef = useRef<HTMLElement | null>(null);
	const sessionPanelTriggerRef = useRef<HTMLButtonElement | null>(null);
	const workspacePickerTriggerRef = useRef<HTMLButtonElement | null>(null);
	const workspacePickerPanelRef = useRef<HTMLDivElement | null>(null);
	const agentPickerTriggerRef = useRef<HTMLButtonElement | null>(null);
	const agentPickerPanelRef = useRef<HTMLDivElement | null>(null);
	const inputRef = useRef<HTMLInputElement | HTMLTextAreaElement | null>(null);
	const pendingFocusAnimationFrameRef = useRef<number | null>(null);
	const pendingFocusTimeoutRef = useRef<number | null>(null);
	const completionListRef = useRef<HTMLUListElement | null>(null);
	const sessionLogRef = useRef<HTMLDivElement | null>(null);
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
	const fileMode = activeFileToken !== null;
	const launcherMode = activeSessionId === null;
	const textStartsWithSlash = textBeforeCaret.trimStart().startsWith("/");
	const pendingSlashAction =
		launcherMode && activeSlashAction && !textStartsWithSlash ? activeSlashAction : null;
	const markdownPreview = useMemo(() => {
		if (activeSessionId || pendingSlashAction?.id !== "markdown_render") {
			return null;
		}

		return rawText.length > 0 ? rawText : null;
	}, [activeSessionId, pendingSlashAction, rawText]);
	const currentFileNeedle = activeFileToken?.needle ?? "";
	const suggestionMode: SuggestionMode = fileMode
		? "file"
		: !launcherMode
			? "none"
			: pendingSlashAction
				? "none"
				: isActionQuery(textBeforeCaret)
					? "action"
					: isAppSearchReady(textBeforeCaret)
						? "app"
						: "none";
	const visibleActionMatches = useMemo(
		() => (launcherMode ? actionMatches : []),
		[actionMatches, launcherMode],
	);
	const visibleFileMatches = fileMatches;
	const visibleAppMatches = useMemo(
		() => (suggestionMode === "app" ? appMatches : []),
		[appMatches, suggestionMode],
	);
	const suggestionCount =
		suggestionMode === "file"
			? visibleFileMatches.length
			: suggestionMode === "action"
				? visibleActionMatches.length
				: suggestionMode === "app"
					? visibleAppMatches.length
					: 0;
	const canTextMatchCompletions =
		suggestionMode === "file" ? isFileSearchReady(currentFileNeedle) : suggestionMode !== "none";
	const hasSuggestions = !suggestionsHidden && suggestionCount > 0 && canTextMatchCompletions;
	const visibleCount = hasSuggestions ? suggestionCount : 0;
	const selectedActionMatch = visibleActionMatches[selectedIndex];
	const selectedFileMatch = visibleFileMatches[selectedIndex];
	const selectedAppMatch = visibleAppMatches[selectedIndex];
	const completionText =
		suggestionMode === "file"
			? deriveFileCompletion(textBeforeCaret, selectedFileMatch)
			: suggestionMode === "action"
				? deriveActionCompletion(textBeforeCaret, selectedActionMatch)
				: null;
	const hasCompletion = completionText !== null && completionText !== textBeforeCaret;
	const activeSession = activeSessionId ? (sessionDetails[activeSessionId] ?? null) : null;
	const latestSessionPrompt = getLatestUserPrompt(activeSession);
	const activeSessionStatus = activeSession?.session.status ?? null;
	const activeSessionScrollAnchor = activeSession?.session.sessionId ?? null;
	const activeSessionSummary =
		sessionSummaries.find((session) => session.sessionId === activeSessionId) ??
		activeSession?.session ??
		null;
	const selectedAgent =
		agentCatalog.agents.find((agent) => agent.id === selectedAgentId) ??
		agentCatalog.agents.find((agent) => agent.id === agentCatalog.defaultAgentId) ??
		agentCatalog.agents[0] ??
		null;
	const workspaceBreadcrumbs = useMemo(() => buildWorkspaceBreadcrumbs(workspace), [workspace]);
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
	const sessionCanSend = activeSession ? activeSession.session.status === "idle" : false;
	const agentConfigured = agentCatalog.agents.length > 0;
	const primaryActionShortcutLabel = inputMode === "multiline" ? "Ctrl/Cmd+Enter" : "Enter";
	const agentActionShortcutLabel = "Alt+Enter";
	const primaryActionLabel = fileMode
		? "插入路径"
		: activeSessionId
			? "发送"
			: pendingSlashAction
				? `执行 ${pendingSlashAction.aliases[0] ?? pendingSlashAction.title}`
				: "执行";
	const statusBarText = latestSessionPrompt ?? latestSubmittedText;
	const showAgentAction = agentConfigured && !activeSessionId;
	const showCancelActiveSession = activeSessionStatus === "running";
	const canTriggerAgentAction =
		agentConfigured &&
		!activeSessionId &&
		!loading &&
		!creatingSession &&
		rawText.trim().length > 0;
	const canRunPrimaryAction =
		!loading &&
		!creatingSession &&
		(fileMode
			? Boolean(selectedFileMatch)
			: activeSessionId
				? sessionCanSend && rawText.trim().length > 0
				: suggestionMode === "app"
					? Boolean(selectedAppMatch)
					: pendingSlashAction
						? rawText.trim().length > 0
						: Boolean(selectedActionMatch));

	const updateRawText = useCallback(
		(nextValue: string, nextCaretIndex?: number) => {
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
		[activeSessionId],
	);

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

	const syncScrollableLayoutCaps = useCallback(() => {
		const shellElement = shellRef.current;
		const inputElement = inputRef.current;
		const { inputMaxHeight, outputMaxHeight } = resolveLauncherScrollableHeights();

		shellElement?.style.setProperty("--launcher-input-max-height", `${inputMaxHeight}px`);
		shellElement?.style.setProperty("--launcher-output-max-height", `${outputMaxHeight}px`);

		if (!(inputElement instanceof HTMLTextAreaElement) || inputMode !== "multiline") {
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
		if (!(inputElement instanceof HTMLTextAreaElement) || inputMode !== "multiline") {
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
		loading,
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

		if (inputElement instanceof HTMLTextAreaElement && inputMode === "multiline") {
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

	useLayoutEffect(() => {
		if (!workspacePickerOpen) {
			return;
		}

		const triggerElement = workspacePickerTriggerRef.current;
		const shellElement = shellRef.current;
		if (!triggerElement || !shellElement) {
			return;
		}

		const triggerRect = triggerElement.getBoundingClientRect();
		const shellRect = shellElement.getBoundingClientRect();
		const nextX = Math.max(0, triggerRect.left - shellRect.left);
		const nextY = Math.max(0, triggerRect.bottom - shellRect.top + 8);
		const nextWidth = Math.max(240, Math.ceil(triggerRect.width + 148));

		setWorkspacePickerOffset((currentOffset) => {
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
	}, [frameWidth, recentWorkspaceRoots.length, workspacePickerOpen]);

	useLayoutEffect(() => {
		if (!agentPickerOpen) {
			return;
		}

		const triggerElement = agentPickerTriggerRef.current;
		const shellElement = shellRef.current;
		if (!triggerElement || !shellElement) {
			return;
		}

		const triggerRect = triggerElement.getBoundingClientRect();
		const shellRect = shellElement.getBoundingClientRect();
		const nextX = Math.max(0, triggerRect.left - shellRect.left);
		const nextY = Math.max(0, triggerRect.bottom - shellRect.top + 8);
		const nextWidth = Math.max(220, Math.ceil(triggerRect.width + 36));

		setAgentPickerOffset((currentOffset) => {
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
	}, [agentCatalog.agents.length, agentPickerOpen, frameWidth, selectedAgentId]);

	useLayoutEffect(() => {
		if (!sessionPanelOpen) {
			return;
		}

		const triggerElement = sessionPanelTriggerRef.current;
		const shellElement = shellRef.current;
		if (!triggerElement || !shellElement) {
			return;
		}

		const triggerRect = triggerElement.getBoundingClientRect();
		const shellRect = shellElement.getBoundingClientRect();
		const nextWidth = clamp(Math.ceil(triggerRect.width + 136), 320, 420);
		const nextX = Math.max(0, triggerRect.right - shellRect.left - nextWidth);
		const nextY = Math.max(0, triggerRect.bottom - shellRect.top + 10);

		setSessionPanelOffset((currentOffset) => {
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
	}, [frameWidth, sessionPanelOpen, sessionSummaries.length, activeSessionId]);

	useEffect(() => {
		setSuggestionsHidden(false);
		setSelectedIndex(0);
	}, [suggestionMode, textBeforeCaret]);

	useEffect(() => {
		if (suggestionCount === 0) {
			if (selectedIndex !== 0) {
				setSelectedIndex(0);
			}
			return;
		}

		if (selectedIndex >= suggestionCount) {
			setSelectedIndex(suggestionCount - 1);
		}
	}, [selectedIndex, suggestionCount]);

	useLayoutEffect(() => {
		if (!hasSuggestions) {
			return;
		}

		const listElement = completionListRef.current;
		if (!listElement) {
			return;
		}

		const selectedElement = listElement.querySelector<HTMLElement>(
			`[data-suggestion-index="${selectedIndex}"]`,
		);
		if (!selectedElement) {
			return;
		}

		const nextScrollTop =
			selectedElement.offsetTop - listElement.clientHeight / 2 + selectedElement.offsetHeight / 2;
		const maxScrollTop = Math.max(0, listElement.scrollHeight - listElement.clientHeight);
		listElement.scrollTop = clamp(nextScrollTop, 0, maxScrollTop);
	}, [hasSuggestions, selectedIndex, visibleActionMatches, visibleFileMatches, visibleAppMatches]);

	useAutoResizeWindow(shellRef);

	useEffect(() => {
		let cancelled = false;
		let debounceTimer: number | null = null;

		async function loadSuggestions() {
			if (suggestionMode === "file") {
				setLoading(true);
				try {
					const nextMatches = await searchFiles(currentFileNeedle, 8);
					if (!cancelled) {
						showSuggestions({ fileMatches: nextMatches });
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						setError(getErrorMessage(loadError, "文件搜索失败"));
					}
				} finally {
					if (!cancelled) {
						setLoading(false);
					}
				}

				return;
			}

			if (suggestionMode === "action") {
				setLoading(true);
				try {
					const nextMatches = await matchActions(suggestionQuery);
					if (!cancelled) {
						showSuggestions({
							actionMatches: suggestionQuery.rawText.trim().startsWith("/")
								? nextMatches
								: nextMatches.slice(0, 8),
						});
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						setError(getErrorMessage(loadError, "动作匹配失败"));
					}
				} finally {
					if (!cancelled) {
						setLoading(false);
					}
				}

				return;
			}

			if (suggestionMode === "app") {
				setLoading(true);
				try {
					const nextMatches = await searchApps(textBeforeCaret, 8);
					if (!cancelled) {
						showSuggestions({ appMatches: nextMatches });
					}
				} catch (loadError) {
					if (!cancelled) {
						resetSuggestions();
						setError(getErrorMessage(loadError, "应用搜索失败"));
					}
				} finally {
					if (!cancelled) {
						setLoading(false);
					}
				}

				return;
			}

			if (!launcherMode) {
				resetSuggestions();
				setLoading(false);
				return;
			}

			resetSuggestions();
			setLoading(false);
		}

		if (suggestionMode === "file") {
			if (!isFileSearchReady(currentFileNeedle)) {
				resetSuggestions(true);
				setLoading(false);
				return () => {
					cancelled = true;
				};
			}

			debounceTimer = window.setTimeout(() => {
				void loadSuggestions();
			}, fileSearchDebounceMs);
		} else if (suggestionMode === "app") {
			if (!isAppSearchReady(textBeforeCaret)) {
				resetSuggestions(true);
				setLoading(false);
				return () => {
					cancelled = true;
				};
			}

			debounceTimer = window.setTimeout(() => {
				void loadSuggestions();
			}, appSearchDebounceMs);
		} else if (suggestionMode === "action") {
			void loadSuggestions();
		} else {
			resetSuggestions();
			setLoading(false);
		}

		return () => {
			cancelled = true;
			if (debounceTimer !== null) {
				window.clearTimeout(debounceTimer);
			}
		};
	}, [currentFileNeedle, launcherMode, suggestionMode, suggestionQuery, textBeforeCaret]);

	useEffect(() => {
		void getWorkspace().then((nextWorkspace) => {
			setWorkspaceState(nextWorkspace);
		});
		void getAcpAgents().then((catalog) => {
			setAgentCatalog(catalog);
			setSelectedAgentId(catalog.defaultAgentId ?? catalog.agents[0]?.id ?? null);
		});
		void takeAcpRestoreNotices().then((notices) => {
			setRestoreNotices(notices);
		});
		void listAcpSessions().then((summaries) => {
			setSessionSummaries(summaries);
			const activeSession = summaries.find((summary) => summary.isActive);
			setActiveSessionId(activeSession?.sessionId ?? null);
		});

		const workspaceUnlistenPromise = onWorkspaceUpdated((nextWorkspace) => {
			setWorkspaceState(nextWorkspace);
		});
		const ocrErrorUnlistenPromise = onOcrError((message) => {
			setError(message);
			scheduleLauncherInputFocus();
		});
		void subscribeAcpSessionUpdates((detail) => {
			setSessionDetails((current) => upsertSessionDetailRecord(current, detail));
			setSessionSummaries((current) => upsertSessionSummary(current, detail.session));
			if (detail.session.isActive) {
				setActiveSessionId(detail.session.sessionId);
			}
		});
		void subscribeAcpSessionRemovals((sessionId) => {
			setSessionDetails((current) => {
				const next = { ...current };
				delete next[sessionId];
				return next;
			});
			setSessionSummaries((current) => current.filter((item) => item.sessionId !== sessionId));
			setActiveSessionId((current) => (current === sessionId ? null : current));
		});

		return () => {
			void workspaceUnlistenPromise.then((unlisten) => unlisten?.());
			void ocrErrorUnlistenPromise.then((unlisten) => unlisten?.());
		};
	}, [scheduleLauncherInputFocus]);

	useEffect(() => {
		if (!desktopRuntimeAvailable) {
			return;
		}

		let unlisten: (() => void) | null = null;

		void onSelectedText((text) => {
			if (text && text.trim().length > 0) {
				pendingSelectionRef.current = 0;
				setInputMode("multiline");
				updateRawText(text, 0);
				scheduleLauncherInputFocus();
			}
		}).then((unlistenFn) => {
			unlisten = unlistenFn ?? null;
		});

		return () => {
			if (unlisten) {
				unlisten();
			}
		};
	}, [desktopRuntimeAvailable, scheduleLauncherInputFocus, updateRawText]);

	useEffect(() => {
		if (typeof window === "undefined" || typeof document === "undefined") {
			return;
		}

		let unlistenFocusChanged: (() => void) | null = null;

		function handleDocumentVisibilityChange() {
			if (document.visibilityState === "visible") {
				scheduleLauncherInputFocus();
			}
		}

		function handleWindowFocus() {
			scheduleLauncherInputFocus();
		}

		document.addEventListener("visibilitychange", handleDocumentVisibilityChange);
		window.addEventListener("focus", handleWindowFocus);

		if (desktopRuntimeAvailable) {
			void getCurrentWindow()
				.onFocusChanged(({ payload: focused }) => {
					if (focused) {
						scheduleLauncherInputFocus();
					}
				})
				.then((unlisten) => {
					unlistenFocusChanged = unlisten;
				});
		}

		return () => {
			clearScheduledLauncherInputFocus();
			document.removeEventListener("visibilitychange", handleDocumentVisibilityChange);
			window.removeEventListener("focus", handleWindowFocus);
			unlistenFocusChanged?.();
		};
	}, [clearScheduledLauncherInputFocus, desktopRuntimeAvailable, scheduleLauncherInputFocus]);

	useEffect(() => {
		if (!workspacePickerOpen) {
			return;
		}

		function handlePointerDown(event: MouseEvent) {
			const targetNode = event.target as Node;
			if (
				workspacePickerTriggerRef.current?.contains(targetNode) ||
				workspacePickerPanelRef.current?.contains(targetNode)
			) {
				return;
			}
			setWorkspacePickerOpen(false);
		}

		document.addEventListener("mousedown", handlePointerDown);
		return () => {
			document.removeEventListener("mousedown", handlePointerDown);
		};
	}, [workspacePickerOpen]);

	useEffect(() => {
		if (!agentPickerOpen) {
			return;
		}

		function handlePointerDown(event: MouseEvent) {
			const targetNode = event.target as Node;
			if (
				agentPickerTriggerRef.current?.contains(targetNode) ||
				agentPickerPanelRef.current?.contains(targetNode)
			) {
				return;
			}
			setAgentPickerOpen(false);
		}

		document.addEventListener("mousedown", handlePointerDown);
		return () => {
			document.removeEventListener("mousedown", handlePointerDown);
		};
	}, [agentPickerOpen]);

	useEffect(() => {
		if (!sessionPanelOpen) {
			return;
		}

		function handlePointerDown(event: MouseEvent) {
			const targetNode = event.target as Node;
			if (
				sessionPanelTriggerRef.current?.contains(targetNode) ||
				sessionPanelRef.current?.contains(targetNode)
			) {
				return;
			}
			setSessionPanelOpen(false);
		}

		document.addEventListener("mousedown", handlePointerDown);
		return () => {
			document.removeEventListener("mousedown", handlePointerDown);
		};
	}, [sessionPanelOpen]);

	useLayoutEffect(() => {
		const logElement = sessionLogRef.current;
		if (!logElement || !activeSessionScrollAnchor) {
			return;
		}

		logElement.scrollTop = 0;
	}, [activeSessionScrollAnchor]);

	function resetSuggestions(resetSelectedIndex: boolean = false) {
		setActionMatches([]);
		setFileMatches([]);
		setAppMatches([]);
		if (resetSelectedIndex) {
			setSelectedIndex(0);
		}
	}

	function showSuggestions(nextSuggestions: {
		actionMatches?: ActionMatch[];
		fileMatches?: FileSearchMatch[];
		appMatches?: InstalledAppMatch[];
	}) {
		setActionMatches(nextSuggestions.actionMatches ?? []);
		setFileMatches(nextSuggestions.fileMatches ?? []);
		setAppMatches(nextSuggestions.appMatches ?? []);
		setSuggestionsHidden(false);
		setSelectedIndex(0);
		setError(null);
	}

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

	function activatePendingSlashAction(descriptor: ActionDescriptor) {
		flushSync(() => {
			setActiveSlashAction(descriptor);
			pendingSelectionRef.current = 0;
			updateRawText("", 0);
			resetSuggestions(true);
		});
	}

	async function executeLauncherAction(
		descriptor: ActionDescriptor,
		query = fullQuery,
		options?: {
			onSuccess?: () => void;
		},
	) {
		setLoading(true);
		try {
			const executionResult = await executeAction({
				actionId: descriptor.id,
				query,
			});

			setLatestSubmittedText(query.rawText.trim() || descriptor.title);
			options?.onSuccess?.();
			setResult(executionResult);
			setError(null);

			// Hide the always-on-top launcher first so opener targets can take focus.
			if (executionResult.shouldCloseLauncher) {
				await hideLauncherWindow();
			}
			await applyClientEffect(executionResult);

			return true;
		} catch (executionError) {
			setError(getErrorMessage(executionError, "动作执行失败"));
			return false;
		} finally {
			setLoading(false);
		}
	}

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

	function acceptCompletion() {
		if (!completionText) {
			return;
		}

		const nextRawText =
			suggestionMode === "file" && activeFileToken
				? replaceTextRange(rawText, activeFileToken.start, activeFileToken.end, completionText)
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
		}
	}

	function acceptSelectedSuggestion(index: number = selectedIndex) {
		if (suggestionMode === "file") {
			selectFile(index);
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

	function resolveSelectedAction(index: number = selectedIndex) {
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
			? replaceTextRange(rawText, commandTokenRange.start, commandTokenRange.end, completedCommand)
			: replaceTextRange(rawText, 0, boundedCaretIndex, completedCommand);

		return {
			selected,
			slashInputMatch: parseSlashActionInput(nextRawText.value, selected.descriptor.aliases),
			resolvedRawText: nextRawText.value,
		};
	}

	async function runSelectedAction(index: number = selectedIndex) {
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
			explicitSlashSelection || slashInputMatch ? buildQuery(inputMode, actionRawText) : fullQuery;
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
	}

	async function runSelectedApp(index: number = selectedIndex) {
		const selected = visibleAppMatches[index];
		if (!selected) {
			return;
		}

		setLoading(true);
		try {
			const executionResult = await launchApp(selected.path);
			setLatestSubmittedText(rawText.trim() || selected.name);
			setResult(executionResult);
			setError(null);

			if (executionResult.shouldCloseLauncher) {
				await hideLauncherWindow();
			}
		} catch (launchError) {
			setError(getErrorMessage(launchError, "应用启动失败"));
		} finally {
			setLoading(false);
		}
	}

	async function runSessionPrompt() {
		if (!activeSessionId) {
			return;
		}

		const prompt = rawText.trim();
		if (!prompt) {
			return;
		}

		setLoading(true);
		try {
			const detail = await sendAcpPrompt(activeSessionId, prompt);
			setLatestSubmittedText(prompt);
			applySessionDetail(detail);
			resetComposer();
		} catch (promptError) {
			setError(getErrorMessage(promptError, "ACP prompt 发送失败"));
		} finally {
			setLoading(false);
		}
	}

	async function handleAgentExecute() {
		const prompt = rawText.trim();
		if (!prompt) {
			return;
		}

		setLoading(true);
		try {
			let targetSessionId = activeSessionId;

			if (!targetSessionId) {
				const createdDetail = await createAndActivateSession();
				targetSessionId = createdDetail.session.sessionId;
			}

			if (!targetSessionId) {
				throw new Error("ACP session 创建失败");
			}

			const detail = await sendAcpPrompt(targetSessionId, prompt);
			setLatestSubmittedText(prompt);
			applySessionDetail(detail);
			resetComposer();
		} catch (agentError) {
			setError(getErrorMessage(agentError, "Agent 执行失败"));
		} finally {
			setLoading(false);
		}
	}

	async function runPrimaryAction(index: number = selectedIndex) {
		if (suggestionMode === "file") {
			selectFile(index);
			return;
		}

		if (activeSessionId) {
			await runSessionPrompt();
			return;
		}

		if (suggestionMode === "app") {
			await runSelectedApp(index);
			return;
		}

		if (pendingSlashAction) {
			await runPendingSlashAction();
			return;
		}

		await runSelectedAction(index);
	}

	async function handleKeyDown(
		event: KeyboardEvent<HTMLInputElement> | KeyboardEvent<HTMLTextAreaElement>,
	) {
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

			if (agentPickerOpen) {
				setAgentPickerOpen(false);
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

			await hideLauncherWindow();
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

		if ((event.metaKey || event.ctrlKey) && inputMode === "inline") {
			event.preventDefault();
			setInputMode("multiline");
			const newText = rawText.slice(0, boundedCaretIndex) + "\n" + rawText.slice(boundedCaretIndex);
			pendingSelectionRef.current = boundedCaretIndex + 1;
			updateRawText(newText, boundedCaretIndex + 1);
			return;
		}

		if ((event.metaKey || event.ctrlKey) && inputMode === "multiline") {
			event.preventDefault();
			await runPrimaryAction();
			return;
		}

		if (inputMode === "inline" && !event.shiftKey) {
			event.preventDefault();
			await runPrimaryAction();
			return;
		}

		if (inputMode === "multiline" && hasSuggestions) {
			event.preventDefault();
			if (suggestionMode === "action") {
				await runSelectedAction();
				return;
			}

			if (suggestionMode === "file") {
				acceptSelectedSuggestion();
				return;
			}

			await runPrimaryAction();
		}
	}

	function syncCaretIndex(element: HTMLInputElement | HTMLTextAreaElement) {
		setCaretIndex(element.selectionEnd ?? 0);
	}

	async function applyWorkspaceSelection(path: string, fallbackMessage: string) {
		try {
			const nextWorkspace = await setWorkspace(path);
			setWorkspaceState(nextWorkspace);
			setError(null);
		} catch (workspaceError) {
			setError(getErrorMessage(workspaceError, fallbackMessage));
		}
	}

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

	async function handleWorkspaceCrumbClick(path: string) {
		setWorkspacePickerOpen(false);
		await applyWorkspaceSelection(path, "切换工作目录失败");
	}

	async function handleRecentWorkspaceClick(path: string) {
		setWorkspacePickerOpen(false);
		await applyWorkspaceSelection(path, "切换最近目录失败");
	}

	function handleAgentSelect(agent: AcpAgentConfig) {
		setSelectedAgentId(agent.id);
		setAgentPickerOpen(false);
		setError(null);
	}

	async function createAndActivateSession() {
		const detail = await createAcpSession(selectedAgent?.id ?? null);
		setSessionDetails((current) => upsertSessionDetailRecord(current, detail));
		const summaries = await activateAcpSession(detail.session.sessionId);
		setSessionSummaries(summaries);
		setActiveSessionId(detail.session.sessionId);
		return detail;
	}

	async function handleCreateSession() {
		setCreatingSession(true);
		try {
			await createAndActivateSession();
			setSessionPanelOpen(true);
			setResult(null);
			setError(null);
		} catch (sessionError) {
			setError(getErrorMessage(sessionError, "创建 ACP session 失败"));
		} finally {
			setCreatingSession(false);
		}
	}

	async function handleSessionDotClick(sessionId: string) {
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
	}

	async function handleSessionPanelSelect(sessionId: string) {
		await handleSessionDotClick(sessionId);
		setSessionPanelOpen(false);
	}

	async function closeSessionById(sessionId: string, collapsePanelWhenLast: boolean) {
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
	}

	async function handleCloseSession(sessionId: string) {
		await closeSessionById(sessionId, true);
	}

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

	const inputPlaceholder = activeSessionId
		? "向当前 ACP session 发送消息，或用 @ 插入当前 workspace 文件路径"
		: pendingSlashAction
			? `已选择 ${pendingSlashAction.aliases[0] ?? pendingSlashAction.title}，输入待处理文本后按 Enter`
			: "输入应用名，或用 / 执行动作、@ 搜索当前 workspace 文件";

	return (
		<main className="launcher-shell" ref={shellRef}>
			<section className="launcher-frame" ref={frameRef} style={{ width: `${frameWidth}px` }}>
				<LauncherHeader
					workspacePickerOpen={workspacePickerOpen}
					workspacePickerTriggerRef={workspacePickerTriggerRef}
					workspaceBreadcrumbs={workspaceBreadcrumbs}
					onToggleWorkspacePicker={() => setWorkspacePickerOpen((current) => !current)}
					onSelectWorkspaceCrumb={(path) => void handleWorkspaceCrumbClick(path)}
					onWorkspaceDragStart={handleWorkspaceDragStart}
					agentConfigured={agentConfigured}
					agentPickerOpen={agentPickerOpen}
					agentPickerTriggerRef={agentPickerTriggerRef}
					selectedAgentName={selectedAgent?.name ?? null}
					onToggleAgentPicker={() => setAgentPickerOpen((current) => !current)}
					creatingSession={creatingSession}
					onCreateSession={() => void handleCreateSession()}
					sessionPanelOpen={sessionPanelOpen}
					sessionPanelTriggerRef={sessionPanelTriggerRef}
					sessionTriggerSummary={sessionTriggerSummary}
					sessionCount={sessionSummaries.length}
					visibleSessionDots={visibleSessionDots}
					activeSessionId={activeSessionId}
					overflowSessionCount={overflowSessionCount}
					onToggleSessionPanel={() => setSessionPanelOpen((current) => !current)}
					onSelectSessionDot={(sessionId) => void handleSessionDotClick(sessionId)}
					onOpenSessionPanel={() => setSessionPanelOpen(true)}
				/>

				<RestoreNoticeList restoreNotices={restoreNotices} />

				<LauncherComposer
					inputMode={inputMode}
					inputRef={inputRef}
					rawText={rawText}
					inputPlaceholder={inputPlaceholder}
					statusText={statusBarText}
					onUpdateRawText={updateRawText}
					onSyncCaretIndex={syncCaretIndex}
					onKeyDown={handleKeyDown}
					onOpenSettings={onOpenSettings}
					hasCompletion={hasCompletion}
					onAcceptCompletion={acceptCompletion}
					loading={loading}
					creatingSession={creatingSession}
					showAgentAction={showAgentAction}
					primaryActionShortcutLabel={primaryActionShortcutLabel}
					primaryActionLabel={primaryActionLabel}
					canRunPrimaryAction={canRunPrimaryAction}
					agentActionShortcutLabel={agentActionShortcutLabel}
					onAgentExecute={() => void handleAgentExecute()}
					showCancelActiveSession={showCancelActiveSession}
					onCancelActiveSession={() => void handleCancelActiveSession()}
					onRunPrimaryAction={() => void runPrimaryAction()}
					onCloseLauncher={() => void hideLauncherWindow()}
				/>

				<LauncherFeedback
					activeSession={activeSession}
					sessionLogRef={sessionLogRef}
					result={result}
					jsonPreview={jsonPreview}
					markdownPreview={markdownPreview}
					workspace={workspace}
					error={error}
				/>
			</section>

			<CompletionPopup
				hasSuggestions={hasSuggestions}
				suggestionMode={suggestionMode}
				completionOffset={completionOffset}
				completionListRef={completionListRef}
				selectedIndex={selectedIndex}
				visibleFileMatches={visibleFileMatches}
				visibleActionMatches={visibleActionMatches}
				visibleAppMatches={visibleAppMatches}
				onSelectIndex={setSelectedIndex}
				onSelectFile={selectFile}
				onRunSelectedAction={(index) => void runSelectedAction(index)}
				onRunSelectedApp={(index) => void runSelectedApp(index)}
			/>

			<SessionPanel
				open={sessionPanelOpen}
				offset={sessionPanelOffset}
				panelRef={sessionPanelRef}
				agentConfigured={agentConfigured}
				sessionSummaries={sessionSummaries}
				sessionRunningCount={sessionRunningCount}
				sessionAttentionCount={sessionAttentionCount}
				activeSessionId={activeSessionId}
				activeSessionSummary={activeSessionSummary}
				workspace={workspace}
				onSelectSession={(sessionId) => void handleSessionPanelSelect(sessionId)}
				onCloseSession={(sessionId) => void handleCloseSession(sessionId)}
			/>

			<WorkspacePickerPanel
				open={workspacePickerOpen}
				offset={workspacePickerOffset}
				panelRef={workspacePickerPanelRef}
				recentWorkspaceRoots={recentWorkspaceRoots}
				workspace={workspace}
				onPickWorkspace={() => void handleWorkspacePick()}
				onSelectRecentWorkspace={(path) => void handleRecentWorkspaceClick(path)}
			/>

			<AgentPickerPanel
				open={agentPickerOpen}
				offset={agentPickerOffset}
				panelRef={agentPickerPanelRef}
				agents={agentCatalog.agents}
				selectedAgentId={selectedAgent?.id ?? null}
				onSelectAgent={handleAgentSelect}
			/>
		</main>
	);
}
