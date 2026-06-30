import type { RefObject } from "react";
import type { AcpAgentConfig } from "../../../lib/tauri/types";
import type { FloatingPanelOffset } from "../types";

interface AgentPickerPanelProps {
	open: boolean;
	offset: FloatingPanelOffset;
	panelRef: RefObject<HTMLDivElement | null>;
	agents: AcpAgentConfig[];
	selectedAgentId: string | null;
	onSelectAgent: (agent: AcpAgentConfig) => void;
}

export function AgentPickerPanel({
	open,
	offset,
	panelRef,
	agents,
	selectedAgentId,
	onSelectAgent,
}: AgentPickerPanelProps) {
	if (!open) {
		return null;
	}

	return (
		<div
			className="agent-picker-panel"
			id="agent-picker-panel"
			ref={panelRef}
			aria-label="选择 Agent"
			style={{
				position: "absolute",
				left: `${offset.x}px`,
				top: `${offset.y}px`,
				width: `${offset.width}px`,
			}}
		>
			{agents.map((agent) => (
				<button
					className={
						selectedAgentId === agent.id ? "agent-picker-item active" : "agent-picker-item"
					}
					key={agent.id}
					onClick={() => onSelectAgent(agent)}
					type="button"
				>
					<span className="agent-picker-item-name">{agent.name}</span>
					<span className="agent-picker-item-meta">
						{agent.shellCommand?.trim() || agent.program}
					</span>
				</button>
			))}
		</div>
	);
}
