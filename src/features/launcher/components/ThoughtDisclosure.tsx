interface ThoughtDisclosureProps {
	content: string;
	contentId: string;
	collapsed: boolean;
	onToggle: () => void;
	className?: string;
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
		return `${preferred.slice(0, softStopIndex).trim()}...`;
	}

	return `${preferred.trimEnd()}...`;
}

function splitThoughtParagraphs(content: string) {
	return content
		.replace(/\r\n?/g, "\n")
		.split(/\n{2,}/)
		.map((paragraph) => paragraph.trim())
		.filter((paragraph) => paragraph.length > 0);
}

export function ThoughtDisclosure({
	content,
	contentId,
	collapsed,
	onToggle,
	className,
}: ThoughtDisclosureProps) {
	const summary = collapsed ? summarizeThoughtPreview(content).trim() : null;
	const paragraphs = splitThoughtParagraphs(content);

	return (
		<div
			className={
				className ? `message-block thought-block ${className}` : "message-block thought-block"
			}
		>
			<button
				aria-controls={contentId}
				aria-expanded={!collapsed}
				aria-label={collapsed ? "展开辅助思路" : "隐藏辅助思路"}
				className="thought-toggle"
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
			{!collapsed ? (
				<div className="thought-content" id={contentId} role="region" aria-label="思路详情">
					{paragraphs.map((paragraph, index) => (
						<p key={`${contentId}-paragraph-${index}`} className="thought-paragraph">
							{paragraph}
						</p>
					))}
				</div>
			) : null}
		</div>
	);
}
