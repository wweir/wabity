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
	statusText: string | null;
	onUpdateRawText: (value: string, caretIndex: number) => void;
	onSyncCaretIndex: (element: HTMLInputElement | HTMLTextAreaElement) => void;
	onKeyDown: (event: KeyboardEvent<HTMLInputElement> | KeyboardEvent<HTMLTextAreaElement>) => void;
	onOpenSettings?: () => void;
	hasCompletion: boolean;
	onAcceptCompletion: () => void;
	loading: boolean;
	creatingSession: boolean;
	showAgentAction: boolean;
	primaryActionShortcutLabel: string;
	primaryActionLabel: string;
	canRunPrimaryAction: boolean;
	agentActionShortcutLabel: string;
	onAgentExecute: () => void;
	showCancelActiveSession: boolean;
	onCancelActiveSession: () => void;
	onRunPrimaryAction: () => void;
	onCloseLauncher: () => void;
}

function LauncherStatusBar({ statusText }: { statusText: string | null }) {
	const viewportRef = useRef<HTMLDivElement | null>(null);
	const measureRef = useRef<HTMLSpanElement | null>(null);
	const [shouldScroll, setShouldScroll] = useState(false);
	const [scrollDistance, setScrollDistance] = useState(0);
	const [scrollDurationSeconds, setScrollDurationSeconds] = useState(0);
	const normalizedStatusText = statusText?.trim() ?? "";

	useLayoutEffect(() => {
		const viewportElement = viewportRef.current;
		const measureElement = measureRef.current;
		if (!viewportElement || !measureElement || !normalizedStatusText) {
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
	}, [normalizedStatusText]);

	return (
		<div className="launcher-status-bar" aria-label="状态栏">
			<span className="launcher-status-bar-label">最近输入</span>
			<div className="launcher-status-bar-viewport" ref={viewportRef}>
				{normalizedStatusText ? (
					shouldScroll ? (
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
				<span className="launcher-status-bar-measure" ref={measureRef}>
					{normalizedStatusText}
				</span>
			</div>
		</div>
	);
}

export function LauncherComposer({
	inputMode,
	inputRef,
	rawText,
	inputPlaceholder,
	statusText,
	onUpdateRawText,
	onSyncCaretIndex,
	onKeyDown,
	onOpenSettings,
	hasCompletion,
	onAcceptCompletion,
	loading,
	creatingSession,
	showAgentAction,
	primaryActionShortcutLabel,
	primaryActionLabel,
	canRunPrimaryAction,
	agentActionShortcutLabel,
	onAgentExecute,
	showCancelActiveSession,
	onCancelActiveSession,
	onRunPrimaryAction,
	onCloseLauncher,
}: LauncherComposerProps) {
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

	return (
		<>
			{inputMode === "inline" ? (
				<input
					{...inputProps}
					className="launcher-input inline"
					ref={(element) => {
						inputRef.current = element;
					}}
					type="text"
				/>
			) : (
				<textarea
					{...inputProps}
					className="launcher-input multiline"
					ref={(element) => {
						inputRef.current = element;
					}}
					rows={4}
				/>
			)}

			<div className="control-row">
				<LauncherStatusBar statusText={statusText} />
				<div className="action-buttons">
					{onOpenSettings ? (
						<button
							className="control-button settings-button"
							onClick={onOpenSettings}
							type="button"
						>
							设置
						</button>
					) : null}
					{hasCompletion ? (
						<button className="control-button" onClick={onAcceptCompletion} type="button">
							补全
						</button>
					) : null}
					{showAgentAction ? (
						<button
							className="control-button agent-execute-button"
							disabled={loading || creatingSession || rawText.trim().length === 0}
							onClick={onAgentExecute}
							title={`Agent 执行 (${agentActionShortcutLabel})`}
							type="button"
						>
							<span>{loading ? "启动 Agent..." : "Agent 执行"}</span>
							{loading ? null : (
								<span className="control-button-shortcut">{agentActionShortcutLabel}</span>
							)}
						</button>
					) : null}
					{showCancelActiveSession ? (
						<button className="control-button" onClick={onCancelActiveSession} type="button">
							取消
						</button>
					) : null}
					<button
						className="control-button active primary-action-button"
						disabled={!canRunPrimaryAction}
						onClick={onRunPrimaryAction}
						title={`${primaryActionLabel} (${primaryActionShortcutLabel})`}
						type="button"
					>
						<span>{primaryActionLabel}</span>
						<span className="control-button-shortcut">{primaryActionShortcutLabel}</span>
					</button>
					<button className="control-button" onClick={onCloseLauncher} type="button">
						关闭
					</button>
				</div>
			</div>
		</>
	);
}
