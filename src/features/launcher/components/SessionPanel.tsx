import type { RefObject } from "react";
import type { AcpSessionSummary, WorkspaceState } from "../../../lib/tauri/types";
import { formatSessionStatus } from "../sessions";
import { formatWorkspacePath } from "../workspace";

interface SessionPanelProps {
	open: boolean;
	offset: {
		x: number;
		y: number;
		width: number;
	};
	panelRef: RefObject<HTMLElement | null>;
	agentConfigured: boolean;
	sessionSummaries: AcpSessionSummary[];
	sessionRunningCount: number;
	sessionAttentionCount: number;
	activeSessionId: string | null;
	activeSessionSummary: AcpSessionSummary | null;
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
	activeSessionSummary,
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
			style={{
				position: "absolute",
				left: `${offset.x}px`,
				top: `${offset.y}px`,
				width: `${offset.width}px`,
			}}
		>
			<header className="session-panel-header">
				<div className="session-panel-heading">
					<strong className="session-panel-heading-title">ACP 会话</strong>
					<span className="session-panel-heading-summary">
						{sessionSummaries.length === 0
							? "还没有创建会话。"
							: `共 ${sessionSummaries.length} 个，运行中 ${sessionRunningCount} 个，待处理更新 ${sessionAttentionCount} 个`}
					</span>
				</div>
				{activeSessionSummary ? (
					<span className="session-panel-active-pill">当前：{activeSessionSummary.title}</span>
				) : null}
			</header>
			{sessionSummaries.length === 0 ? (
				<p className="session-panel-empty">
					{agentConfigured
						? "还没有 ACP session。点击左侧 + 先创建一个。"
						: "还没有 ACP session。先在设置里配置 agent。"}
				</p>
			) : (
				<div className="session-panel-list">
					{sessionSummaries.map((session) => (
						<article
							className={
								session.sessionId === activeSessionId
									? "session-panel-item active"
									: "session-panel-item"
							}
							key={session.sessionId}
						>
							<button
								className="session-panel-main"
								onClick={() => onSelectSession(session.sessionId)}
								type="button"
							>
								<span className="session-panel-title-row">
									<span className="session-panel-title">{session.title}</span>
									{session.sessionId === activeSessionId ? (
										<span className="session-panel-badge">当前</span>
									) : null}
								</span>
								<span className="session-panel-meta-row">
									<span className="session-panel-status">{formatSessionStatus(session)}</span>
									<span className="session-panel-meta">{session.agentName}</span>
									<span className="session-panel-meta">
										{formatWorkspacePath(session.workspaceRoot, workspace)}
									</span>
								</span>
							</button>
							<button
								className="session-panel-close"
								onClick={() => onCloseSession(session.sessionId)}
								type="button"
							>
								关闭
							</button>
						</article>
					))}
				</div>
			)}
		</section>
	);
}
