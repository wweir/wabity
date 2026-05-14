import { useEffect, useLayoutEffect, type RefObject } from "react";

import { clamp } from "./layout";

interface LauncherSelectionEffectsInput {
	completionListRef: RefObject<HTMLUListElement | null>;
	flattenedClipboardEntries: Array<{ id: string }>;
	hasSuggestions: boolean;
	selectedClipboardEntryId: string | null;
	selectedIndex: number;
	setSelectedClipboardEntryId: (entryId: string | null) => void;
	setSelectedIndex: (index: number) => void;
	setSuggestionsHidden: (hidden: boolean) => void;
	suggestionCount: number;
	suggestionMode: string;
	textBeforeCaret: string;
	visibleSuggestionKeys: readonly unknown[];
}

export function useLauncherSelectionEffects({
	completionListRef,
	flattenedClipboardEntries,
	hasSuggestions,
	selectedClipboardEntryId,
	selectedIndex,
	setSelectedClipboardEntryId,
	setSelectedIndex,
	setSuggestionsHidden,
	suggestionCount,
	suggestionMode,
	textBeforeCaret,
	visibleSuggestionKeys,
}: LauncherSelectionEffectsInput): void {
	useEffect(() => {
		setSuggestionsHidden(false);
		setSelectedIndex(0);
	}, [setSelectedIndex, setSuggestionsHidden, suggestionMode, textBeforeCaret]);

	useEffect(() => {
		if (flattenedClipboardEntries.length === 0) {
			if (selectedClipboardEntryId !== null) {
				setSelectedClipboardEntryId(null);
			}
			return;
		}

		if (
			selectedClipboardEntryId &&
			flattenedClipboardEntries.some((entry) => entry.id === selectedClipboardEntryId)
		) {
			return;
		}

		setSelectedClipboardEntryId(flattenedClipboardEntries[0]?.id ?? null);
	}, [flattenedClipboardEntries, selectedClipboardEntryId, setSelectedClipboardEntryId]);

	useEffect(() => {
		if (suggestionCount === 0) {
			if (selectedIndex !== 0) {
				setSelectedIndex(0);
			}
			return;
		}

		if (selectedIndex >= suggestionCount) {
			setSelectedIndex(suggestionCount - 1);
		}
	}, [selectedIndex, setSelectedIndex, suggestionCount]);

	useLayoutEffect(() => {
		if (!hasSuggestions) {
			return;
		}

		const listElement = completionListRef.current;
		if (!listElement) {
			return;
		}

		const selectedElement = listElement.querySelector<HTMLElement>(
			`[data-suggestion-index="${selectedIndex}"]`,
		);
		if (!selectedElement) {
			return;
		}

		const nextScrollTop =
			selectedElement.offsetTop - listElement.clientHeight / 2 + selectedElement.offsetHeight / 2;
		const maxScrollTop = Math.max(0, listElement.scrollHeight - listElement.clientHeight);
		listElement.scrollTop = clamp(nextScrollTop, 0, maxScrollTop);
	}, [completionListRef, hasSuggestions, selectedIndex, visibleSuggestionKeys]);
}
