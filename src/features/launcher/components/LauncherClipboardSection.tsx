import { memo } from "react";
import type { RefObject } from "react";

import type {
	ClipboardHistoryEntry,
	ClipboardHistorySelectionMode,
} from "../../../lib/tauri/types";
import { ClipboardHistoryPanel } from "./ClipboardHistoryPanel";

interface LauncherClipboardSectionProps {
	open: boolean;
	panelRef: RefObject<HTMLElement | null>;
	selectionMode: ClipboardHistorySelectionMode;
	pinnedEntries: ClipboardHistoryEntry[];
	recentEntries: ClipboardHistoryEntry[];
	selectedEntryId: string | null;
	onSelectEntry: (entryId: string | null) => void;
	onPasteEntry: (entryId: string) => void;
	onTogglePin: (entryId: string) => void;
	onDeleteEntry: (entryId: string) => void;
}

export const LauncherClipboardSection = memo(function LauncherClipboardSection({
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
}: LauncherClipboardSectionProps) {
	return (
		<ClipboardHistoryPanel
			open={open}
			panelRef={panelRef}
			selectionMode={selectionMode}
			pinnedEntries={pinnedEntries}
			recentEntries={recentEntries}
			selectedEntryId={selectedEntryId}
			onSelectEntry={onSelectEntry}
			onPasteEntry={onPasteEntry}
			onTogglePin={onTogglePin}
			onDeleteEntry={onDeleteEntry}
		/>
	);
});
