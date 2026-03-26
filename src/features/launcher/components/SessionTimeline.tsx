import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { AcpActionEvent, AcpMessageBlock, AcpSessionMessage } from "../../../lib/tauri/types";
import { MarkdownRenderer } from "./MarkdownRenderer";

type ActionKind =
	| "tool"
	| "tool-call"
	| "tool-update"
	| "plan"
	| "mode"
	| "config"
	| "commands"
	| "info"
	| "error";
type RawActionKind = Exclude<ActionKind, "tool">;

interface ActionPill {
	id: string;
	kind: ActionKind;
	title: string;
	detail: string | null;
	count: number;
	correlationId: string | null;
	inputDetail: string | null;
	outputDetail: string | null;
}

function isStepMarkerPill(pill: ActionPill) {
	return pill.kind === "info" && /^第\s*\d+\s*步$/u.test(pill.title.trim());
}

function getActionIcon(kind: ActionKind): string {
	switch (kind) {
		case "tool":
		case "tool-call":
			return "⚙";
		case "tool-update":
			return "↻";
		case "plan":
			return "📋";
		case "mode":
			return "🔀";
		case "config":
			return "⚙";
		case "commands":
			return "📝";
		case "info":
			return "ℹ";
		default:
			return "•";
	}
}

function normalizeActionKind(kind: string): RawActionKind {
	switch (kind) {
		case "tool-call":
		case "tool-update":
		case "plan":
		case "mode":
		case "config":
		case "commands":
		case "info":
		case "error":
			return kind;
		default:
			return "info";
	}
}

function appendDetailSection(current: string | null, next: string | null) {
	if (!next) {
		return current;
	}

	if (!current || current === next) {
		return next;
	}

	return `${current}\n\n${next}`;
}

function formatToolDetail(inputDetail: string | null, outputDetail: string | null) {
	const sections: string[] = [];

	if (inputDetail) {
		sections.push(`输入\n${inputDetail}`);
	}

	if (outputDetail) {
		sections.push(`输出\n${outputDetail}`);
	}

	return sections.length > 0 ? sections.join("\n\n") : null;
}

