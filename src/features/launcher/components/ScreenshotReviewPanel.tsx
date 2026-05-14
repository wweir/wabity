import type { ScreenshotReviewPayload } from "../../../lib/tauri/types";

interface ScreenshotReviewPanelProps {
	review: ScreenshotReviewPayload;
	previewDataUrl: string | null;
	editedText: string;
	selectedBlockIds: string[];
	busy: boolean;
	copyFeedback: string | null;
	onEditedTextChange: (value: string) => void;
	onToggleBlock: (blockId: string) => void;
	onSelectAllBlocks: () => void;
	onClearBlockSelection: () => void;
	onUseSelectedBlocks: () => void;
	onTranslate: () => void;
	onCopy: () => void;
	onRetry: () => void;
	onCancel: () => void;
}

function ocrStatusLabel(status: ScreenshotReviewPayload["ocr"]["status"]) {
	switch (status) {
		case "success":
			return "OCR 已完成";
		case "empty":
			return "OCR 未识别到文本";
		case "failed":
			return "OCR 失败";
	}
}

function providerLabel(provider: ScreenshotReviewPayload["ocr"]["provider"]) {
	return provider === "llm_ocr" ? "LLM OCR" : "系统 OCR";
}

export function ScreenshotReviewPanel({
	review,
	previewDataUrl,
	editedText,
	selectedBlockIds,
	busy,
	copyFeedback,
	onEditedTextChange,
	onToggleBlock,
	onSelectAllBlocks,
	onClearBlockSelection,
	onUseSelectedBlocks,
	onTranslate,
	onCopy,
	onRetry,
	onCancel,
}: ScreenshotReviewPanelProps) {
	const blockCount = review.ocr.blocks.length;
	const selectedBlockCount = selectedBlockIds.length;
	const canUseText = editedText.trim().length > 0;
	const selectedBlockIdSet = new Set(selectedBlockIds);
	const actionDisabled = busy || !canUseText;

	return (
		<section className="screenshot-review-panel" aria-label="截图 OCR Review">
			<header className="screenshot-review-header">
				<div>
					<div className="screenshot-review-eyebrow">Screenshot Review</div>
					<h2>确认要翻译的截图文字</h2>
				</div>
				<div className={`screenshot-review-status screenshot-review-status-${review.ocr.status}`}>
					{ocrStatusLabel(review.ocr.status)}
				</div>
			</header>

			<div className="screenshot-review-meta">
				<span>{providerLabel(review.ocr.provider)}</span>
				<span>
					{review.capture.backend === "screen_capture_kit"
						? "macOS ScreenCaptureKit"
						: review.capture.backend}
				</span>
				{review.imageWidth > 0 && review.imageHeight > 0 ? (
					<span>
						{review.imageWidth}×{review.imageHeight}
					</span>
				) : null}
			</div>

			{review.ocr.provider === "llm_ocr" ? (
				<p className="screenshot-review-warning">
					当前使用 LLM OCR：截图已先发送到已配置的多模态 OCR 模型，Review 只负责确认和编辑识别文本。
				</p>
			) : null}

			{review.ocr.errorMessage ? (
				<p className="screenshot-review-error">{review.ocr.errorMessage}</p>
			) : null}

			<div className="screenshot-review-grid">
				<div className="screenshot-review-preview">
					{previewDataUrl ? (
						<img src={previewDataUrl} alt="截图预览" draggable={false} />
					) : (
						<div className="screenshot-review-preview-placeholder">正在加载截图预览…</div>
					)}
				</div>

				<div className="screenshot-review-editor">
					<label className="screenshot-review-label" htmlFor="screenshot-review-text">
						可编辑文本
					</label>
					<textarea
						id="screenshot-review-text"
						value={editedText}
						onChange={(event) => onEditedTextChange(event.currentTarget.value)}
						onKeyDown={(event) => {
							if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
								event.preventDefault();
								if (!actionDisabled) {
									onTranslate();
								}
							}
						}}
						placeholder="OCR 没有返回文本时，可以在这里手动输入要翻译的内容。"
					/>
					<div className="screenshot-review-editor-hint">
						这里是最终提交文本。未手动编辑前，选择 OCR 块会同步更新；手动编辑后以这里为准。
					</div>
				</div>
			</div>

			<div className="screenshot-review-blocks-header">
				<span>OCR 块 · {blockCount}</span>
				<div>
					<button type="button" onClick={onSelectAllBlocks} disabled={busy || blockCount === 0}>
						全选
					</button>
					<button
						type="button"
						onClick={onUseSelectedBlocks}
						disabled={busy || selectedBlockCount === 0}
					>
						填入选中
					</button>
					<button
						type="button"
						onClick={onClearBlockSelection}
						disabled={busy || selectedBlockCount === 0}
					>
						清空
					</button>
				</div>
			</div>

			{blockCount > 0 ? (
				<div className="screenshot-review-blocks" role="list">
					{review.ocr.blocks.map((block) => {
						const selected = selectedBlockIdSet.has(block.id);
						return (
							<button
								key={block.id}
								type="button"
								className={selected ? "selected" : undefined}
								onClick={() => onToggleBlock(block.id)}
								disabled={busy}
								role="listitem"
							>
								<span>{block.text}</span>
								{typeof block.confidence === "number" ? (
									<small>{Math.round(block.confidence * 100)}%</small>
								) : null}
							</button>
						);
					})}
				</div>
			) : (
				<p className="screenshot-review-empty-blocks">
					没有可选 OCR 块。可以重新截图，或直接编辑文本。
				</p>
			)}

			<footer className="screenshot-review-actions">
				<div className="screenshot-review-copy-feedback" aria-live="polite">
					{copyFeedback}
				</div>
				<button type="button" onClick={onCancel} disabled={busy}>
					取消
				</button>
				<button type="button" onClick={onRetry} disabled={busy}>
					重新截图
				</button>
				<button type="button" onClick={onCopy} disabled={actionDisabled}>
					复制文本
				</button>
				<button
					type="button"
					className="screenshot-review-primary-action"
					onClick={onTranslate}
					disabled={actionDisabled}
				>
					确认翻译
				</button>
			</footer>
		</section>
	);
}
