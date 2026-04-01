import type { MouseEvent as ReactMouseEvent, RefObject } from "react";
import type {
	ClipboardHistoryEntry,
	ClipboardHistorySelectionMode,
} from "../../../lib/tauri/types";

interface ClipboardHistoryPanelProps {
	open: boolean;
	panelRef: RefObject<HTMLElement | null>;
	selectionMode: ClipboardHistorySelectionMode;
	pinnedEntries: ClipboardHistoryEntry[];
	recentEntries: ClipboardHistoryEntry[];
	selectedEntryId: string | null;
	onSelectEntry: (entryId: string) => void;
	onPasteEntry: (entryId: string) => void;
	onTogglePin: (entryId: string) => void;
	onDeleteEntry: (entryId: string) => void;
}

function preventFocusSteal(event: ReactMouseEvent<HTMLElement>) {
	event.preventDefault();
}

function ClipboardHistorySection({
	title,
	emptyText,
	entries,
	selectedEntryId,
	onSelectEntry,
	onPasteEntry,
	onTogglePin,
	onDeleteEntry,
}: {
	title: string;
	emptyText: string;
	entries: ClipboardHistoryEntry[];
	selectedEntryId: string | null;
	onSelectEntry: (entryId: string) => void;
	onPasteEntry: (entryId: string) => void;
	onTogglePin: (entryId: string) => void;
	onDeleteEntry: (entryId: string) => void;
}) {
	return (
		<section className="clipboard-history-panel-section">
			<header className="clipboard-history-panel-section-header">
				<strong className="clipboard-history-panel-section-title">{title}</strong>
				<span className="clipboard-history-panel-section-count">{entries.length}</span>
			</header>
			{entries.length === 0 ? (
				<p className="clipboard-history-panel-empty">{emptyText}</p>
			) : (
				<div className="clipboard-history-panel-list" role="list">
					{entries.map((entry) => {
						const selected = entry.id === selectedEntryId;
						return (
							<div
								key={entry.id}
								className={
									selected ? "clipboard-history-panel-item active" : "clipboard-history-panel-item"
								}
								role="listitem"
							>
								<button
									className="clipboard-history-panel-item-main"
									onMouseDown={preventFocusSteal}
									onMouseEnter={() => onSelectEntry(entry.id)}
									onClick={() => onPasteEntry(entry.id)}
									type="button"
								>
									<span className="clipboard-history-panel-item-text">{entry.text}</span>
								</button>
								<span className="clipboard-history-panel-item-actions">
									<button
										className="clipboard-history-panel-item-action"
										onMouseDown={preventFocusSteal}
										onClick={(event) => {
											event.stopPropagation();
											onTogglePin(entry.id);
										}}
										type="button"
									>
										{entry.pinned ? "取消固定" : "固定"}
									</button>
									<button
										className="clipboard-history-panel-item-action danger"
										onMouseDown={preventFocusSteal}
										onClick={(event) => {
											event.stopPropagation();
											onDeleteEntry(entry.id);
										}}
										type="button"
									>
										删除
									</button>
								</span>
							</div>
						);
					})}
				</div>
			)}
		</section>
	);
}

export function ClipboardHistoryPanel({
	open,
	panelRef,
	selectionMode,
	pinnedEntries,
	recentEntries,
	selectedEntryId,
	onSelectEntry,
	onPasteEntry,
	onTogglePin,
	onDeleteEntry,
}: ClipboardHistoryPanelProps) {
	if (!open) {
		return null;
	}

	return (
		<section
			aria-label="历史剪贴板"
			className="clipboard-history-panel clipboard-history-panel-standalone"
			id="clipboard-history-panel"
			ref={panelRef}
			role="dialog"
		>
			<header className="clipboard-history-panel-header">
				<div className="clipboard-history-panel-heading">
					<strong className="clipboard-history-panel-heading-title">历史剪贴板</strong>
					<span className="clipboard-history-panel-heading-summary">
						{selectionMode === "insert_into_launcher"
							? "Enter 插入输入框 · Cmd/Ctrl+P 切换 Pin"
							: "Enter 回贴外部应用 · Cmd/Ctrl+P 切换 Pin"}
					</span>
				</div>
			</header>
			<ClipboardHistorySection
				title="常用"
				emptyText="还没有固定项"
				entries={pinnedEntries}
				selectedEntryId={selectedEntryId}
				onSelectEntry={onSelectEntry}
				onPasteEntry={onPasteEntry}
				onTogglePin={onTogglePin}
				onDeleteEntry={onDeleteEntry}
			/>
			<ClipboardHistorySection
				title="最近"
				emptyText="还没有最近文本"
				entries={recentEntries}
				selectedEntryId={selectedEntryId}
				onSelectEntry={onSelectEntry}
				onPasteEntry={onPasteEntry}
				onTogglePin={onTogglePin}
				onDeleteEntry={onDeleteEntry}
			/>
		</section>
	);
}
