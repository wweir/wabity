import {
	useLayoutEffect,
	useRef,
	useState,
	type CSSProperties,
	type ChangeEvent,
	type KeyboardEvent,
	type MouseEvent as ReactMouseEvent,
	type RefObject,
	type SyntheticEvent,
} from "react";
import type { InputMode } from "../types";

interface LauncherComposerProps {
	inputMode: InputMode;
	inputRef: RefObject<HTMLInputElement | HTMLTextAreaElement | null>;
	rawText: string;
	inputPlaceholder: string;
	inputLabel: string;
	inputDescriptionId?: string;
	statusLabel: string;
	statusItems: string[];
	statusTone: "default" | "progress" | "error";
	onUpdateRawText: (value: string, caretIndex: number) => void;
	onSyncCaretIndex: (element: HTMLInputElement | HTMLTextAreaElement) => void;
	onKeyDown: (event: KeyboardEvent<HTMLInputElement> | KeyboardEvent<HTMLTextAreaElement>) => void;
	onOpenSettings?: () => void;
	hasCompletion: boolean;
	hasSuggestions: boolean;
	completionPopupId?: string;
	activeCompletionOptionId?: string;
	onAcceptCompletion: () => void;
	creatingSession: boolean;
	agentActionPending: boolean;
	showAgentAction: boolean;
	showTranslateAction: boolean;
	primaryActionShortcutLabel: string;
	primaryActionLabel: string;
	primaryActionTone: "qa" | "execute" | "send" | "path" | "translate";
	canRunPrimaryAction: boolean;
	canRunTranslateAction: boolean;
	agentActionShortcutLabel: string;
	onAgentExecute: () => void;
	onRunTranslateAction: () => void;
	showCancelActiveSession: boolean;
	onCancelActiveSession: () => void;
	onRunPrimaryAction: () => void;
}

function LauncherStatusBar({
	id,
	statusLabel,
	statusItems,
	statusTone,
}: {
	id?: string;
	statusLabel: string;
	statusItems: string[];
	statusTone: "default" | "progress" | "error";
}) {
	const viewportRef = useRef<HTMLDivElement | null>(null);
	const measureRef = useRef<HTMLSpanElement | null>(null);
	const [shouldScroll, setShouldScroll] = useState(false);
	const [scrollDistance, setScrollDistance] = useState(0);
	const [scrollDurationSeconds, setScrollDurationSeconds] = useState(0);
	const normalizedStatusItems = statusItems
		.map((item) => item.trim())
		.filter((item) => item.length > 0);
	const hasMultipleItems = normalizedStatusItems.length > 1;
	const normalizedStatusText = normalizedStatusItems[0] ?? "";

	useLayoutEffect(() => {
		const viewportElement = viewportRef.current;
		const measureElement = measureRef.current;
		if (!viewportElement || !measureElement || !normalizedStatusText || hasMultipleItems) {
			setShouldScroll(false);
			setScrollDistance(0);
			setScrollDurationSeconds(0);
			return;
		}

		const updateScrollState = () => {
			const overflow = measureElement.scrollWidth - viewportElement.clientWidth;
			if (overflow <= 0) {
				setShouldScroll(false);
				setScrollDistance(0);
				setScrollDurationSeconds(0);
				return;
			}

			const gap = 32;
			const nextDistance = measureElement.scrollWidth + gap;
			setShouldScroll(true);
			setScrollDistance(nextDistance);
			setScrollDurationSeconds(Math.max(10, nextDistance / 28));
		};

		updateScrollState();

		if (typeof ResizeObserver === "undefined") {
			return;
		}

		const observer = new ResizeObserver(() => {
			updateScrollState();
		});
		observer.observe(viewportElement);
		observer.observe(measureElement);
		return () => {
			observer.disconnect();
		};
	}, [hasMultipleItems, normalizedStatusText]);

	return (
		<div
			id={id}
			aria-atomic="true"
			aria-live="polite"
			className={`launcher-status-bar tone-${statusTone}${hasMultipleItems ? " has-multiple" : ""}`}
			role="status"
		>
			<span className="launcher-status-bar-label">{statusLabel}</span>
			<div className="launcher-status-bar-viewport" ref={viewportRef}>
				{normalizedStatusItems.length > 0 ? (
					hasMultipleItems ? (
						<div className="launcher-status-bar-list">
							{normalizedStatusItems.map((item, index) => (
								<span key={`${index}:${item}`} className="launcher-status-bar-text">
									{item}
								</span>
							))}
						</div>
					) : shouldScroll ? (
						<div
							aria-label={normalizedStatusText}
							className="launcher-status-bar-marquee"
							style={
								{
									"--status-scroll-distance": `${scrollDistance}px`,
									"--status-scroll-duration": `${scrollDurationSeconds}s`,
								} as CSSProperties
							}
						>
							<span className="launcher-status-bar-text">{normalizedStatusText}</span>
							<span aria-hidden="true" className="launcher-status-bar-separator">
								·
							</span>
							<span aria-hidden="true" className="launcher-status-bar-text">
								{normalizedStatusText}
							</span>
						</div>
					) : (
						<span className="launcher-status-bar-text">{normalizedStatusText}</span>
					)
				) : (
					<span className="launcher-status-bar-text empty">暂无最近输入</span>
				)}
				{hasMultipleItems ? null : (
					<span className="launcher-status-bar-measure" ref={measureRef}>
						{normalizedStatusText}
					</span>
				)}
			</div>
		</div>
	);
}

