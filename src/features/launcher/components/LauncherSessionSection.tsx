import { memo } from "react";
import type { RefObject } from "react";

import type { AcpSessionSummary, WorkspaceState } from "../../../lib/tauri/types";
import { SessionPanel } from "./SessionPanel";

interface LauncherSessionSectionProps {
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
	workspace: WorkspaceState;
	onSelectSession: (sessionId: string) => void;
	onCloseSession: (sessionId: string) => void;
}

export const LauncherSessionSection = memo(function LauncherSessionSection({
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
}: LauncherSessionSectionProps) {
	return (
		<SessionPanel
			open={open}
			offset={offset}
			panelRef={panelRef}
			agentConfigured={agentConfigured}
			sessionSummaries={sessionSummaries}
			sessionRunningCount={sessionRunningCount}
			sessionAttentionCount={sessionAttentionCount}
			activeSessionId={activeSessionId}
			workspace={workspace}
			onSelectSession={onSelectSession}
			onCloseSession={onCloseSession}
		/>
	);
});