function decodeEscapedSequences(value: string) {
	return value.replace(/\\u([0-9a-fA-F]{4})|\\(["\\/bfnrt])/g, (match, unicodeHex, escapedChar) => {
		if (unicodeHex) {
			return String.fromCharCode(Number.parseInt(unicodeHex, 16));
		}

		switch (escapedChar) {
			case "b":
				return "\b";
			case "f":
				return "\f";
			case "n":
				return "\n";
			case "r":
				return "\r";
			case "t":
				return "\t";
			case '"':
				return '"';
			case "\\":
				return "\\";
			case "/":
				return "/";
			default:
				return match;
		}
	});
}

function formatActionDetailForDisplay(detail: string | null) {
	if (!detail) {
		return null;
	}

	const trimmed = detail.trim();
	if (trimmed.startsWith('"') && trimmed.endsWith('"')) {
		try {
			const parsed = JSON.parse(trimmed);
			if (typeof parsed === "string") {
				return parsed;
			}
		} catch {
			return decodeEscapedSequences(detail);
		}
	}

	return decodeEscapedSequences(detail);
}

function summarizeThoughtPreview(content: string) {
	const singleLine = content.replace(/\s+/g, " ").trim();
	if (singleLine.length <= 36) {
		return singleLine;
	}

	const preferred = singleLine.slice(0, 36);
	const hardStopIndex = Math.max(
		preferred.lastIndexOf("。"),
		preferred.lastIndexOf("！"),
		preferred.lastIndexOf("？"),
		preferred.lastIndexOf(". "),
		preferred.lastIndexOf("!"),
		preferred.lastIndexOf("?"),
	);
	if (hardStopIndex >= 18) {
		return preferred.slice(0, hardStopIndex + 1).trim();
	}

	const softStopIndex = Math.max(
		preferred.lastIndexOf("，"),
		preferred.lastIndexOf("、"),
		preferred.lastIndexOf(","),
		preferred.lastIndexOf(" "),
	);
	if (softStopIndex >= 18) {
		return `${preferred.slice(0, softStopIndex).trim()}…`;
	}

	return `${preferred.trimEnd()}…`;
}

function splitThoughtParagraphs(content: string) {
	return content
		.replace(/\r\n?/g, "\n")
		.split(/\n{2,}/)
		.map((paragraph) => paragraph.trim())
		.filter((paragraph) => paragraph.length > 0);
}

function createToolPill(action: AcpActionEvent, index: number): ActionPill {
	const correlationId = action.correlationId ?? null;
	const inputDetail = action.kind === "tool-call" ? action.detail : null;
	const outputDetail = action.kind === "tool-update" ? action.detail : null;

	return {
		id: correlationId ? `tool:${correlationId}` : `tool:${action.title}:${index}`,
		kind: "tool",
		title: action.title || "工具调用",
		detail: formatToolDetail(inputDetail, outputDetail),
		count: 1,
		correlationId,
		inputDetail,
		outputDetail,
	};
}

function mergeToolAction(pill: ActionPill, action: AcpActionEvent) {
	if (action.title && (!pill.title || pill.title === "工具调用" || pill.title === "工具结果")) {
		pill.title = action.title;
	}

	if (action.kind === "tool-call") {
		pill.inputDetail = appendDetailSection(pill.inputDetail, action.detail);
	} else {
		pill.outputDetail = appendDetailSection(pill.outputDetail, action.detail);
	}

	pill.detail = formatToolDetail(pill.inputDetail, pill.outputDetail);
}

function mergeActions(actions: AcpActionEvent[]): ActionPill[] {
	const pills: ActionPill[] = [];
	const toolPillIndexes = new Map<string, number>();

	for (const [index, action] of actions.entries()) {
		const correlationId = action.correlationId ?? null;
		const isToolAction = action.kind === "tool-call" || action.kind === "tool-update";

		if (isToolAction) {
			if (correlationId) {
				const pillIndex = toolPillIndexes.get(correlationId);
				if (pillIndex !== undefined) {
					mergeToolAction(pills[pillIndex], action);
					continue;
				}

				pills.push(createToolPill(action, index));
				toolPillIndexes.set(correlationId, pills.length - 1);
				continue;
			}

			const previousPill = pills[pills.length - 1];
			if (
				action.kind === "tool-call" &&
				previousPill &&
				previousPill.kind === "tool" &&
				previousPill.correlationId === null &&
				previousPill.title === action.title &&
				previousPill.outputDetail === null
			) {
				previousPill.count += 1;
				mergeToolAction(previousPill, action);
				continue;
			}

			pills.push(createToolPill(action, index));
			continue;
		}

		pills.push({
			id: correlationId
				? `${action.kind}:${correlationId}`
				: `${action.kind}:${action.title}:${index}`,
			kind: normalizeActionKind(action.kind),
			title: action.title,
			detail: action.detail,
			count: 1,
			correlationId,
			inputDetail: null,
			outputDetail: null,
		});
	}

	return pills;
}

function ActionBar({ actions }: { actions: AcpActionEvent[] }) {
	const [expandedPill, setExpandedPill] = useState<string | null>(null);
	const pills = useMemo(() => mergeActions(actions), [actions]);
	const visibleDetailPill = expandedPill
		? (pills.find((pill) => pill.id === expandedPill && !isStepMarkerPill(pill)) ?? null)
		: null;
	const detailPanelId = visibleDetailPill ? `action-detail-${visibleDetailPill.id}` : null;

	useEffect(() => {
		if (expandedPill && !pills.some((pill) => pill.id === expandedPill)) {
			setExpandedPill(null);
		}
	}, [expandedPill, pills]);

	if (pills.length === 0) {
		return null;
	}

	return (
		<div className="action-bar" role="group" aria-label="操作轨迹">
			<div className="action-bar-content">
				{pills.map((pill) =>
					isStepMarkerPill(pill) ? (
						<span key={pill.id} className="action-step-marker">
							{pill.title}
						</span>
					) : pill.detail ? (
						<button
							key={pill.id}
							type="button"
							className={`action-pill ${pill.kind} interactive${expandedPill === pill.id ? " detail-open" : ""}`}
							aria-expanded={expandedPill === pill.id}
							aria-controls={expandedPill === pill.id && detailPanelId ? detailPanelId : undefined}
							aria-label={`${expandedPill === pill.id ? "收起" : "查看"} ${pill.title} 详情`}
							onClick={() => setExpandedPill(expandedPill === pill.id ? null : pill.id)}
						>
							<span className="action-pill-icon">{getActionIcon(pill.kind)}</span>
							<span className="action-pill-title">{pill.title}</span>
							{pill.count > 1 ? <span className="action-pill-count">×{pill.count}</span> : null}
						</button>
					) : (
						<span key={pill.id} className={`action-pill ${pill.kind}`}>
							<span className="action-pill-icon">{getActionIcon(pill.kind)}</span>
							<span className="action-pill-title">{pill.title}</span>
							{pill.count > 1 ? <span className="action-pill-count">×{pill.count}</span> : null}
						</span>
					),
				)}
			</div>
			{visibleDetailPill?.detail ? (
				<div
					className="action-bar-detail"
					id={detailPanelId ?? undefined}
					role="region"
					aria-label={`${visibleDetailPill.title} 详情`}
				>
					<span className="action-bar-detail-label">
						{visibleDetailPill.kind === "tool" ? "调用细节" : "详情"}
					</span>
					<pre>{formatActionDetailForDisplay(visibleDetailPill.detail)}</pre>
				</div>
			) : null}
		</div>
	);
}

function ThoughtToggle({
	contentId,
	collapsed,
	onToggle,
	preview,
	className,
}: {
	contentId: string;
	collapsed: boolean;
	onToggle: () => void;
	preview?: string;
	className?: string;
}) {
	const summary = collapsed ? preview?.trim() : null;

	return (
		<button
			aria-controls={contentId}
			aria-expanded={!collapsed}
			aria-label={collapsed ? "展开辅助思路" : "隐藏辅助思路"}
			className={className ? `thought-toggle ${className}` : "thought-toggle"}
			onClick={onToggle}
			type="button"
		>
			<span className="thought-toggle-copy">
				<span className="thought-label">思路</span>
				{summary ? <span className="thought-preview">{summary}</span> : null}
			</span>
			<span aria-hidden="true" className="thought-caret">
				{collapsed ? "展开" : "隐藏"}
			</span>
		</button>
	);
}

function ThoughtContent({
	content,
	contentId,
	className,
}: {
	content: string;
	contentId: string;
	className?: string;
}) {
	const paragraphs = splitThoughtParagraphs(content);

	return (
		<div
			className={className ? `thought-content ${className}` : "thought-content"}
			id={contentId}
			role="region"
			aria-label="思路详情"
		>
			{paragraphs.map((paragraph, index) => (
				<p key={`${contentId}-paragraph-${index}`} className="thought-paragraph">
					{paragraph}
				</p>
			))}
		</div>
	);
}

function ThoughtBlock({
	id,
	content,
	collapsed,
	onToggle,
}: {
	id: string;
	content: string;
	collapsed: boolean;
	onToggle: () => void;
}) {
	const contentId = `${id}-content`;

	return (
		<div className="message-block thought-block">
			<ThoughtToggle
				contentId={contentId}
				collapsed={collapsed}
				onToggle={onToggle}
				preview={summarizeThoughtPreview(content)}
			/>
			{!collapsed ? <ThoughtContent content={content} contentId={contentId} /> : null}
		</div>
	);
}

function MarkdownContent({ content, pending }: { content: string; pending: boolean }) {
	return (
		<MarkdownRenderer
			content={content}
			className="session-message-content markdown-body"
			pending={pending}
		/>
	);
}

function normalizeAssistantBlocks(blocks: AcpMessageBlock[]) {
	const thoughtParts: string[] = [];
	const actionItems: AcpActionEvent[] = [];
	const contentParts: string[] = [];

	for (const block of blocks) {
		switch (block.type) {
			case "thought":
				thoughtParts.push(block.content);
				break;
			case "actions":
				actionItems.push(...block.items);
				break;
			case "content":
				contentParts.push(block.text);
				break;
		}
	}

	const normalizedBlocks: AcpMessageBlock[] = [];
	if (contentParts.length > 0) {
		normalizedBlocks.push({ type: "content", text: contentParts.join("") });
	}
	if (actionItems.length > 0) {
		normalizedBlocks.push({ type: "actions", items: actionItems });
	}
	if (thoughtParts.length > 0) {
		normalizedBlocks.push({ type: "thought", content: thoughtParts.join("") });
	}

	return normalizedBlocks;
}

function getUserMessageContent(message: AcpSessionMessage) {
	return message.blocks
		.filter(
			(block: AcpMessageBlock): block is { type: "content"; text: string } =>
				block.type === "content",
		)
		.map((block) => block.text)
		.join("");
}

export function SessionTimeline({ messages }: { messages: AcpSessionMessage[] }) {
	const [collapsedThoughts, setCollapsedThoughts] = useState<Set<string>>(new Set());
	const knownThoughtIdsRef = useRef<Set<string>>(new Set());
	const orderedMessages = useMemo(() => [...messages].reverse(), [messages]);

	useLayoutEffect(() => {
		const thoughtIds = orderedMessages.flatMap((message) =>
			message.role !== "assistant"
				? []
				: normalizeAssistantBlocks(message.blocks).flatMap((block, index) =>
						block.type === "thought" ? [`${message.id}-thought-${index}`] : [],
					),
		);
		setCollapsedThoughts((previous) => {
			const activeThoughts = new Set(thoughtIds);
			const next = new Set<string>();

			for (const thoughtId of previous) {
				if (activeThoughts.has(thoughtId)) {
					next.add(thoughtId);
				}
			}

			for (const thoughtId of activeThoughts) {
				if (!knownThoughtIdsRef.current.has(thoughtId)) {
					next.add(thoughtId);
				}
			}

			knownThoughtIdsRef.current = activeThoughts;
			return next;
		});
	}, [orderedMessages]);

	const handleToggleThought = (id: string) => {
		setCollapsedThoughts((previous) => {
			const next = new Set(previous);
			if (next.has(id)) {
				next.delete(id);
			} else {
				next.add(id);
			}
			return next;
		});
	};

	return (
		<>
			{orderedMessages.map((message) => {
				if (message.role === "user") {
					return (
						<article key={message.id} className="session-message user">
							<header className="session-message-role">
								<span className="session-message-role-badge">你</span>
							</header>
							<div className="session-message-stack">
								<pre className="session-message-content">{getUserMessageContent(message)}</pre>
							</div>
						</article>
					);
				}

				if (message.role !== "assistant") {
					return null;
				}

				const mergedBlocks = normalizeAssistantBlocks(message.blocks);
				return (
					<article key={message.id} className="session-message assistant">
						<header className="session-message-role">
							<span className="session-message-role-badge">Agent</span>
						</header>
						<div className="session-message-stack">
							{mergedBlocks.length === 0 ? (
								<p className="session-message-content pending">…</p>
							) : (
								mergedBlocks.map((block, index) => {
									const blockId = `${message.id}-${block.type}-${index}`;
									switch (block.type) {
										case "thought":
											return (
												<ThoughtBlock
													key={blockId}
													id={blockId}
													content={block.content}
													collapsed={collapsedThoughts.has(blockId)}
													onToggle={() => handleToggleThought(blockId)}
												/>
											);
										case "actions":
											return <ActionBar key={blockId} actions={block.items} />;
										case "content":
											return (
												<MarkdownContent
													key={blockId}
													content={block.text}
													pending={message.pending}
												/>
											);
									}
								})
							)}
						</div>
					</article>
				);
			})}
		</>
	);
}