export function LauncherComposer({
	inputMode,
	inputRef,
	rawText,
	inputPlaceholder,
	inputLabel,
	inputDescriptionId,
	statusLabel,
	statusItems,
	statusTone,
	onUpdateRawText,
	onSyncCaretIndex,
	onKeyDown,
	onOpenSettings,
	hasCompletion,
	hasSuggestions,
	completionPopupId,
	activeCompletionOptionId,
	onAcceptCompletion,
	creatingSession,
	agentActionPending,
	showAgentAction,
	showTranslateAction,
	primaryActionShortcutLabel,
	primaryActionLabel,
	primaryActionTone,
	canRunPrimaryAction,
	canRunTranslateAction,
	agentActionShortcutLabel,
	onAgentExecute,
	onRunTranslateAction,
	showCancelActiveSession,
	onCancelActiveSession,
	onRunPrimaryAction,
}: LauncherComposerProps) {
	const inputId = "launcher-primary-input";
	const inputProps = {
		autoFocus: true,
		autoCapitalize: "none" as const,
		autoComplete: "off",
		autoCorrect: "off" as const,
		placeholder: inputPlaceholder,
		spellCheck: false,
		value: rawText,
		onChange: (event: ChangeEvent<HTMLInputElement> | ChangeEvent<HTMLTextAreaElement>) =>
			onUpdateRawText(event.target.value, event.target.selectionEnd ?? event.target.value.length),
		onClick: (event: ReactMouseEvent<HTMLInputElement> | ReactMouseEvent<HTMLTextAreaElement>) =>
			onSyncCaretIndex(event.currentTarget),
		onKeyDown,
		onKeyUp: (event: KeyboardEvent<HTMLInputElement> | KeyboardEvent<HTMLTextAreaElement>) =>
			onSyncCaretIndex(event.currentTarget),
		onSelect: (event: SyntheticEvent<HTMLInputElement> | SyntheticEvent<HTMLTextAreaElement>) =>
			onSyncCaretIndex(event.currentTarget),
	};
	const sharedAccessibilityProps = {
		"aria-label": inputLabel,
		"aria-describedby": inputDescriptionId,
		"aria-controls": completionPopupId,
		"aria-expanded": hasSuggestions,
		"aria-activedescendant": activeCompletionOptionId,
		"aria-autocomplete": "list" as const,
	};

	return (
		<>
			<label className="sr-only" htmlFor={inputId}>
				{inputLabel}
			</label>
			{inputMode === "inline" ? (
				<input
					{...inputProps}
					{...sharedAccessibilityProps}
					className="launcher-input inline"
					id={inputId}
					role="combobox"
					ref={(element) => {
						inputRef.current = element;
					}}
					type="text"
				/>
			) : (
				<textarea
					{...inputProps}
					{...sharedAccessibilityProps}
					className="launcher-input multiline"
					id={inputId}
					ref={(element) => {
						inputRef.current = element;
					}}
					rows={4}
				/>
			)}

			<div className="control-row">
				<LauncherStatusBar
					id={inputDescriptionId}
					statusLabel={statusLabel}
					statusItems={statusItems}
					statusTone={statusTone}
				/>
				<div className="action-buttons">
					{onOpenSettings ? (
						<button
							aria-label="打开设置"
							className="control-button settings-button"
							onClick={onOpenSettings}
							title="设置"
							type="button"
						>
							<svg
								aria-label="设置"
								className="control-button-icon settings-button-icon"
								fill="none"
								role="img"
								stroke="currentColor"
								strokeLinecap="round"
								strokeLinejoin="round"
								strokeWidth="1.4"
								viewBox="0 0 16 16"
							>
								<path d="M4 2.25v2.35" />
								<path d="M4 7.65v6.1" />
								<path d="M8 2.25v5.45" />
								<path d="M8 10.8v2.95" />
								<path d="M12 2.25v1.55" />
								<path d="M12 6.9v6.85" />
								<circle cx="4" cy="6.1" r="1.5" />
								<circle cx="8" cy="9.25" r="1.5" />
								<circle cx="12" cy="5.35" r="1.5" />
							</svg>
						</button>
					) : null}
					{showTranslateAction ? (
						<button
							className="control-button translate-button"
							disabled={!canRunTranslateAction}
							onClick={onRunTranslateAction}
							title="翻译"
							type="button"
						>
							<svg
								aria-label="翻译"
								className="control-button-icon"
								fill="currentColor"
								role="img"
								viewBox="0 0 16 16"
							>
								<path d="M3 2a1 1 0 00-1 1v7a1 1 0 001 1h4v2H5v1h5v-1H8v-2h4a1 1 0 001-1V3a1 1 0 00-1-1H3zm1.5 2h1l1.5 4h-1l-.3-1H4.3L4 8H3l1.5-4zm.5 1l-.4 1.3h.8L5 5zm4.5-1h3v1h-2v1h2v1h-2v1h2v1h-3V4z" />
							</svg>
							<span>翻译</span>
						</button>
					) : null}
					{showAgentAction ? (
						<button
							className="control-button agent-execute-button"
							disabled={agentActionPending || creatingSession || rawText.trim().length === 0}
							onClick={onAgentExecute}
							title={`Agent 执行 (${agentActionShortcutLabel})`}
							type="button"
						>
							<svg
								aria-label="Agent"
								className="control-button-icon"
								fill="currentColor"
								role="img"
								viewBox="0 0 16 16"
							>
								<path d="M8 1a1 1 0 011 1v1h3a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h3V2a1 1 0 011-1z" />
								<circle cx="5.5" cy="7.5" r="1" fill="var(--button-background, #fff)" />
								<circle cx="10.5" cy="7.5" r="1" fill="var(--button-background, #fff)" />
								<path d="M6 10h4v1H6z" fill="var(--button-background, #fff)" />
							</svg>
							<span>{agentActionPending ? "执行中..." : "Agent"}</span>
							{agentActionPending ? null : (
								<span className="control-button-shortcut">{agentActionShortcutLabel}</span>
							)}
						</button>
					) : null}
					{showCancelActiveSession ? (
						<button
							className="control-button cancel-button"
							onClick={onCancelActiveSession}
							type="button"
						>
							<svg
								aria-label="取消"
								className="control-button-icon"
								fill="currentColor"
								role="img"
								viewBox="0 0 16 16"
							>
								<path
									clipRule="evenodd"
									d="M8 16A8 8 0 108 0a8 8 0 000 16zM5.354 5.354a.5.5 0 01.708 0L8 7.293l1.938-1.939a.5.5 0 01.708.708L8.707 8l1.939 1.938a.5.5 0 01-.708.708L8 8.707l-1.938 1.939a.5.5 0 01-.708-.708L7.293 8 5.354 6.062a.5.5 0 010-.708z"
									fillRule="evenodd"
								/>
							</svg>
							<span>取消</span>
						</button>
					) : null}
					<button
						className={`control-button active primary-action-button primary-action-button-${primaryActionTone}`}
						disabled={!canRunPrimaryAction}
						onClick={onRunPrimaryAction}
						title={`${primaryActionLabel} (${primaryActionShortcutLabel})`}
						type="button"
					>
						<svg
							aria-label={primaryActionLabel}
							className="control-button-icon"
							fill="currentColor"
							role="img"
							viewBox="0 0 16 16"
						>
							<path d="M3 2a1 1 0 00-1 1v8a1 1 0 001 1h2v2l2.5-2H13a1 1 0 001-1V3a1 1 0 00-1-1H3z" />
							<path
								d="M8 4a1.5 1.5 0 00-1.5 1.5h1a.5.5 0 011 0c0 .28-.22.5-.5.5H8v1h.5c.83 0 1.5-.67 1.5-1.5S9.33 4 8.5 4H8z"
								fill="var(--button-background, #fff)"
							/>
							<circle cx="8" cy="8.5" r=".5" fill="var(--button-background, #fff)" />
						</svg>
						<span>{primaryActionLabel}</span>
						<span className="control-button-shortcut">{primaryActionShortcutLabel}</span>
					</button>
					{hasCompletion ? (
						<button
							className="control-button completion-button"
							onClick={onAcceptCompletion}
							title="Tab 补全"
							type="button"
						>
							<svg
								aria-label="补全"
								className="control-button-icon"
								fill="currentColor"
								role="img"
								viewBox="0 0 16 16"
							>
								<path
									clipRule="evenodd"
									d="M3.5 2A1.5 1.5 0 002 3.5v9A1.5 1.5 0 003.5 14h9a1.5 1.5 0 001.5-1.5v-9A1.5 1.5 0 0012.5 2h-9zM8 4.5a.75.75 0 01.53.22l2.5 2.5a.75.75 0 01-1.06 1.06L8.75 6.56v4.69a.75.75 0 01-1.5 0V6.56L6.03 8.28a.75.75 0 01-1.06-1.06l2.5-2.5A.75.75 0 018 4.5z"
									fillRule="evenodd"
								/>
							</svg>
							<span>补全</span>
						</button>
					) : null}
				</div>
			</div>
		</>
	);
}
