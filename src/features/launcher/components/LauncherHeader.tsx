import type { MouseEvent as ReactMouseEvent, RefObject } from "react";
import type { AcpSessionSummary } from "../../../lib/tauri/types";
import type { WorkspaceBreadcrumb } from "../workspace";
import { buildSessionDotClassName, formatSessionStatus } from "../sessions";

interface LauncherHeaderProps {
	workspacePickerOpen: boolean;
	workspacePickerTriggerRef: RefObject<HTMLButtonElement | null>;
	workspaceBreadcrumbs: WorkspaceBreadcrumb[];
	onToggleWorkspacePicker: () => void;
	onSelectWorkspaceCrumb: (path: string) => void;
	onWorkspaceDragStart: (event: ReactMouseEvent<HTMLDivElement>) => void;
	agentConfigured: boolean;
	agentPickerOpen: boolean;
	agentPickerTriggerRef: RefObject<HTMLButtonElement | null>;
	selectedAgentName: string | null;
	onToggleAgentPicker: () => void;
	creatingSession: boolean;
	onCreateSession: () => void;
	sessionPanelOpen: boolean;
	sessionPanelTriggerRef: RefObject<HTMLButtonElement | null>;
	sessionTriggerSummary: string;
	sessionCount: number;
	visibleSessionDots: AcpSessionSummary[];
	activeSessionId: string | null;
	overflowSessionCount: number;
	onToggleSessionPanel: () => void;
	onSelectSessionDot: (sessionId: string) => void;
	onOpenSessionPanel: () => void;
}

export function LauncherHeader({
	workspacePickerOpen,
	workspacePickerTriggerRef,
	workspaceBreadcrumbs,
	onToggleWorkspacePicker,
	onSelectWorkspaceCrumb,
	onWorkspaceDragStart,
	agentConfigured,
	agentPickerOpen,
	agentPickerTriggerRef,
	selectedAgentName,
	onToggleAgentPicker,
	creatingSession,
	onCreateSession,
	sessionPanelOpen,
	sessionPanelTriggerRef,
	sessionTriggerSummary,
	sessionCount,
	visibleSessionDots,
	activeSessionId,
	overflowSessionCount,
	onToggleSessionPanel,
	onSelectSessionDot,
	onOpenSessionPanel,
}: LauncherHeaderProps) {
	return (
		<header className="workspace-row">
			<div className="workspace-bar">
				<div className="workspace-picker">
					<button
						aria-expanded={workspacePickerOpen}
						aria-haspopup="menu"
						className={
							workspacePickerOpen ? "workspace-picker-input active" : "workspace-picker-input"
						}
						onClick={onToggleWorkspacePicker}
						ref={workspacePickerTriggerRef}
						type="button"
					>
						<span className="workspace-picker-label">Workspace</span>
						<span className="workspace-picker-caret" aria-hidden="true">
							▾
						</span>
					</button>
				</div>

				<div className="workspace-breadcrumbs" role="navigation" aria-label="当前工作目录">
					{workspaceBreadcrumbs.map((crumb, index) => (
						<button
							className="workspace-crumb"
							key={`${crumb.path}-${index}`}
							onClick={() => onSelectWorkspaceCrumb(crumb.path)}
							type="button"
						>
							{crumb.label}
						</button>
					))}
				</div>

				<div
					aria-hidden="true"
					className="workspace-drag-zone"
					data-tauri-drag-region
					onMouseDown={onWorkspaceDragStart}
				/>
			</div>

			<div className="session-strip" aria-label="ACP sessions">
				{agentConfigured ? (
					<button
						aria-expanded={agentPickerOpen}
						aria-haspopup="menu"
						className={agentPickerOpen ? "agent-picker-trigger active" : "agent-picker-trigger"}
						onClick={onToggleAgentPicker}
						ref={agentPickerTriggerRef}
						type="button"
					>
						<span className="agent-picker-trigger-label">Agent</span>
						<span className="agent-picker-trigger-value">{selectedAgentName ?? "未选择"}</span>
						<span className="agent-picker-trigger-caret" aria-hidden="true">
							▾
						</span>
					</button>
				) : null}
				{agentConfigured ? (
					<button
						className="session-create-button"
						disabled={creatingSession}
						onClick={onCreateSession}
						type="button"
					>
						{creatingSession ? "..." : "+"}
					</button>
				) : null}
				<button
					aria-controls="session-panel"
					aria-expanded={sessionPanelOpen}
					aria-haspopup="dialog"
					className={sessionPanelOpen ? "session-panel-toggle active" : "session-panel-toggle"}
					onClick={onToggleSessionPanel}
					ref={sessionPanelTriggerRef}
					type="button"
				>
					<span className="session-panel-toggle-label">会话</span>
					<span className="session-panel-toggle-summary">{sessionTriggerSummary}</span>
					<span className="session-panel-toggle-count" aria-label={`${sessionCount} 个会话`}>
						{sessionCount}
					</span>
				</button>
				{visibleSessionDots.map((session) => (
					<button
						aria-label={`${session.title}，${formatSessionStatus(session)}`}
						className={buildSessionDotClassName(session, activeSessionId)}
						key={session.sessionId}
						onClick={() => onSelectSessionDot(session.sessionId)}
						title={`${session.title} · ${formatSessionStatus(session)}`}
						type="button"
					/>
				))}
				{overflowSessionCount > 0 ? (
					<button className="session-overflow-button" onClick={onOpenSessionPanel} type="button">
						+{overflowSessionCount}
					</button>
				) : null}
			</div>
		</header>
	);
}
