import type { AcpSessionDetail, AcpSessionSummary } from "../../lib/tauri/types";

export const visibleSessionDotsLimit = 6;

function mergeSessionSummary(current: AcpSessionSummary | undefined, next: AcpSessionSummary) {
	if (!current) {
		return next;
	}

	if (next.lastUpdatedAtMs !== current.lastUpdatedAtMs) {
		return next.lastUpdatedAtMs > current.lastUpdatedAtMs ? next : current;
	}

	const currentScore =
		Number(current.attention) + Number(current.isActive) + Number(Boolean(current.lastError));
	const nextScore =
		Number(next.attention) + Number(next.isActive) + Number(Boolean(next.lastError));
	if (nextScore !== currentScore) {
		return nextScore > currentScore ? next : current;
	}

	return next;
}

function mergeSessionDetail(current: AcpSessionDetail | undefined, next: AcpSessionDetail) {
	if (!current) {
		return next;
	}

	if (next.session.lastUpdatedAtMs < current.session.lastUpdatedAtMs) {
		return current;
	}

	return next;
}

export function upsertSessionSummary(current: AcpSessionSummary[], next: AcpSessionSummary) {
	const existing = current.find((item) => item.sessionId === next.sessionId);
	const withoutCurrent = current.filter((item) => item.sessionId !== next.sessionId);
	withoutCurrent.push(mergeSessionSummary(existing, next));
	withoutCurrent.sort((left, right) => {
		if (right.lastUpdatedAtMs !== left.lastUpdatedAtMs) {
			return right.lastUpdatedAtMs - left.lastUpdatedAtMs;
		}
		return left.sessionId.localeCompare(right.sessionId);
	});
	return withoutCurrent;
}

export function upsertSessionDetailRecord(
	current: Record<string, AcpSessionDetail>,
	next: AcpSessionDetail,
): Record<string, AcpSessionDetail> {
	return {
		...current,
		[next.session.sessionId]: mergeSessionDetail(current[next.session.sessionId], next),
	};
}

export function formatSessionStatus(session: AcpSessionSummary) {
	switch (session.status) {
		case "starting":
			return "启动中";
		case "idle":
			return "空闲";
		case "running":
			return "运行中";
		case "error":
			return session.errorLevel === "fatal" ? "不可恢复错误" : "可恢复错误";
		case "exited":
			return "已断开";
		default:
			return session.status;
	}
}

export function buildSessionStatusClassName(session: AcpSessionSummary) {
	switch (session.status) {
		case "starting":
			return "session-panel-status-starting";
		case "idle":
			return "session-panel-status-idle";
		case "running":
			return "session-panel-status-running";
		case "error":
			return session.errorLevel === "fatal"
				? "session-panel-status-fatal"
				: "session-panel-status-recoverable";
		case "exited":
			return "session-panel-status-disconnected";
		default:
			return "session-panel-status-idle";
	}
}

export function buildSessionTriggerSummary(
	sessionSummaries: AcpSessionSummary[],
	activeSession: AcpSessionSummary | null,
	runningCount: number,
	attentionCount: number,
) {
	if (sessionSummaries.length === 0) {
		return "无会话";
	}

	if (activeSession) {
		return `${formatSessionStatus(activeSession)} · ${activeSession.title}`;
	}

	if (attentionCount > 0) {
		return `${attentionCount} 个待处理更新`;
	}

	if (runningCount > 0) {
		return `${runningCount} 个运行中`;
	}

	return `${sessionSummaries.length} 个已连接`;
}

export function buildSessionDotClassName(
	session: AcpSessionSummary,
	activeSessionId: string | null,
) {
	return [
		"session-dot",
		session.sessionId === activeSessionId ? "active" : "",
		session.attention ? "attention" : "",
		session.status === "running" ? "running" : "",
		session.status === "exited" ? "disconnected" : "",
		session.status === "error" && session.errorLevel === "fatal" ? "fatal-error" : "",
		session.status === "error" && session.errorLevel !== "fatal" ? "recoverable-error" : "",
	]
		.filter(Boolean)
		.join(" ");
}
