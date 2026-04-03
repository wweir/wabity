import hljs from "highlight.js/lib/core";
import jsonLanguage from "highlight.js/lib/languages/json";
import { lazy, memo, Suspense, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import type { AcpSessionDetail, AcpSessionMessage } from "../../../lib/tauri/types";
import type {
	ExecutionResult,
	RagAnswerStructuredPayload,
	RagCitation,
	RagRetrievalSummary,
} from "../types";
import { isRagAnswerStructuredPayload } from "../types";
import { RagCitationList } from "./RagCitationList";

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
					aria-label={`Copy ${label}`}
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
	qaCitations: RagCitation[];
	qaMessages: AcpSessionMessage[];
	qaRetrieval: RagRetrievalSummary | null;
	sessionLogRef: RefObject<HTMLDivElement | null>;
	result: ExecutionResult | null;
	resultPending?: boolean;
	jsonPreview: string | null;
	markdownPreview: string | null;
	onOpenRagCitation: (citation: RagCitation) => void | Promise<void>;
}

function resolveCopyLabel(copyState: CopyState): string {
	switch (copyState) {
		case "copied":
			return "Copied";
		case "failed":
			return "Retry";
		default:
			return "Copy";
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
	qaCitations,
	qaMessages,
	qaRetrieval,
	sessionLogRef,
	result,
	resultPending = false,
	jsonPreview,
	markdownPreview,
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
		content = <SessionFeedback activeSession={activeSession} sessionLogRef={sessionLogRef} />;
	} else if (jsonPreview) {
		content = <MemoizedResultCard label="JSON Preview" content={jsonPreview} render="json" />;
	} else if (markdownPreview) {
		content = (
			<MemoizedResultCard label="Markdown Preview" content={markdownPreview} render="markdown" />
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

	return <>{content}</>;
}

const SessionFeedback = memo(function SessionFeedback({
	activeSession,
	sessionLogRef,
}: {
	activeSession: AcpSessionDetail;
	sessionLogRef: RefObject<HTMLDivElement | null>;
}) {
	const shouldFollowPendingRef = useRef(true);
	const trackedSessionIdRef = useRef<string | null>(null);
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
			<Suspense fallback={<p className="status-line">加载会话内容...</p>}>
				<SessionTimeline messages={visibleMessages} />
			</Suspense>
		</div>
	);
});

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

	return (
		<>
			<MemoizedResultCard
				label={resultLabel}
				content={result.primaryText ?? ""}
				render={resultRenderMode ?? "plain"}
				pending={resultPending}
			/>
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
		return "Document Answer";
	}

	switch (resultRenderMode) {
		case "markdown":
			return "Markdown Output";
		case "json":
			return "JSON Output";
		default:
			return "Text Output";
	}
}

function resolveRagPayload(result: ExecutionResult | null): RagAnswerStructuredPayload | null {
	return result && isRagAnswerStructuredPayload(result.structuredPayload)
		? result.structuredPayload
		: null;
}
