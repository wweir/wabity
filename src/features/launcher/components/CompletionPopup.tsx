import type { RefObject } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import type {
	ActionMatch,
	FileSearchMatch,
	FloatingPanelOffset,
	InstalledAppMatch,
} from "../types";
import { primaryActionLabel, type SuggestionMode } from "../query";

interface CompletionPopupProps {
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

export function CompletionPopup({
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
}: CompletionPopupProps) {
	function buildOptionId(index: number) {
		return `${optionIdPrefix}-${suggestionMode}-${index}`;
	}

	function preventFocusSteal(event: ReactMouseEvent<HTMLLIElement>) {
		event.preventDefault();
	}

	if (!hasSuggestions) {
		return null;
	}

	return (
		<div
			className="completion-popup"
			style={{
				position: "absolute",
				left: `${completionOffset.x}px`,
				top: `${completionOffset.y}px`,
				width: `${completionOffset.width}px`,
			}}
		>
			<ul
				aria-label={
					suggestionMode === "file"
						? "文件补全候选"
						: suggestionMode === "action"
							? "动作补全候选"
							: "应用候选"
				}
				className="suggestion-list completion-list"
				id={popupId}
				ref={completionListRef}
				role="listbox"
			>
				{suggestionMode === "file"
					? visibleFileMatches.map((match, index) => (
							<li
								key={match.path}
								aria-selected={index === selectedIndex}
								className={index === selectedIndex ? "suggestion-item selected" : "suggestion-item"}
								data-suggestion-index={index}
								id={buildOptionId(index)}
								onMouseDown={preventFocusSteal}
								onClick={() => onSelectFile(index)}
								onMouseEnter={() => onSelectIndex(index)}
								role="option"
							>
								<div className="suggestion-line">
									<strong>{match.fileName}</strong>
									<span className="suggestion-meta">{match.parent}</span>
								</div>
							</li>
						))
					: suggestionMode === "action"
						? visibleActionMatches.map((match, index) => (
								<li
									key={match.descriptor.id}
									aria-selected={index === selectedIndex}
									className={
										index === selectedIndex ? "suggestion-item selected" : "suggestion-item"
									}
									data-suggestion-index={index}
									id={buildOptionId(index)}
									onMouseDown={preventFocusSteal}
									onClick={() => onRunSelectedAction(index)}
									onMouseEnter={() => onSelectIndex(index)}
									role="option"
								>
									<div className="suggestion-line">
										<strong>{primaryActionLabel(match)}</strong>
										<span className="suggestion-meta">{match.descriptor.summary}</span>
									</div>
								</li>
							))
						: visibleAppMatches.map((match, index) => (
								<li
									key={match.path}
									aria-selected={index === selectedIndex}
									className={
										index === selectedIndex ? "suggestion-item selected" : "suggestion-item"
									}
									data-suggestion-index={index}
									id={buildOptionId(index)}
									onMouseDown={preventFocusSteal}
									onClick={() => onRunSelectedApp(index)}
									onMouseEnter={() => onSelectIndex(index)}
									role="option"
								>
									<div className="suggestion-line">
										<strong>{match.name}</strong>
										<span className="suggestion-meta">{match.path}</span>
									</div>
								</li>
							))}
			</ul>
		</div>
	);
}
