import hljs from "highlight.js/lib/core";
import jsonLanguage from "highlight.js/lib/languages/json";
import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import type { RefObject } from "react";
import type { AcpSessionDetail, WorkspaceState } from "../../../lib/tauri/types";
import type { ExecutionResult } from "../types";
import { formatWorkspacePath } from "../workspace";

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
				<pre className="result-card-code result-card-code-plain">{content}</pre>
			)}
		</section>
	);
}

interface LauncherFeedbackProps {
	activeSession: AcpSessionDetail | null;
	sessionLogRef: RefObject<HTMLDivElement | null>;
	result: ExecutionResult | null;
	jsonPreview: string | null;
	markdownPreview: string | null;
	workspace: WorkspaceState;
	error: string | null;
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
	sessionLogRef,
	result,
	jsonPreview,
	markdownPreview,
	workspace,
	error,
}: LauncherFeedbackProps) {
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
				<ResultCard
					label={
						resolveResultRenderMode(result) === "markdown"
							? "Markdown Output"
							: resolveResultRenderMode(result) === "json"
								? "JSON Output"
								: "Command Output"
					}
					content={result.primaryText}
					render={resolveResultRenderMode(result)}
				/>
			) : null}

			{activeSession ? (
				<p className="status-line">
					{activeSession.session.title}
					{" · "}
					{activeSession.session.agentName}
					{" · "}
					{formatWorkspacePath(activeSession.session.workspaceRoot, workspace)}
					{activeSession.session.lastError ? ` · ${activeSession.session.lastError}` : ""}
				</p>
			) : null}

			{error ? <p className="status-line error">{error}</p> : null}
		</>
	);
}
