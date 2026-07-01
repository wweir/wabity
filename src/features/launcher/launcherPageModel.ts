import { openUrl } from "@tauri-apps/plugin-opener";

import type {
	AcpSessionDetail,
	AcpSessionMessage,
	RagRuntimeStatus,
	WorkspaceState,
} from "../../lib/tauri/types";
import { ragAnswerActionDescriptor } from "./actionCatalog";
import { parseSlashActionInput, type SuggestionMode } from "./query";
import { formatWorkspacePath } from "./workspace";
import type {
	ActionDescriptor,
	ActionMatch,
	ExecutionResult,
	FileSearchMatch,
	InstalledAppMatch,
	InputMode,
	RagAnswerStructuredPayload,
	RunningProcessMatch,
} from "./types";

export const defaultInputMode: InputMode = "inline";
export const launcherCompletionPopupId = "launcher-completion-popup";
export const launcherCompletionOptionIdPrefix = "launcher-completion-option";
export const QA_RESULT_BLUR_AUTO_HIDE_SUPPRESSION_MS = 1_500;
export const QA_RESULT_BLUR_AUTO_HIDE_RESTORE_AFTER_SETTLE_MS = 900;

const CLIPBOARD_HISTORY_PINNED_HOTKEY_START_CODE = "A".charCodeAt(0);

export type PrimaryActionTone = "qa" | "execute" | "send" | "path" | "translate";

export type PrimaryActionState =
	| { kind: "search_path"; label: string; tone: PrimaryActionTone; enabled: false }
	| { kind: "insert_path"; label: string; tone: PrimaryActionTone; enabled: true }
	| { kind: "send"; label: string; tone: PrimaryActionTone; enabled: boolean }
	| {
			kind: "pending_action" | "run_action";
			label: string;
			tone: PrimaryActionTone;
			enabled: boolean;
	  }
	| { kind: "rag_answer"; label: string; tone: PrimaryActionTone; enabled: true }
	| { kind: "launch_app"; label: string; tone: PrimaryActionTone; enabled: boolean }
	| { kind: "run_kill"; label: string; tone: PrimaryActionTone; enabled: boolean }
	| { kind: "idle"; label: string; tone: PrimaryActionTone; enabled: false };

export interface LauncherStatusBarState {
	announce: boolean;
	label: string;
	items: string[];
	tone: "default" | "progress" | "error";
}

interface LauncherStatusBarStateInput {
	activeSession: AcpSessionDetail | null;
	activeSessionBusy: boolean;
	clipboardHistoryShortcut: string;
	creatingSession: boolean;
	error: string | null;
	hasSettingsAction: boolean;
	latestSubmittedText: string | null;
	operationStatusText: string | null;
	ragRuntimeBar: ReturnType<typeof buildRagRuntimeStatusText>;
	shouldShowQaMessages: boolean;
	showShortcutHint: boolean;
	workspace: WorkspaceState;
}

interface PrimaryActionStateInput {
	activeSessionId: string | null;
	appSearchActive: boolean;
	fileMode: boolean;
	killSearchQuery: string;
	pendingSlashAction: ActionDescriptor | null;
	rawText: string;
	selectedActionMatch: ActionMatch | undefined;
	selectedAppMatch: InstalledAppMatch | undefined;
	selectedFileMatch: FileSearchMatch | undefined;
	sessionCanSend: boolean;
	shouldFallbackToRagAnswer: boolean;
	suggestionMode: SuggestionMode;
}

export function derivePrimaryActionState({
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
}: PrimaryActionStateInput): PrimaryActionState {
	if (fileMode) {
		return selectedFileMatch
			? {
					kind: "insert_path",
					label: "插入路径",
					tone: "path",
					enabled: true,
				}
			: {
					kind: "search_path",
					label: "搜索路径",
					tone: "path",
					enabled: false,
				};
	}

	if (activeSessionId) {
		return {
			kind: "send",
			label: "发送",
			tone: "send",
			enabled: sessionCanSend && rawText.trim().length > 0,
		};
	}

	if (pendingSlashAction) {
		const actionView = describePrimaryActionDescriptor(pendingSlashAction);
		return {
			kind: "pending_action",
			...actionView,
			enabled: rawText.trim().length > 0,
		};
	}

	if (suggestionMode === "kill") {
		return {
			kind: "run_kill",
			label: "终止进程",
			tone: "execute",
			enabled: killSearchQuery.trim().length > 0,
		};
	}

	if (shouldFallbackToRagAnswer) {
		return {
			kind: "rag_answer",
			label: "问答",
			tone: "qa",
			enabled: true,
		};
	}

	if (appSearchActive) {
		return {
			kind: "launch_app",
			label: "执行",
			tone: "execute",
			enabled: Boolean(selectedAppMatch),
		};
	}

	if (selectedActionMatch) {
		const actionView = describePrimaryActionDescriptor(selectedActionMatch.descriptor);
		return {
			kind: "run_action",
			...actionView,
			enabled: true,
		};
	}

	return {
		kind: "idle",
		label: "执行",
		tone: "execute",
		enabled: false,
	};
}

