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
}

export type ExecutionStatus = "success" | "warning" | "error";

export interface ExecutionResult {
	status: ExecutionStatus;
	primaryText: string | null;
	secondaryText: string | null;
	structuredPayload: Record<string, unknown> | null;
	nextActions: string[];
	shouldCloseLauncher: boolean;
}

export interface FloatingPanelOffset {
	x: number;
	y: number;
	width: number;
}
