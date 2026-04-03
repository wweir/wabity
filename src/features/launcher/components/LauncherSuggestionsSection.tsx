import { memo } from "react";
import type { RefObject } from "react";

import type {
	ActionMatch,
	FileSearchMatch,
	FloatingPanelOffset,
	InstalledAppMatch,
} from "../types";
import type { SuggestionMode } from "../query";
import { CompletionPopup } from "./CompletionPopup";

interface LauncherSuggestionsSectionProps {
	hasSuggestions: boolean;
	suggestionMode: SuggestionMode;
	popupId: string;
	optionIdPrefix: string;
	completionOffset: FloatingPanelOffset;
	completionListRef: RefObject<HTMLUListElement | null>;
	selectedIndex: number;
	visibleFileMatches: FileSearchMatch[];
	visibleActionMatches: ActionMatch[];
	visibleAppMatches: InstalledAppMatch[];
	onSelectIndex: (index: number) => void;
	onSelectFile: (index: number) => void;
	onRunSelectedAction: (index: number) => void;
	onRunSelectedApp: (index: number) => void;
}

export const LauncherSuggestionsSection = memo(function LauncherSuggestionsSection({
	hasSuggestions,
	suggestionMode,
	popupId,
	optionIdPrefix,
	completionOffset,
	completionListRef,
	selectedIndex,
	visibleFileMatches,
	visibleActionMatches,
	visibleAppMatches,
	onSelectIndex,
	onSelectFile,
	onRunSelectedAction,
	onRunSelectedApp,
}: LauncherSuggestionsSectionProps) {
	return (
		<CompletionPopup
			hasSuggestions={hasSuggestions}
			suggestionMode={suggestionMode}
			popupId={popupId}
			optionIdPrefix={optionIdPrefix}
			completionOffset={completionOffset}
			completionListRef={completionListRef}
			selectedIndex={selectedIndex}
			visibleFileMatches={visibleFileMatches}
			visibleActionMatches={visibleActionMatches}
			visibleAppMatches={visibleAppMatches}
			onSelectIndex={onSelectIndex}
			onSelectFile={onSelectFile}
			onRunSelectedAction={onRunSelectedAction}
			onRunSelectedApp={onRunSelectedApp}
		/>
	);
});