export function buildLauncherStatusBarState({
	activeSession,
	activeSessionBusy,
	clipboardHistoryShortcut,
	creatingSession,
	error,
	hasSettingsAction,
	latestSubmittedText,
	operationStatusText,
	ragRuntimeBar,
	shouldShowQaMessages,
	showShortcutHint,
	workspace,
}: LauncherStatusBarStateInput): LauncherStatusBarState {
	const sessionInfoItems =
		activeSession && activeSession.session.workspaceRoot
			? [
					`${activeSession.session.title} · ${activeSession.session.agentName}`,
					formatWorkspacePath(activeSession.session.workspaceRoot, workspace),
				]
			: [];
	const qaInfoItems = shouldShowQaMessages
		? ["问答会话", formatWorkspacePath(workspace.rootPath, workspace)]
		: [];

	if (operationStatusText) {
		const items = [operationStatusText, ...(activeSession ? sessionInfoItems : qaInfoItems)];
		if (error) {
			items.push(`错误 · ${error}`);
		}

		return {
			announce: true,
			label: "运行状态",
			items,
			tone: error ? "error" : "progress",
		};
	}

	if (creatingSession) {
		const items = ["正在创建 Agent 会话", ...sessionInfoItems];
		if (error) {
			items.push(`错误 · ${error}`);
		}

		return {
			announce: true,
			label: "运行状态",
			items,
			tone: error ? "error" : "progress",
		};
	}

	if (activeSessionBusy) {
		const items = [
			activeSession?.session.agentName
				? `Agent 执行中 · ${activeSession.session.agentName}`
				: "Agent 执行中 · 正在等待响应",
			...sessionInfoItems,
		];
		if (error) {
			items.push(`错误 · ${error}`);
		}

		return {
			announce: true,
			label: "运行状态",
			items,
			tone: error ? "error" : "progress",
		};
	}

	if (ragRuntimeBar.text) {
		const items = [ragRuntimeBar.text];
		if (error) {
			items.push(`错误 · ${error}`);
		}

		return {
			announce: true,
			label: "运行状态",
			items,
			tone: error ? "error" : ragRuntimeBar.tone,
		};
	}

	if (activeSession) {
		const items = [...sessionInfoItems];
		if (activeSession.session.lastError) {
			items.push(`错误 · ${activeSession.session.lastError}`);
		}
		if (error && error !== activeSession.session.lastError) {
			items.push(`错误 · ${error}`);
		}

		return {
			announce: Boolean(error) || Boolean(activeSession.session.lastError),
			label: "会话状态",
			items,
			tone: items.some((item) => item.startsWith("错误 ·")) ? "error" : "default",
		};
	}

	if (shouldShowQaMessages) {
		const items = [...qaInfoItems];
		if (error) {
			items.push(`错误 · ${error}`);
		}

		return {
			announce: Boolean(error),
			label: "会话状态",
			items,
			tone: error ? "error" : "default",
		};
	}

	const items = latestSubmittedText
		? [latestSubmittedText]
		: showShortcutHint
			? [
					"Esc 关闭",
					`${clipboardHistoryShortcut} 历史剪贴板`,
					hasSettingsAction ? "快捷键可在设置中修改" : "",
				]
			: [];
	if (error) {
		items.push(`错误 · ${error}`);
	}

	return {
		announce: Boolean(error),
		label: error ? "状态" : showShortcutHint ? "快捷提示" : "最近输入",
		items,
		tone: error ? "error" : "default",
	};
}

