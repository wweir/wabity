import type { RefObject } from "react";
import type { AcpAgentConfig } from "../../../lib/tauri/types";

interface AgentPickerPanelProps {
	open: boolean;
	offset: {
		x: number;
		y: number;
		width: number;
	};
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
			ref={panelRef}
			role="menu"
			aria-label="选择 ACP agent"
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
