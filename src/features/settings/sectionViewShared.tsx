import type { ReactElement } from "react";
import type { DependencyHealthItem, LlmFieldKey, McpFieldKey, RagFieldKey } from "./settingsTypes";

export type BindSectionBlockRef = (blockId: string) => (element: HTMLElement | null) => void;
export type BindMcpFieldRef = (
	scope: "server",
	id: string,
	field: McpFieldKey,
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

function getDependencyHealthStatusLabel(status: DependencyHealthItem["status"]) {
	switch (status) {
		case "ok":
			return "正常";
		case "warning":
			return "注意";
		case "info":
			return "说明";
	}
}

function getDependencyHealthStatusChipClass(status: DependencyHealthItem["status"]) {
	switch (status) {
		case "ok":
			return "settings-status-chip settings-status-chip-success";
		case "warning":
			return "settings-status-chip settings-status-chip-warn";
		case "info":
			return "settings-status-chip settings-status-chip-info";
	}
}

export function renderDependencyHealthList(items: readonly DependencyHealthItem[]): ReactElement {
	return (
		<div className="settings-dependency-health" aria-label="依赖健康摘要">
			{items.map((item) => (
				<div className="settings-dependency-health-item" data-status={item.status} key={item.label}>
					<div className="settings-dependency-health-title-row">
						<strong>{item.label}</strong>
						<span className={getDependencyHealthStatusChipClass(item.status)}>
							{getDependencyHealthStatusLabel(item.status)}
						</span>
					</div>
					<span className="settings-help-text settings-help-text-tight">{item.detail}</span>
				</div>
			))}
		</div>
	);
}