export function getPinnedClipboardHotkeyIndex(code: string): number | null {
	if (!/^Key[A-Z]$/.test(code)) {
		return null;
	}

	return code.charCodeAt(code.length - 1) - CLIPBOARD_HISTORY_PINNED_HOTKEY_START_CODE;
}

export function getRecentClipboardHotkeyIndex(code: string): number | null {
	if (!/^Digit\d$/.test(code)) {
		return null;
	}

	const digit = code.charAt(code.length - 1);
	return digit === "0" ? 9 : Number(digit) - 1;
}

export async function applyClientEffect(result: ExecutionResult): Promise<void> {
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

export function resolveLauncherOperationStatus(actionId: string): string | null {
	switch (actionId) {
		case "rag_answer":
			return "模型请求中 · 正在生成文档回答";
		case "translate_text":
			return "模型请求中 · 正在翻译文本";
		default:
			return null;
	}
}

export function extractQaPrompt(rawText: string): string {
	const slashInput = parseSlashActionInput(rawText, ragAnswerActionDescriptor.aliases);
	return (slashInput?.content ?? rawText).trim();
}

export function describePrimaryActionDescriptor(descriptor: ActionDescriptor): {
	label: string;
	tone: PrimaryActionTone;
} {
	switch (descriptor.id) {
		case "rag_answer":
			return {
				label: "问答",
				tone: "qa",
			};
		case "translate_text":
			return {
				label: "翻译",
				tone: "translate",
			};
		default:
			return {
				label: descriptor.title,
				tone: "execute",
			};
	}
}

export function deriveKillCompletion(match: RunningProcessMatch | undefined): string | null {
	if (!match) {
		return null;
	}

	return `pid:${match.pid}`;
}

export function buildQaAssistantMessageBlocks(
	result: ExecutionResult,
	payload: RagAnswerStructuredPayload | null,
): AcpSessionMessage["blocks"] {
	const blocks: AcpSessionMessage["blocks"] = [];
	const reasoning = payload?.reasoning?.trim();
	if (result.primaryText) {
		blocks.push({
			type: "content",
			text: result.primaryText,
		});
	}
	const actions = payload?.actions ?? [];
	if (actions.length > 0) {
		blocks.push({
			type: "actions",
			items: actions,
		});
	}
	if (reasoning) {
		blocks.push({
			type: "thought",
			content: reasoning,
		});
	}

	return blocks;
}

export function buildRagRuntimeStatusText(status: RagRuntimeStatus): {
	text: string | null;
	tone: "default" | "progress" | "error";
} {
	const warningSuffix = status.warningCount > 0 ? ` · ${status.warningCount} 条警告` : "";
	const latestWarning =
		status.recentWarnings.length > 0
			? status.recentWarnings[status.recentWarnings.length - 1]
			: null;
	switch (status.phase) {
		case "scanning":
			return {
				text:
					status.totalFileCount > 0
						? `索引构建中 · 已扫描 ${status.scannedFileCount} 个文件，当前进度 ${status.completedFileCount}/${status.totalFileCount}${warningSuffix}`
						: status.scannedFileCount > 0
							? `索引构建中 · 已扫描 ${status.scannedFileCount} 个文件${warningSuffix}`
							: `索引构建中 · 正在扫描文档${warningSuffix}`,
				tone: "progress",
			};
		case "indexing":
			return {
				text:
					status.totalFileCount > 0
						? `索引构建中 · 当前进度 ${status.completedFileCount}/${status.totalFileCount}（剩余 ${status.pendingFileCount} 个文件）${warningSuffix}`
						: status.pendingFileCount > 0
							? `索引构建中 · 正在计算向量（剩余 ${status.pendingFileCount} 个文件）${warningSuffix}`
							: `索引构建中 · 正在计算向量${warningSuffix}`,
				tone: "progress",
			};
		case "error":
			return {
				text: status.lastError
					? `索引构建失败 · ${status.lastError}`
					: "索引构建失败 · 请检查 RAG 配置和日志",
				tone: "error",
			};
		case "idle":
			if (status.warningCount > 0) {
				return {
					text: latestWarning
						? `最近一次索引构建有 ${status.warningCount} 条警告 · ${latestWarning}`
						: `最近一次索引构建有 ${status.warningCount} 条警告`,
					tone: "default",
				};
			}
			return { text: null, tone: "default" };
		default:
			return { text: null, tone: "default" };
	}
}
