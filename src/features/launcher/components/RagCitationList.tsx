import { useEffect, useMemo, useState, type MouseEvent, type FocusEvent } from "react";
import { createPortal } from "react-dom";
import type { RagCitation, RagRetrievalSummary } from "../types";

interface RagCitationListProps {
	citations: RagCitation[];
	retrieval: RagRetrievalSummary;
	onOpenCitation: (citation: RagCitation) => void | Promise<void>;
}

interface ActiveTooltip {
	citation: RagCitation;
	rect: DOMRect;
}

const tooltipHorizontalPadding = 12;
const tooltipWidth = 420;

function getFileNameFromPath(path: string): string {
	const segments = path.split(/[\\/]+/u).filter((segment) => segment.length > 0);
	if (segments.length === 0) {
		return path;
	}

	return segments[segments.length - 1] ?? path;
}

function formatCitationLocation(citation: RagCitation): string {
	if (citation.pageStart !== null) {
		if (citation.pageEnd !== null && citation.pageEnd !== citation.pageStart) {
			return `pages ${citation.pageStart}-${citation.pageEnd}`;
		}
		return `page ${citation.pageStart}`;
	}

	if (citation.lineStart !== null && citation.lineEnd !== null) {
		const paragraph =
			citation.paragraphLineStart !== null ? ` · paragraph ${citation.paragraphLineStart}` : "";
		return `lines ${citation.lineStart}-${citation.lineEnd}${paragraph}`;
	}

	return `chunk #${citation.chunkIndex}`;
}

export function RagCitationList({ citations, retrieval, onOpenCitation }: RagCitationListProps) {
	const [activeTooltip, setActiveTooltip] = useState<ActiveTooltip | null>(null);

	useEffect(() => {
		if (!activeTooltip) {
			return;
		}

		const updatePosition = () => {
			if (!activeTooltip) {
				return;
			}

			const nextTrigger = document.querySelector<HTMLElement>(
				`[data-rag-citation-key="${CSS.escape(
					`${activeTooltip.citation.absolutePath}#${activeTooltip.citation.chunkIndex}`,
				)}"]`,
			);
			if (!nextTrigger) {
				setActiveTooltip(null);
				return;
			}

			setActiveTooltip((current) => {
				if (!current) {
					return current;
				}

				return {
					...current,
					rect: nextTrigger.getBoundingClientRect(),
				};
			});
		};

		window.addEventListener("resize", updatePosition);
		window.addEventListener("scroll", updatePosition, true);

		return () => {
			window.removeEventListener("resize", updatePosition);
			window.removeEventListener("scroll", updatePosition, true);
		};
	}, [activeTooltip]);

	const tooltipLayout = useMemo(() => {
		if (!activeTooltip) {
			return null;
		}

		const maxWidth = Math.min(tooltipWidth, window.innerWidth - tooltipHorizontalPadding * 2);
		const left = Math.min(
			Math.max(activeTooltip.rect.left, tooltipHorizontalPadding),
			window.innerWidth - maxWidth - tooltipHorizontalPadding,
		);
		const top =
			activeTooltip.rect.top > 180 ? activeTooltip.rect.top - 10 : activeTooltip.rect.bottom + 10;
		const placement = activeTooltip.rect.top > 180 ? "top" : "bottom";

		return {
			left,
			maxWidth,
			top,
			placement,
		};
	}, [activeTooltip]);

	function showTooltip(
		event: MouseEvent<HTMLButtonElement> | FocusEvent<HTMLButtonElement>,
		citation: RagCitation,
	): void {
		setActiveTooltip({
			citation,
			rect: event.currentTarget.getBoundingClientRect(),
		});
	}

	function hideTooltip(): void {
		setActiveTooltip(null);
	}

	return (
		<>
			<section className="rag-citation-list">
				<div className="rag-citation-header">
					<p className="rag-citation-summary">
						<span className="rag-citation-title">References</span>
						<span>{retrieval.matchCount} 个片段</span>
						<span>·</span>
						<span>{retrieval.fileCount} 个文件</span>
					</p>
				</div>
				<ul className="rag-citation-items">
					{citations.map((citation) => {
						const fileName = getFileNameFromPath(citation.path);
						const citationKey = `${citation.absolutePath}#${citation.chunkIndex}`;
						return (
							<li className="rag-citation-entry" key={citationKey}>
								<button
									type="button"
									className="rag-citation-item"
									data-rag-citation-key={citationKey}
									onClick={() => void onOpenCitation(citation)}
									onFocus={(event) => showTooltip(event, citation)}
									onBlur={hideTooltip}
									onMouseEnter={(event) => showTooltip(event, citation)}
									onMouseLeave={hideTooltip}
								>
									<span className="rag-citation-index">[{citation.id}]</span>
									<span className="rag-citation-file">{fileName}</span>
								</button>
							</li>
						);
					})}
				</ul>
			</section>
			{activeTooltip && tooltipLayout
				? createPortal(
						<div
							className={`rag-citation-tooltip rag-citation-tooltip-${tooltipLayout.placement}`}
							role="tooltip"
							style={{
								left: `${tooltipLayout.left}px`,
								maxWidth: `${tooltipLayout.maxWidth}px`,
								top: `${tooltipLayout.top}px`,
							}}
						>
							<div className="rag-citation-tooltip-row">
								<span className="rag-citation-tooltip-label">文件</span>
								<span className="rag-citation-tooltip-value">{activeTooltip.citation.path}</span>
							</div>
							<div className="rag-citation-tooltip-row">
								<span className="rag-citation-tooltip-label">命中</span>
								<span className="rag-citation-tooltip-value">
									chunk #{activeTooltip.citation.chunkIndex} ·{" "}
									{formatCitationLocation(activeTooltip.citation)} · score{" "}
									{activeTooltip.citation.score.toFixed(3)}
								</span>
							</div>
							{activeTooltip.citation.anchorLabel ? (
								<div className="rag-citation-tooltip-row">
									<span className="rag-citation-tooltip-label">锚点</span>
									<span className="rag-citation-tooltip-value">
										{activeTooltip.citation.anchorLabel}
									</span>
								</div>
							) : null}
							{activeTooltip.citation.headingPath.length > 0 ? (
								<div className="rag-citation-tooltip-row">
									<span className="rag-citation-tooltip-label">标题</span>
									<span className="rag-citation-tooltip-value">
										{activeTooltip.citation.headingPath.join(" > ")}
									</span>
								</div>
							) : null}
							<div className="rag-citation-tooltip-row">
								<span className="rag-citation-tooltip-label">绝对路径</span>
								<span className="rag-citation-tooltip-value">
									{activeTooltip.citation.absolutePath}
								</span>
							</div>
							<div className="rag-citation-tooltip-snippet">{activeTooltip.citation.snippet}</div>
						</div>,
						document.body,
					)
				: null}
		</>
	);
}
