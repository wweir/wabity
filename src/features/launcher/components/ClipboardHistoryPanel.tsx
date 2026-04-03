import type { MouseEvent as ReactMouseEvent, RefObject } from "react";
import type {
	ClipboardHistoryEntry,
	ClipboardHistorySelectionMode,
} from "../../../lib/tauri/types";

function getPinnedShortcutLabel(index: number) {
	const hotkey = String.fromCharCode("A".charCodeAt(0) + index);
	return `Alt+${hotkey}`;
}

function getRecentShortcutLabel(index: number) {
	const hotkey = index === 9 ? "0" : String(index + 1);
	return `Alt+${hotkey}`;
}

function PinIcon({ pinned }: { pinned: boolean }) {
	return (
		<svg
			aria-hidden="true"
			className="clipboard-history-panel-item-action-icon"
			fill={pinned ? "currentColor" : "none"}
			stroke="currentColor"
			strokeLinecap="round"
			strokeLinejoin="round"
			strokeWidth="1.35"
			viewBox="0 0 16 16"
		>
			<path d="M5.25 2.75h5.5l-1.2 3.15 2 1.9H4.45l2-1.9-1.2-3.15Z" />
			<path d="M8 8.15v5.1" />
			<path d="M6.6 13.25 8 14.75l1.4-1.5" />
		</svg>
	);
}

function DeleteIcon() {
	return (
		<svg
			aria-hidden="true"
			className="clipboard-history-panel-item-action-icon"
			fill="none"
			stroke="currentColor"
			strokeLinecap="round"
			strokeLinejoin="round"
			strokeWidth="1.35"
			viewBox="0 0 16 16"
		>
			<path d="M3.75 4.5h8.5" />
			<path d="M6.25 2.75h3.5" />
			<path d="M5.1 4.5v7.1a1 1 0 0 0 1 1h3.8a1 1 0 0 0 1-1V4.5" />
			<path d="M6.8 6.6v3.7" />
			<path d="M9.2 6.6v3.7" />
		</svg>
	);
}

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
	getShortcutLabel,
	selectedEntryId,
	onSelectEntry,
	onPasteEntry,
	onTogglePin,
	onDeleteEntry,
}: {
	title: string;
	emptyText: string;
	entries: ClipboardHistoryEntry[];
	getShortcutLabel: (index: number) => string;
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
					{entries.map((entry, index) => {
						const selected = entry.id === selectedEntryId;
						const shortcutLabel = getShortcutLabel(index);
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
									<span className="clipboard-history-panel-item-hotkey">{shortcutLabel}</span>
									<span className="clipboard-history-panel-item-text">{entry.text}</span>
								</button>
								<span className="clipboard-history-panel-item-actions">
									<button
										aria-label={entry.pinned ? "取消固定" : "固定"}
										className={`clipboard-history-panel-item-action${
											entry.pinned ? " active" : ""
										}`}
										onMouseDown={preventFocusSteal}
										onClick={(event) => {
											event.stopPropagation();
											onTogglePin(entry.id);
										}}
										title={entry.pinned ? "取消固定" : "固定"}
										type="button"
									>
										<PinIcon pinned={entry.pinned} />
									</button>
									<button
										aria-label="删除"
										className="clipboard-history-panel-item-action danger"
										onMouseDown={preventFocusSteal}
										onClick={(event) => {
											event.stopPropagation();
											onDeleteEntry(entry.id);
										}}
										title="删除"
										type="button"
									>
										<DeleteIcon />
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
							? "Enter 插入输入框 · Alt+A-E / 1-0 直贴"
							: "Enter 回贴外部应用 · Alt+A-E / 1-0 直贴"}
					</span>
				</div>
			</header>
			<ClipboardHistorySection
				title="常用"
				emptyText="还没有固定项"
				entries={pinnedEntries}
				getShortcutLabel={getPinnedShortcutLabel}
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
				getShortcutLabel={getRecentShortcutLabel}
				selectedEntryId={selectedEntryId}
				onSelectEntry={onSelectEntry}
				onPasteEntry={onPasteEntry}
				onTogglePin={onTogglePin}
				onDeleteEntry={onDeleteEntry}
			/>
		</section>
	);
}
