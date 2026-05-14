import type {
	ActionMatch,
	ExecutionRequest,
	ExecutionResult,
	FileSearchMatch,
	InstalledAppMatch,
	QueryPayload,
	RunningProcessMatch,
} from "../../../features/launcher/types";
import {
	executeActionFallback,
	launchAppFallback,
	matchActionsFallback,
	searchAppsFallback,
	searchProcessesFallback,
} from "../../../features/launcher/fallback";
import {
	browserRagRuntimeStatus,
	buildBrowserSessionDetail,
	browserWorkspaceState,
} from "./defaults";
import {
	invokeIfDesktop,
	invokeOrDefault,
	listenIfDesktop,
	resolveCurrentWindowLabel,
	subscribeChannelIfDesktop,
	type ExecutionProgressEvent,
	type OcrTranslationResultEvent,
	type OcrTranslationStartedEvent,
	type OcrTranslationStreamEvent,
} from "./runtime";
import type {
	AcpRestoreNotice,
	AcpSessionDetail,
	AcpSessionSummary,
	RagRuntimeStatus,
	ScreenCaptureRect,
	ScreenshotReviewAction,
	LauncherFailureEvent,
	ScreenshotReviewPayload,
	WorkspaceState,
} from "../types";

export async function matchActions(query: QueryPayload): Promise<ActionMatch[]> {
	return invokeOrDefault("match_actions", () => matchActionsFallback(query), { query });
}

export async function searchFiles(query: string, limit = 8): Promise<FileSearchMatch[]> {
	return invokeOrDefault("search_files", [], { query, limit });
}

export async function searchApps(query: string, limit = 8): Promise<InstalledAppMatch[]> {
	return invokeOrDefault("search_apps", () => searchAppsFallback(query), { query, limit });
}

export async function searchProcesses(query: string, limit = 8): Promise<RunningProcessMatch[]> {
	return invokeOrDefault("search_processes", () => searchProcessesFallback(query), {
		query,
		limit,
	});
}

export async function executeAction(request: ExecutionRequest): Promise<ExecutionResult> {
	return invokeOrDefault("execute_action", () => executeActionFallback(request), { request });
}

export async function launchApp(path: string): Promise<ExecutionResult> {
	return invokeOrDefault("launch_app", () => launchAppFallback(path), { path });
}

export async function openDocumentReference(path: string): Promise<void> {
	return invokeIfDesktop("open_document_reference", { path });
}

export async function hideLauncherWindow(): Promise<void> {
	return invokeIfDesktop("hide_launcher_window", {
		windowLabel: await resolveCurrentWindowLabel(),
	});
}

export async function beginTransientWindowInteraction(): Promise<void> {
	return invokeIfDesktop("begin_transient_window_interaction");
}

export async function endTransientWindowInteraction(): Promise<void> {
	return invokeIfDesktop("end_transient_window_interaction");
}

export async function armLauncherBlurAutoHideSuppression(durationMs: number): Promise<void> {
	return invokeIfDesktop("arm_launcher_blur_auto_hide_suppression", {
		durationMs: Math.max(1, Math.ceil(durationMs)),
	});
}

export async function setLauncherBlurAutoHideEnabled(enabled: boolean): Promise<void> {
	return invokeIfDesktop("set_launcher_blur_auto_hide_enabled", { enabled });
}

export async function getLauncherPinned(): Promise<boolean> {
	return invokeOrDefault("get_launcher_pinned", false);
}

export async function setLauncherPinned(pinned: boolean): Promise<boolean> {
	return invokeOrDefault("set_launcher_pinned", pinned, { pinned });
}

export async function onOcrTranslationResult(
	callback: (payload: OcrTranslationResultEvent) => void,
) {
	return listenIfDesktop("ocr-translation-result", callback);
}

export async function onOcrTranslationStarted(
	callback: (payload: OcrTranslationStartedEvent) => void,
) {
	return listenIfDesktop("ocr-translation-started", callback);
}

export async function onOcrTranslationStream(
	callback: (payload: OcrTranslationStreamEvent) => void,
) {
	return listenIfDesktop("ocr-translation-stream", callback);
}

export async function onExecutionProgress(callback: (payload: ExecutionProgressEvent) => void) {
	return listenIfDesktop("execution-progress", callback);
}

export async function onScreenshotReviewStarted(
	callback: (payload: ScreenshotReviewPayload) => void,
) {
	return listenIfDesktop("screenshot-review-started", callback);
}

export async function onLauncherFailure(callback: (payload: LauncherFailureEvent) => void) {
	return listenIfDesktop("launcher-failure", callback);
}

