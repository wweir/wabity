import type { RefObject } from "react";
import type { AcpSessionSummary, WorkspaceState } from "../../../lib/tauri/types";
import type { FloatingPanelOffset } from "../types";
import { buildSessionStatusClassName, formatSessionStatus } from "../sessions";
import { formatWorkspacePath } from "../workspace";

interface SessionPanelProps {
	open: boolean;
	offset: FloatingPanelOffset;
	panelRef: RefObject<HTMLElement | null>;
	agentConfigured: boolean;
	sessionSummaries: AcpSessionSummary[];
	sessionRunningCount: number;
	sessionAttentionCount: number;
	activeSessionId: string | null;
	workspace: WorkspaceState;
	onSelectSession: (sessionId: string) => void;
	onCloseSession: (sessionId: string) => void;
}

export function SessionPanel({
	open,
	offset,
	panelRef,
	agentConfigured,
	sessionSummaries,
	sessionRunningCount,
	sessionAttentionCount,
	activeSessionId,
	workspace,
	onSelectSession,
	onCloseSession,
}: SessionPanelProps) {
	if (!open) {
		return null;
	}

	return (
		<section
			className="session-panel"
			aria-label="全部会话"
			id="session-panel"
			ref={panelRef}
			role="region"
			style={{
				position: "absolute",
				left: `${offset.x}px`,
				top: `${offset.y}px`,
				width: `${offset.width}px`,
			}}
		>
			<header className="session-panel-header">
				<div className="session-panel-heading">
					<strong className="session-panel-heading-title">Pi Agent 会话</strong>
					<span className="session-panel-heading-summary">
						{sessionSummaries.length === 0
							? "还没有创建会话。"
							: `共 ${sessionSummaries.length} 个，运行中 ${sessionRunningCount} 个，待处理更新 ${sessionAttentionCount} 个`}
					</span>
				</div>
			</header>
			{sessionSummaries.length === 0 ? (
				<p className="session-panel-empty">
					{agentConfigured
						? "还没有 Pi Agent session。点击左侧 + 先创建一个。"
						: "还没有 Pi Agent session。先在设置里配置 agent。"}
				</p>
			) : (
				<div className="session-panel-list" role="list">
					{sessionSummaries.map((session) => (
						<article
							className={
								session.sessionId === activeSessionId
									? "session-panel-item active"
									: "session-panel-item"
							}
							key={session.sessionId}
							role="listitem"
						>
							<button
								className="session-panel-main"
								onClick={() => onSelectSession(session.sessionId)}
								title={`${session.title} · ${formatSessionStatus(session)}`}
								type="button"
							>
								{session.sessionId === activeSessionId ? (
									<span className="session-panel-kicker">当前会话</span>
								) : null}
								<span className="session-panel-title-row">
									<span className="session-panel-title">{session.title}</span>
								</span>
								<span className="session-panel-detail-row">
									<span className={`session-panel-status ${buildSessionStatusClassName(session)}`}>
										{formatSessionStatus(session)}
									</span>
									{session.attention ? (
										<span className="session-panel-status session-panel-status-attention">
											待处理更新
										</span>
									) : null}
									<span className="session-panel-agent">{session.agentName}</span>
								</span>
								<span className="session-panel-path">
									{formatWorkspacePath(session.workspaceRoot, workspace)}
								</span>
							</button>
							<button
								className="session-panel-close"
								aria-label={`关闭会话 ${session.title}`}
								onClick={() => onCloseSession(session.sessionId)}
								title={`关闭会话 ${session.title}`}
								type="button"
							>
								<span aria-hidden="true" className="session-panel-close-icon">
									×
								</span>
							</button>
						</article>
					))}
				</div>
			)}
		</section>
	);
}
