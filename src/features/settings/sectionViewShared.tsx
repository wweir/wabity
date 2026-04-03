import type { ReactElement } from "react";
import type { AcpFieldKey, LlmFieldKey, McpFieldKey, RagFieldKey } from "./settingsTypes";

export type BindSectionBlockRef = (blockId: string) => (element: HTMLElement | null) => void;
export type BindAcpFieldRef = (
	scope: "agent" | "server",
	id: string,
	field: McpFieldKey | AcpFieldKey,
) => (node: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement | null) => void;
export type BindLlmFieldRef = (
	id: string,
	field: LlmFieldKey,
) => (node: HTMLInputElement | null) => void;
export type BindRagFieldRef = (
	field: RagFieldKey,
) => (node: HTMLSelectElement | HTMLTextAreaElement | null) => void;

export function renderFieldError(issueId: string, issue?: string): ReactElement | null {
	if (!issue) {
		return null;
	}

	return (
		<span className="settings-field-error" id={issueId}>
			{issue}
		</span>
	);
}

export function renderIssueList(issues: readonly string[]): ReactElement | null {
	if (issues.length === 0) {
		return null;
	}

	return (
		<ul className="settings-issue-list">
			{issues.map((issue) => (
				<li key={issue}>{issue}</li>
			))}
		</ul>
	);
}

export function renderValidationIssueBox(issues: readonly string[]): ReactElement | null {
	const issueList = renderIssueList(issues);
	if (!issueList) {
		return null;
	}

	return <div className="settings-validation-box settings-banner-error">{issueList}</div>;
}
