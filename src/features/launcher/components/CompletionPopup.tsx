import type { ReactElement, RefObject } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import type {
	ActionMatch,
	FileSearchMatch,
	FloatingPanelOffset,
	InstalledAppMatch,
	RunningProcessMatch,
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
	visibleKillMatches: RunningProcessMatch[];
	onSelectIndex: (index: number) => void;
	onSelectFile: (index: number) => void;
	onSelectKill: (index: number) => void;
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
	visibleKillMatches,
	onSelectIndex,
	onSelectFile,
	onSelectKill,
	onRunSelectedAction,
	onRunSelectedApp,
}: CompletionPopupProps): ReactElement | null {
	function buildOptionId(index: number): string {
		return `${optionIdPrefix}-${suggestionMode}-${index}`;
	}

	function preventFocusSteal(event: ReactMouseEvent<HTMLLIElement>): void {
		event.preventDefault();
	}

	function getAriaLabel(): string {
		switch (suggestionMode) {
			case "file":
				return "文件补全候选";
			case "action":
				return "动作补全候选";
			case "kill":
				return "进程补全候选";
			case "app":
				return "应用候选";
			default:
				return "补全候选";
		}
	}

	function getSuggestionItemClassName(index: number): string {
		return index === selectedIndex ? "suggestion-item selected" : "suggestion-item";
	}

	function renderFileSuggestions(): ReactElement[] {
		return visibleFileMatches.map((match, index) => (
			<li
				key={match.path}
				aria-selected={index === selectedIndex}
				className={getSuggestionItemClassName(index)}
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
		));
	}

	function renderActionSuggestions(): ReactElement[] {
		return visibleActionMatches.map((match, index) => (
			<li
				key={match.descriptor.id}
				aria-selected={index === selectedIndex}
				className={getSuggestionItemClassName(index)}
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
		));
	}

	function renderKillSuggestions(): ReactElement[] {
		return visibleKillMatches.map((match, index) => (
			<li
				key={`${match.pid}:${match.processName}`}
				aria-selected={index === selectedIndex}
				className={getSuggestionItemClassName(index)}
				data-suggestion-index={index}
				id={buildOptionId(index)}
				onMouseDown={preventFocusSteal}
				onClick={() => onSelectKill(index)}
				onMouseEnter={() => onSelectIndex(index)}
				role="option"
			>
				<div className="suggestion-line">
					<strong>
						{match.displayName}
						{match.kind === "app" ? " · App" : " · Process"}
					</strong>
					<span className="suggestion-meta">
						pid:{match.pid} · {match.processName}
					</span>
				</div>
			</li>
		));
	}

	function renderAppSuggestions(): ReactElement[] {
		return visibleAppMatches.map((match, index) => (
			<li
				key={match.path}
				aria-selected={index === selectedIndex}
				className={getSuggestionItemClassName(index)}
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
		));
	}

	function renderSuggestions(): ReactElement[] {
		switch (suggestionMode) {
			case "file":
				return renderFileSuggestions();
			case "action":
				return renderActionSuggestions();
			case "kill":
				return renderKillSuggestions();
			case "app":
				return renderAppSuggestions();
			default:
				return [];
		}
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
				aria-label={getAriaLabel()}
				className="suggestion-list completion-list"
				id={popupId}
				ref={completionListRef}
				role="listbox"
			>
				{renderSuggestions()}
			</ul>
		</div>
	);
}
