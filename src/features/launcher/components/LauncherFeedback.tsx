import hljs from "highlight.js/lib/core";
import jsonLanguage from "highlight.js/lib/languages/json";
import {
	lazy,
	memo,
	Suspense,
	useEffect,
	useId,
	useLayoutEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type { ReactNode, RefObject } from "react";
import type {
	AcpConfigOption,
	AcpConfigOptionGroup,
	AcpSessionDetail,
	AcpSessionMessage,
} from "../../../lib/tauri/types";
import { LauncherPinButton } from "../../../app/LauncherPinButton";
import type {
	ExecutionResult,
	RagAnswerStructuredPayload,
	RagCitation,
	RagRetrievalSummary,
	TranslationResultStructuredPayload,
} from "../types";
import { isRagAnswerStructuredPayload, isTranslationResultStructuredPayload } from "../types";
import { RagCitationList } from "./RagCitationList";
import { ThoughtDisclosure } from "./ThoughtDisclosure";

type ResultRenderMode = "plain" | "json" | "markdown";

const SessionTimeline = lazy(() =>
	import("./SessionTimeline").then((module) => ({
		default: module.SessionTimeline,
	})),
);
const MarkdownRenderer = lazy(() =>
	import("./MarkdownRenderer").then((module) => ({
		default: module.MarkdownRenderer,
	})),
);

hljs.registerLanguage("json", jsonLanguage);

type CopyState = "idle" | "copied" | "failed";
const STREAM_FOLLOW_SLOP_PX = 24;

function isJsonContent(content: string): boolean {
	try {
		JSON.parse(content);
		return true;
	} catch {
		return false;
	}
}

interface ResultCardProps {
	label: string;
	content: string;
	render: ResultRenderMode;
	pending?: boolean;
}

function ResultCard({ label, content, render, pending = false }: ResultCardProps) {
	const [copyState, setCopyState] = useState<CopyState>("idle");
	const highlightedJson = useMemo(() => {
		if (render !== "json") {
			return null;
		}

		return hljs.highlight(content, {
			language: "json",
			ignoreIllegals: true,
		}).value;
	}, [content, render]);

	useEffect(() => {
		setCopyState("idle");
	}, [content]);

	useEffect(() => {
		if (copyState !== "copied") {
			return;
		}

		const timeoutId = window.setTimeout(() => {
			setCopyState("idle");
		}, 1600);

		return () => window.clearTimeout(timeoutId);
	}, [copyState]);

	async function handleCopy() {
		try {
			await navigator.clipboard.writeText(content);
			setCopyState("copied");
		} catch {
			setCopyState("failed");
		}
	}

	return (
		<section className="result-line result-card">
			<div className="result-card-header">
				<span className="result-card-label">{label}</span>
				<button
					type="button"
					className="control-button result-card-copy"
					onClick={() => void handleCopy()}
					aria-label={`复制${label}`}
					disabled={pending}
				>
					{resolveCopyLabel(copyState)}
				</button>
			</div>
			{render === "markdown" ? (
				<div className="result-card-markdown">
					<Suspense fallback={<p className="status-line">加载 Markdown 渲染器...</p>}>
						<MarkdownRenderer content={content} pending={pending} />
					</Suspense>
				</div>
			) : highlightedJson ? (
				<pre className={`result-card-code result-card-code-json${pending ? " pending" : ""}`}>
					<code
						className="hljs language-json"
						dangerouslySetInnerHTML={{ __html: highlightedJson }}
					/>
				</pre>
			) : (
				<pre className={`result-card-text${pending ? " pending" : ""}`}>{content}</pre>
			)}
		</section>
	);
}

const MemoizedResultCard = memo(ResultCard);

interface LauncherFeedbackProps {
	activeSession: AcpSessionDetail | null;
	launcherPinned: boolean;
	runtimeControlPendingKey: string | null;
	qaCitations: RagCitation[];
	qaMessages: AcpSessionMessage[];
	qaRetrieval: RagRetrievalSummary | null;
	sessionLogRef: RefObject<HTMLDivElement | null>;
	result: ExecutionResult | null;
	resultPending?: boolean;
	jsonPreview: string | null;
	markdownPreview: string | null;
	onSetAcpSessionConfigOption: (configId: string, valueId: string) => void | Promise<void>;
	onSetAcpSessionMode: (modeId: string) => void | Promise<void>;
	onLauncherPinnedChange?: (pinned: boolean) => void | Promise<void>;
	onOpenRagCitation: (citation: RagCitation) => void | Promise<void>;
}

function resolveCopyLabel(copyState: CopyState): string {
	switch (copyState) {
		case "copied":
			return "已复制";
		case "failed":
			return "重试";
		default:
			return "复制";
	}
}

function resolveResultRenderMode(result: ExecutionResult): ResultRenderMode {
	if (result.structuredPayload?.render === "markdown") {
		return "markdown";
	}

	if (result.primaryText && isJsonContent(result.primaryText)) {
		return "json";
	}

	return "plain";
}

function resolvePendingMessageBottom(container: HTMLDivElement): number | null {
	const pendingMessage = container.querySelector<HTMLElement>('[data-session-pending="true"]');
	if (!pendingMessage) {
		return null;
	}

	const containerRect = container.getBoundingClientRect();
	const pendingRect = pendingMessage.getBoundingClientRect();
	return container.scrollTop + (pendingRect.bottom - containerRect.top);
}

function isFollowingPendingMessage(container: HTMLDivElement): boolean {
	const pendingBottom = resolvePendingMessageBottom(container);
	if (pendingBottom === null) {
		return true;
	}

	const viewportBottom = container.scrollTop + container.clientHeight;
	return pendingBottom - viewportBottom <= STREAM_FOLLOW_SLOP_PX;
}

function scrollPendingMessageIntoView(container: HTMLDivElement) {
	const pendingBottom = resolvePendingMessageBottom(container);
	if (pendingBottom === null) {
		return;
	}

	const targetScrollTop = Math.max(0, pendingBottom - container.clientHeight);
	if (Math.abs(container.scrollTop - targetScrollTop) <= 1) {
		return;
	}

	container.scrollTop = targetScrollTop;
}

export function LauncherFeedback({
	activeSession,
	launcherPinned,
	runtimeControlPendingKey,
	qaCitations,
	qaMessages,
	qaRetrieval,
	sessionLogRef,
	result,
	resultPending = false,
	jsonPreview,
	markdownPreview,
	onSetAcpSessionConfigOption,
	onSetAcpSessionMode,
	onLauncherPinnedChange,
	onOpenRagCitation,
}: LauncherFeedbackProps) {
	const ragPayload = resolveRagPayload(result);
	const resultRenderMode = result ? resolveResultRenderMode(result) : null;
	const visibleQaMessages = qaMessages.filter((message) => message.role !== "system");
	const shouldShowQaMessages =
		!activeSession && !jsonPreview && !markdownPreview && !result && visibleQaMessages.length > 0;
	const shouldShowQaCitations =
		shouldShowQaMessages && qaRetrieval !== null && qaCitations.length > 0;
	let content: ReactNode = null;

	if (activeSession) {
		content = (
			<SessionFeedback
				activeSession={activeSession}
				runtimeControlPendingKey={runtimeControlPendingKey}
				sessionLogRef={sessionLogRef}
				onSetAcpSessionConfigOption={onSetAcpSessionConfigOption}
				onSetAcpSessionMode={onSetAcpSessionMode}
			/>
		);
	} else if (jsonPreview) {
		content = <MemoizedResultCard label="JSON 预览" content={jsonPreview} render="json" />;
	} else if (markdownPreview) {
		content = (
			<MemoizedResultCard label="Markdown 预览" content={markdownPreview} render="markdown" />
		);
	} else if (result?.primaryText) {
		content = (
			<ResultFeedback
				onOpenRagCitation={onOpenRagCitation}
				ragPayload={ragPayload}
				result={result}
				resultPending={resultPending}
				resultRenderMode={resultRenderMode}
			/>
		);
	} else if (shouldShowQaMessages) {
		content = (
			<QaFeedback
				onOpenRagCitation={onOpenRagCitation}
				qaCitations={qaCitations}
				qaRetrieval={qaRetrieval}
				sessionLogRef={sessionLogRef}
				shouldShowQaCitations={shouldShowQaCitations}
				visibleQaMessages={visibleQaMessages}
			/>
		);
	}

	if (!content) {
		return null;
	}

	return (
		<section className="launcher-feedback-region" aria-label="交互内容">
			{onLauncherPinnedChange ? (
				<div className="launcher-feedback-toolbar">
					<LauncherPinButton
						className="launcher-feedback-pin-button"
						pinned={launcherPinned}
						onToggle={() => void onLauncherPinnedChange(!launcherPinned)}
					/>
				</div>
			) : null}
			{content}
		</section>
	);
}

const SessionFeedback = memo(function SessionFeedback({
	activeSession,
	runtimeControlPendingKey,
	sessionLogRef,
	onSetAcpSessionConfigOption,
	onSetAcpSessionMode,
}: {
	activeSession: AcpSessionDetail;
	runtimeControlPendingKey: string | null;
	sessionLogRef: RefObject<HTMLDivElement | null>;
	onSetAcpSessionConfigOption: (configId: string, valueId: string) => void | Promise<void>;
	onSetAcpSessionMode: (modeId: string) => void | Promise<void>;
}) {
	const shouldFollowPendingRef = useRef(true);
	const trackedSessionIdRef = useRef<string | null>(null);
	const currentMode = useMemo(
		() =>
			activeSession.runtime.availableModes.find(
				(mode) => mode.id === activeSession.runtime.currentModeId,
			) ?? null,
		[activeSession.runtime.availableModes, activeSession.runtime.currentModeId],
	);
	const hasRuntimeControls =
		activeSession.runtime.availableModes.length > 0 ||
		activeSession.runtime.configOptions.length > 0;
	const isRuntimeReadOnly = activeSession.session.status !== "idle";
	const visibleMessages = useMemo(
		() => activeSession.messages.filter((message) => message.role !== "system"),
		[activeSession.messages],
	);

	useEffect(() => {
		const logElement = sessionLogRef.current;
		if (!logElement) {
			return;
		}

		const syncFollowState = () => {
			shouldFollowPendingRef.current = isFollowingPendingMessage(logElement);
		};

		syncFollowState();
		logElement.addEventListener("scroll", syncFollowState, { passive: true });
		return () => {
			logElement.removeEventListener("scroll", syncFollowState);
		};
	}, [activeSession.session.sessionId, sessionLogRef]);

	useLayoutEffect(() => {
		const logElement = sessionLogRef.current;
		if (!logElement) {
			return;
		}

		if (trackedSessionIdRef.current !== activeSession.session.sessionId) {
			trackedSessionIdRef.current = activeSession.session.sessionId;
			shouldFollowPendingRef.current = true;
		}

		if (activeSession.session.status !== "running" || !shouldFollowPendingRef.current) {
			return;
		}

		scrollPendingMessageIntoView(logElement);
	}, [activeSession, sessionLogRef]);

	return (
		<div className="session-log" ref={sessionLogRef}>
			{hasRuntimeControls ? (
				<section className="session-runtime-panel" aria-label="Pi Agent 运行时配置">
					<header className="session-runtime-header">
						<div className="session-runtime-heading">
							<span className="session-runtime-kicker">Pi Agent Runtime</span>
							<strong className="session-runtime-title">当前会话运行时</strong>
						</div>
						<span className="session-runtime-summary">
							{activeSession.session.status === "idle" ? "可切换" : "运行中时只读"}
						</span>
					</header>
					<div className="session-runtime-grid">
						{activeSession.runtime.availableModes.length > 0 ? (
							<label className="session-runtime-field">
								<span className="session-runtime-label">运行模式</span>
								<select
									className="session-runtime-select"
									value={activeSession.runtime.currentModeId ?? ""}
									disabled={isRuntimeReadOnly || runtimeControlPendingKey === "mode"}
									onChange={(event) => {
										if (event.target.value) {
											void onSetAcpSessionMode(event.target.value);
										}
									}}
								>
									{activeSession.runtime.availableModes.map((mode) => (
										<option key={mode.id} value={mode.id}>
											{mode.name}
										</option>
									))}
								</select>
								{currentMode?.description ? (
									<span className="session-runtime-help">{currentMode.description}</span>
								) : null}
							</label>
						) : null}
						{activeSession.runtime.configOptions.map((option) => (
							<label className="session-runtime-field" key={option.id}>
								<span className="session-runtime-label-row">
									<span className="session-runtime-label">{option.name}</span>
									{option.category ? (
										<span className="session-runtime-badge">
											{formatConfigCategory(option.category)}
										</span>
									) : null}
								</span>
								<select
									className="session-runtime-select"
									value={option.kind.currentValueId}
									disabled={isRuntimeReadOnly || runtimeControlPendingKey === `config:${option.id}`}
									onChange={(event) => {
										void onSetAcpSessionConfigOption(option.id, event.target.value);
									}}
								>
									{renderConfigOptions(option)}
								</select>
								{option.description ? (
									<span className="session-runtime-help">{option.description}</span>
								) : null}
							</label>
						))}
					</div>
				</section>
			) : null}
			<Suspense fallback={<p className="status-line">加载会话内容...</p>}>
				<SessionTimeline messages={visibleMessages} />
			</Suspense>
		</div>
	);
});

function formatConfigCategory(category: string) {
	switch (category) {
		case "model":
			return "模型";
		case "mode":
			return "模式";
		case "thought_level":
			return "思考级别";
		default:
			return category;
	}
}

function renderConfigOptions(option: AcpConfigOption) {
	if (option.kind.groups.length > 0) {
		return option.kind.groups.map((group) => renderConfigOptionGroup(group));
	}

	return option.kind.options.map((entry) => (
		<option key={entry.valueId} value={entry.valueId}>
			{entry.name}
		</option>
	));
}

function renderConfigOptionGroup(group: AcpConfigOptionGroup) {
	return (
		<optgroup key={group.id} label={group.name}>
			{group.options.map((entry) => (
				<option key={entry.valueId} value={entry.valueId}>
					{entry.name}
				</option>
			))}
		</optgroup>
	);
}

const ResultFeedback = memo(function ResultFeedback({
	onOpenRagCitation,
	ragPayload,
	result,
	resultPending,
	resultRenderMode,
}: {
	onOpenRagCitation: (citation: RagCitation) => void | Promise<void>;
	ragPayload: ReturnType<typeof resolveRagPayload>;
	result: ExecutionResult;
	resultPending: boolean;
	resultRenderMode: ResultRenderMode | null;
}) {
	const resultLabel = resolveResultLabel(ragPayload, resultRenderMode);
	const translationPayload = resolveTranslationPayload(result);
	const translationReasoning = translationPayload?.reasoning?.trim() ?? "";
	const [thoughtCollapsed, setThoughtCollapsed] = useState(true);
	const thoughtId = useId();

	useEffect(() => {
		setThoughtCollapsed(true);
	}, [translationReasoning]);

	return (
		<>
			<MemoizedResultCard
				label={resultLabel}
				content={result.primaryText ?? ""}
				render={resultRenderMode ?? "plain"}
				pending={resultPending}
			/>
			{translationReasoning ? (
				<ThoughtDisclosure
					content={translationReasoning}
					contentId={`translation-thought-${thoughtId}`}
					collapsed={thoughtCollapsed}
					onToggle={() => setThoughtCollapsed((current) => !current)}
				/>
			) : null}
			{ragPayload ? (
				<RagCitationList
					citations={ragPayload.citations}
					retrieval={ragPayload.retrieval}
					onOpenCitation={onOpenRagCitation}
				/>
			) : null}
		</>
	);
});

const QaFeedback = memo(function QaFeedback({
	onOpenRagCitation,
	qaCitations,
	qaRetrieval,
	sessionLogRef,
	shouldShowQaCitations,
	visibleQaMessages,
}: {
	onOpenRagCitation: (citation: RagCitation) => void | Promise<void>;
	qaCitations: RagCitation[];
	qaRetrieval: RagRetrievalSummary | null;
	sessionLogRef: RefObject<HTMLDivElement | null>;
	shouldShowQaCitations: boolean;
	visibleQaMessages: AcpSessionMessage[];
}) {
	return (
		<>
			<div className="session-log" ref={sessionLogRef}>
				<Suspense fallback={<p className="status-line">加载问答历史...</p>}>
					<SessionTimeline messages={visibleQaMessages} />
				</Suspense>
			</div>
			{shouldShowQaCitations && qaRetrieval ? (
				<RagCitationList
					citations={qaCitations}
					retrieval={qaRetrieval}
					onOpenCitation={onOpenRagCitation}
				/>
			) : null}
		</>
	);
});

function resolveResultLabel(
	ragPayload: ReturnType<typeof resolveRagPayload>,
	resultRenderMode: ResultRenderMode | null,
): string {
	if (ragPayload) {
		return "文档回答";
	}

	switch (resultRenderMode) {
		case "markdown":
			return "Markdown 输出";
		case "json":
			return "JSON 输出";
		default:
			return "文本结果";
	}
}

function resolveRagPayload(result: ExecutionResult | null): RagAnswerStructuredPayload | null {
	return result && isRagAnswerStructuredPayload(result.structuredPayload)
		? result.structuredPayload
		: null;
}

function resolveTranslationPayload(
	result: ExecutionResult | null,
): TranslationResultStructuredPayload | null {
	return result && isTranslationResultStructuredPayload(result.structuredPayload)
		? result.structuredPayload
		: null;
}
