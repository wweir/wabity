import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { AcpActionEvent, AcpMessageBlock, AcpSessionMessage } from "../../../lib/tauri/types";
import { MarkdownRenderer } from "./MarkdownRenderer";
import { ThoughtDisclosure } from "./ThoughtDisclosure";

type ActionKind =
	| "tool-call"
	| "tool-update"
	| "plan"
	| "mode"
	| "config"
	| "commands"
	| "info"
	| "error";

function isStepMarkerAction(action: AcpActionEvent) {
	return action.kind === "info" && /^第\s*\d+\s*步$/u.test(action.title.trim());
}

function getActionKindLabel(kind: ActionKind): string {
	switch (kind) {
		case "tool-call":
			return "调用";
		case "tool-update":
			return "结果";
		case "plan":
			return "计划";
		case "mode":
			return "模式";
		case "config":
			return "配置";
		case "commands":
			return "命令";
		case "info":
			return "信息";
		case "error":
			return "错误";
		default:
			return "事件";
	}
}

function getActionPrefix(kind: ActionKind) {
	switch (kind) {
		case "tool-call":
		case "tool-update":
			return null;
		default:
			return getActionKindLabel(kind);
	}
}

function normalizeActionKind(kind: string): ActionKind {
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

function getActionTitle(action: AcpActionEvent, kind: ActionKind) {
	if (action.title.trim().length > 0) {
		return action.title;
	}

	return getActionKindLabel(kind);
}

function createActionId(action: AcpActionEvent, index: number) {
	const kind = normalizeActionKind(action.kind);
	const correlationId = action.correlationId ?? null;
	return correlationId
		? `${kind}:${correlationId}:${index}`
		: `${kind}:${action.title.trim() || "untitled"}:${index}`;
}

function ActionTrail({ actions }: { actions: AcpActionEvent[] }) {
	const [expandedActionId, setExpandedActionId] = useState<string | null>(null);
	const actionIds = useMemo(
		() => actions.map((action, index) => createActionId(action, index)),
		[actions],
	);

	useEffect(() => {
		if (expandedActionId && !actionIds.includes(expandedActionId)) {
			setExpandedActionId(null);
		}
	}, [actionIds, expandedActionId]);

	if (actions.length === 0) {
		return null;
	}

	return (
		<div className="action-trail" role="group" aria-label="执行轨迹">
			<ol className="action-trail-list">
				{actions.map((action, index) => {
					const id = actionIds[index];
					const kind = normalizeActionKind(action.kind);
					const prefix = getActionPrefix(kind);
					const title = getActionTitle(action, kind);
					const detail = formatActionDetailForDisplay(action.detail);
					const detailPanelId = detail ? `action-detail-${id}` : undefined;
					const expanded = expandedActionId === id;

					if (isStepMarkerAction(action)) {
						return (
							<li key={id} className="action-trail-step">
								{title}
							</li>
						);
					}

					return (
						<li key={id} className={`action-entry ${kind}`}>
							{detail ? (
								<button
									type="button"
									className={`action-entry-toggle${expanded ? " detail-open" : ""}`}
									aria-expanded={expanded}
									aria-controls={expanded ? detailPanelId : undefined}
									aria-label={`${expanded ? "收起" : "展开"} ${title} 详情`}
									onClick={() => setExpandedActionId(expanded ? null : id)}
								>
									<span className="action-entry-copy">
										{prefix ? <span className="action-entry-kind">{prefix}</span> : null}
										<span className="action-entry-title">{title}</span>
									</span>
									<span className="action-entry-disclosure">{expanded ? "▾" : "▸"}</span>
								</button>
							) : (
								<div className="action-entry-body">
									<span className="action-entry-copy">
										{prefix ? <span className="action-entry-kind">{prefix}</span> : null}
										<span className="action-entry-title">{title}</span>
									</span>
								</div>
							)}
							{detail && expanded ? (
								<div
									className="action-entry-detail"
									id={detailPanelId}
									role="region"
									aria-label={`${title} 详情`}
								>
									<pre>{detail}</pre>
								</div>
							) : null}
						</li>
					);
				})}
			</ol>
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
				: message.blocks.flatMap((block, index) =>
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

				return (
					<article
						key={message.id}
						className="session-message assistant"
						data-session-pending={message.pending ? "true" : undefined}
					>
						<header className="session-message-role">
							<span className="session-message-role-badge">Agent</span>
						</header>
						<div className="session-message-stack">
							{message.blocks.length === 0 ? (
								<p className="session-message-content pending">…</p>
							) : (
								message.blocks.map((block, index) => {
									const blockId = `${message.id}-${block.type}-${index}`;
									switch (block.type) {
										case "thought":
											return (
												<ThoughtDisclosure
													key={blockId}
													content={block.content}
													contentId={`${blockId}-content`}
													collapsed={collapsedThoughts.has(blockId)}
													onToggle={() => handleToggleThought(blockId)}
												/>
											);
										case "actions":
											return <ActionTrail key={blockId} actions={block.items} />;
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
