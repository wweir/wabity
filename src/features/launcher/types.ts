import type { AcpActionEvent } from "../../lib/tauri/types";

export type InputMode = "inline" | "multiline" | "ocr" | "clipboard" | "selection";

export interface QueryPayload {
	mode: InputMode;
	rawText: string;
	segments: string[];
	language: string | null;
	sourceMetadata: SourceMetadata;
}

export interface SourceMetadata {
	createdAtMs: number;
	sourceHint: string;
}

export interface ActionDescriptor {
	id: string;
	title: string;
	summary: string;
	aliases: string[];
	keywords: string[];
	supportedInputModes: InputMode[];
	category: string;
	priority: number;
}

export interface ActionMatch {
	descriptor: ActionDescriptor;
	score: number;
}

export interface FileSearchMatch {
	path: string;
	fileName: string;
	parent: string;
	score: number;
}

export interface InstalledAppMatch {
	name: string;
	path: string;
	score: number;
}

export interface ExecutionRequest {
	actionId: string;
	query: QueryPayload;
	conversation?: ExecutionConversationTurn[];
	conversationState?: ExecutionConversationState | null;
}

export type ExecutionStatus = "success" | "warning" | "error";

export type ExecutionConversationRole = "user" | "assistant";

export interface ExecutionConversationTurn {
	role: ExecutionConversationRole;
	content: string;
}

export interface ExecutionConversationState {
	previousResponseId: string | null;
	continuationScope: string | null;
	citations: RagCitation[];
	actions: AcpActionEvent[];
	toolCalls: RagToolCall[];
}

export interface ExecutionResult {
	status: ExecutionStatus;
	primaryText: string | null;
	secondaryText: string | null;
	structuredPayload: Record<string, unknown> | null;
	nextActions: string[];
	shouldCloseLauncher: boolean;
}

export interface RagCitation {
	id: number;
	absolutePath: string;
	path: string;
	chunkIndex: number;
	lineStart: number;
	lineEnd: number;
	paragraphLineStart: number;
	headingPath: string[];
	score: number;
	distance: number;
	snippet: string;
}

export interface RagRetrievalSummary {
	query: string;
	matchCount: number;
	fileCount: number;
}

export interface RagToolCall {
	name: string;
	source: "builtin" | "mcp";
	status: "ok" | "error";
	summary: string;
}

export interface RagToolUsageSummary {
	available: string[];
	skipped: string[];
	calls: RagToolCall[];
}

export interface RagAnswerStructuredPayload {
	kind: "rag_answer";
	render: "markdown";
	responseId: string | null;
	conversationState?: ExecutionConversationState | null;
	citations: RagCitation[];
	retrieval: RagRetrievalSummary;
	actions: AcpActionEvent[];
	tools: RagToolUsageSummary;
}

export interface FloatingPanelOffset {
	x: number;
	y: number;
	width: number;
}

function isRagCitation(value: unknown): value is RagCitation {
	if (typeof value !== "object" || value === null) {
		return false;
	}

	const candidate = value as Partial<RagCitation>;
	return (
		typeof candidate.id === "number" &&
		typeof candidate.absolutePath === "string" &&
		typeof candidate.path === "string" &&
		typeof candidate.chunkIndex === "number" &&
		typeof candidate.lineStart === "number" &&
		typeof candidate.lineEnd === "number" &&
		typeof candidate.paragraphLineStart === "number" &&
		Array.isArray(candidate.headingPath) &&
		candidate.headingPath.every((item) => typeof item === "string") &&
		typeof candidate.score === "number" &&
		typeof candidate.distance === "number" &&
		typeof candidate.snippet === "string"
	);
}

function isRagToolCall(value: unknown): value is RagToolCall {
	if (typeof value !== "object" || value === null) {
		return false;
	}

	const candidate = value as Partial<RagToolCall>;
	return (
		typeof candidate.name === "string" &&
		(candidate.source === "builtin" || candidate.source === "mcp") &&
		(candidate.status === "ok" || candidate.status === "error") &&
		typeof candidate.summary === "string"
	);
}

function isAcpActionEvent(value: unknown): value is AcpActionEvent {
	if (typeof value !== "object" || value === null) {
		return false;
	}

	const candidate = value as Partial<AcpActionEvent>;
	return (
		typeof candidate.kind === "string" &&
		typeof candidate.title === "string" &&
		(candidate.correlationId === null || typeof candidate.correlationId === "string") &&
		(candidate.detail === null || typeof candidate.detail === "string")
	);
}

function isExecutionConversationState(value: unknown): value is ExecutionConversationState {
	if (typeof value !== "object" || value === null) {
		return false;
	}

	const candidate = value as Partial<ExecutionConversationState>;
	return (
		(candidate.previousResponseId === null || typeof candidate.previousResponseId === "string") &&
		(candidate.continuationScope === null || typeof candidate.continuationScope === "string") &&
		Array.isArray(candidate.citations) &&
		candidate.citations.every(isRagCitation) &&
		Array.isArray(candidate.actions) &&
		candidate.actions.every(isAcpActionEvent) &&
		Array.isArray(candidate.toolCalls) &&
		candidate.toolCalls.every(isRagToolCall)
	);
}

export function isRagAnswerStructuredPayload(
	payload: unknown,
): payload is RagAnswerStructuredPayload {
	if (typeof payload !== "object" || payload === null) {
		return false;
	}

	const candidate = payload as Partial<RagAnswerStructuredPayload>;
	if (candidate.kind !== "rag_answer" || candidate.render !== "markdown") {
		return false;
	}
	if (!(candidate.responseId === null || typeof candidate.responseId === "string")) {
		return false;
	}
	if (
		candidate.conversationState !== undefined &&
		candidate.conversationState !== null &&
		!isExecutionConversationState(candidate.conversationState)
	) {
		return false;
	}
	if (!Array.isArray(candidate.citations) || !candidate.citations.every(isRagCitation)) {
		return false;
	}

	const retrieval = candidate.retrieval as Partial<RagRetrievalSummary> | undefined;
	const tools = candidate.tools as Partial<RagToolUsageSummary> | undefined;
	if (!Array.isArray(candidate.actions) || !candidate.actions.every(isAcpActionEvent)) {
		return false;
	}
	return Boolean(
		retrieval &&
		typeof retrieval.query === "string" &&
		typeof retrieval.matchCount === "number" &&
		typeof retrieval.fileCount === "number" &&
		tools &&
		Array.isArray(tools.available) &&
		tools.available.every((tool) => typeof tool === "string") &&
		Array.isArray(tools.skipped) &&
		tools.skipped.every((tool) => typeof tool === "string") &&
		Array.isArray(tools.calls) &&
		tools.calls.every(isRagToolCall),
	);
}