export async function getScreenshotReviewPreview(sessionId: string): Promise<string> {
	return invokeOrDefault("get_screenshot_review_preview", "", { sessionId });
}

export async function confirmScreenshotReview(
	sessionId: string,
	action: ScreenshotReviewAction,
	editedText: string | null,
): Promise<void> {
	return invokeIfDesktop("confirm_screenshot_review", {
		sessionId,
		action,
		editedText,
	});
}

export async function cancelScreenshotReview(sessionId: string): Promise<void> {
	return invokeIfDesktop("cancel_screenshot_review", { sessionId });
}

export async function retryScreenshotReview(sessionId: string): Promise<void> {
	return invokeIfDesktop("retry_screenshot_review", { sessionId });
}

export async function completeScreenCaptureRegion(
	token: string,
	rect: ScreenCaptureRect,
): Promise<void> {
	return invokeIfDesktop("complete_screen_capture_region", { token, rect });
}

export async function cancelScreenCaptureRegion(token: string): Promise<void> {
	return invokeIfDesktop("cancel_screen_capture_region", { token });
}

export async function onRagRuntimeStatus(callback: (payload: RagRuntimeStatus) => void) {
	return listenIfDesktop("rag-runtime-status", callback);
}

export async function getRagRuntimeStatus(): Promise<RagRuntimeStatus> {
	return invokeOrDefault("get_rag_runtime_status", browserRagRuntimeStatus);
}

export async function getWorkspace(): Promise<WorkspaceState> {
	return invokeOrDefault("get_workspace", browserWorkspaceState);
}

export async function setWorkspace(rootPath: string): Promise<WorkspaceState> {
	return invokeOrDefault(
		"set_workspace",
		{ ...browserWorkspaceState, rootPath, recentRoots: [rootPath] },
		{ rootPath },
	);
}

export async function onWorkspaceUpdated(callback: (workspace: WorkspaceState) => void) {
	return listenIfDesktop("workspace-updated", callback);
}

export async function listAcpSessions(): Promise<AcpSessionSummary[]> {
	return invokeOrDefault("list_acp_sessions", []);
}

export async function takeAcpRestoreNotices(): Promise<AcpRestoreNotice[]> {
	return invokeOrDefault("take_acp_restore_notices", []);
}

export async function getAcpSessionDetail(sessionId: string): Promise<AcpSessionDetail | null> {
	return invokeOrDefault("get_acp_session_detail", null, { sessionId });
}

export async function createAcpSession(agentId: string | null = null): Promise<AcpSessionDetail> {
	return invokeOrDefault("create_acp_session", () => buildBrowserSessionDetail("browser-session"), {
		agentId,
	});
}

export async function activateAcpSession(sessionId: string | null): Promise<AcpSessionSummary[]> {
	return invokeOrDefault("activate_acp_session", [], { sessionId });
}

export async function sendAcpPrompt(sessionId: string, prompt: string): Promise<AcpSessionDetail> {
	return invokeOrDefault("send_acp_prompt", () => buildBrowserSessionDetail(sessionId), {
		sessionId,
		prompt,
	});
}

export async function setAcpSessionMode(
	sessionId: string,
	modeId: string,
): Promise<AcpSessionDetail> {
	return invokeOrDefault("set_acp_session_mode", () => buildBrowserSessionDetail(sessionId), {
		sessionId,
		modeId,
	});
}

export async function setAcpSessionConfigOption(
	sessionId: string,
	configId: string,
	valueId: string,
): Promise<AcpSessionDetail> {
	return invokeOrDefault(
		"set_acp_session_config_option",
		() => buildBrowserSessionDetail(sessionId),
		{ sessionId, configId, valueId },
	);
}

export async function cancelAcpSession(sessionId: string): Promise<void> {
	return invokeIfDesktop("cancel_acp_session", { sessionId });
}

export async function closeAcpSession(sessionId: string): Promise<void> {
	return invokeIfDesktop("close_acp_session", { sessionId });
}

export async function subscribeAcpSessionUpdates(callback: (detail: AcpSessionDetail) => void) {
	return subscribeChannelIfDesktop(
		"subscribe_acp_session_updates",
		"unsubscribe_acp_session_updates",
		callback,
	);
}

export async function subscribeAcpSessionRemovals(callback: (sessionId: string) => void) {
	return subscribeChannelIfDesktop(
		"subscribe_acp_session_removals",
		"unsubscribe_acp_session_removals",
		callback,
	);
}
