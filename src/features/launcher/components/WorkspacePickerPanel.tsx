import type { RefObject } from "react";
import type { WorkspaceState } from "../../../lib/tauri/types";
import { formatWorkspacePath } from "../workspace";

interface WorkspacePickerPanelProps {
	open: boolean;
	offset: {
		x: number;
		y: number;
		width: number;
	};
	panelRef: RefObject<HTMLDivElement | null>;
	recentWorkspaceRoots: string[];
	workspace: WorkspaceState;
	onPickWorkspace: () => void;
	onSelectRecentWorkspace: (path: string) => void;
}

export function WorkspacePickerPanel({
	open,
	offset,
	panelRef,
	recentWorkspaceRoots,
	workspace,
	onPickWorkspace,
	onSelectRecentWorkspace,
}: WorkspacePickerPanelProps) {
	if (!open) {
		return null;
	}

	return (
		<div
			className="workspace-picker-panel"
			ref={panelRef}
			role="menu"
			aria-label="选择 workspace"
			style={{
				position: "absolute",
				left: `${offset.x}px`,
				top: `${offset.y}px`,
				width: `${offset.width}px`,
			}}
		>
			<button className="workspace-picker-action" onClick={onPickWorkspace} type="button">
				选择新的工作目录
			</button>

			<div className="workspace-picker-section">
				<span className="workspace-picker-section-title">最近目录</span>
				{recentWorkspaceRoots.length > 0 ? (
					recentWorkspaceRoots.map((path) => (
						<button
							className="workspace-picker-item"
							key={path}
							onClick={() => onSelectRecentWorkspace(path)}
							type="button"
						>
							{formatWorkspacePath(path, workspace)}
						</button>
					))
				) : (
					<span className="workspace-picker-empty">还没有最近目录</span>
				)}
			</div>
		</div>
	);
}
