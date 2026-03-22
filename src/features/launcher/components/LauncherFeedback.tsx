import hljs from "highlight.js/lib/core";
import jsonLanguage from "highlight.js/lib/languages/json";
import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import type { RefObject } from "react";
import type { AcpSessionDetail, AcpSessionMessage } from "../../../lib/tauri/types";
import type { ExecutionResult, RagCitation, RagRetrievalSummary } from "../types";
import { isRagAnswerStructuredPayload } from "../types";
import { RagCitationList } from "./RagCitationList";

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

function isJsonContent(content: string) {
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
	render: "plain" | "json" | "markdown";
}

function ResultCard({ label, content, render }: ResultCardProps) {
	const [copyState, setCopyState] = useState<CopyState>("idle");
	const isJson = useMemo(() => render === "json", [render]);
	const highlightedJson = useMemo(() => {
		if (!isJson) {
			return null;
		}

		return hljs.highlight(content, {
			language: "json",
			ignoreIllegals: true,
		}).value;
	}, [content, isJson]);

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
				>
					{copyState === "copied" ? "Copied" : copyState === "failed" ? "Retry" : "Copy"}
				</button>
			</div>
			{render === "markdown" ? (
				<div className="result-card-markdown">
					<Suspense fallback={<p className="status-line">加载 Markdown 渲染器...</p>}>
						<MarkdownRenderer content={content} />
					</Suspense>
				</div>
			) : highlightedJson ? (
				<pre className="result-card-code result-card-code-json">
					<code
						className="hljs language-json"
						dangerouslySetInnerHTML={{ __html: highlightedJson }}
					/>
				</pre>
			) : (
				<pre className="result-card-text">{content}</pre>
			)}
		</section>
	);
}

interface LauncherFeedbackProps {
	activeSession: AcpSessionDetail | null;
	qaCitations: RagCitation[];
	qaMessages: AcpSessionMessage[];
	qaRetrieval: RagRetrievalSummary | null;
	sessionLogRef: RefObject<HTMLDivElement | null>;
	result: ExecutionResult | null;
	jsonPreview: string | null;
	markdownPreview: string | null;
	onOpenRagCitation: (citation: RagCitation) => void | Promise<void>;
}

function resolveResultRenderMode(result: ExecutionResult): "plain" | "json" | "markdown" {
	if (result.structuredPayload?.render === "markdown") {
		return "markdown";
	}

	if (result.primaryText && isJsonContent(result.primaryText)) {
		return "json";
	}

	return "plain";
}

export function LauncherFeedback({
	activeSession,
	qaCitations,
	qaMessages,
	qaRetrieval,
	sessionLogRef,
	result,
	jsonPreview,
	markdownPreview,
	onOpenRagCitation,
}: LauncherFeedbackProps) {
	const ragPayload = useMemo(
		() =>
			result && isRagAnswerStructuredPayload(result.structuredPayload)
				? result.structuredPayload
				: null,
		[result],
	);
	const resultRenderMode = useMemo(
		() => (result ? resolveResultRenderMode(result) : null),
		[result],
	);
	const visibleQaMessages = useMemo(
		() => qaMessages.filter((message) => message.role !== "system"),
		[qaMessages],
	);
	const shouldShowQaMessages =
		!activeSession && !jsonPreview && !markdownPreview && !result && visibleQaMessages.length > 0;
	const shouldShowQaCitations =
		shouldShowQaMessages && qaRetrieval !== null && qaCitations.length > 0;

	return (
		<>
			{activeSession ? (
				<div className="session-log" ref={sessionLogRef}>
					<Suspense fallback={<p className="status-line">加载会话内容...</p>}>
						<SessionTimeline
							messages={activeSession.messages.filter((message) => message.role !== "system")}
						/>
					</Suspense>
				</div>
			) : jsonPreview ? (
				<ResultCard label="JSON Preview" content={jsonPreview} render="json" />
			) : markdownPreview ? (
				<ResultCard label="Markdown Preview" content={markdownPreview} render="markdown" />
			) : result?.primaryText ? (
				<>
					<ResultCard
						label={
							ragPayload
								? "Document Answer"
								: resultRenderMode === "markdown"
									? "Markdown Output"
									: resultRenderMode === "json"
										? "JSON Output"
										: "Text Output"
						}
						content={result.primaryText}
						render={resultRenderMode ?? "plain"}
					/>
					{ragPayload ? (
						<RagCitationList
							citations={ragPayload.citations}
							retrieval={ragPayload.retrieval}
							onOpenCitation={onOpenRagCitation}
						/>
					) : null}
				</>
			) : shouldShowQaMessages ? (
				<>
					<div className="session-log" ref={sessionLogRef}>
						<Suspense fallback={<p className="status-line">加载问答历史...</p>}>
							<SessionTimeline messages={visibleQaMessages} />
						</Suspense>
					</div>
					{shouldShowQaCitations ? (
						<RagCitationList
							citations={qaCitations}
							retrieval={qaRetrieval}
							onOpenCitation={onOpenRagCitation}
						/>
					) : null}
				</>
			) : null}
		</>
	);
}
