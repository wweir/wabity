import { useEffect, useMemo, useState } from "react";
import { cancelScreenCaptureRegion, completeScreenCaptureRegion } from "../lib/tauri/client";
import "./screenCaptureOverlay.css";

const MIN_SELECTION_SIZE = 1;

interface Point {
	x: number;
	y: number;
}

interface SelectionRect {
	x: number;
	y: number;
	width: number;
	height: number;
}

function tokenFromLocation(): string {
	return new URLSearchParams(window.location.search).get("token")?.trim() ?? "";
}

function normalizeSelection(start: Point, end: Point): SelectionRect {
	const x = Math.min(start.x, end.x);
	const y = Math.min(start.y, end.y);
	return {
		x,
		y,
		width: Math.abs(end.x - start.x),
		height: Math.abs(end.y - start.y),
	};
}

function canSubmitSelection(selection: SelectionRect | null): selection is SelectionRect {
	return (
		selection !== null &&
		selection.width >= MIN_SELECTION_SIZE &&
		selection.height >= MIN_SELECTION_SIZE
	);
}

function clamp(value: number, min: number, max: number): number {
	return Math.min(Math.max(value, min), max);
}

export function ScreenCaptureOverlay() {
	const token = useMemo(tokenFromLocation, []);
	const [dragStart, setDragStart] = useState<Point | null>(null);
	const [dragEnd, setDragEnd] = useState<Point | null>(null);
	const [submitting, setSubmitting] = useState(false);
	const selection = dragStart && dragEnd ? normalizeSelection(dragStart, dragEnd) : null;

	useEffect(() => {
		function handleKeyDown(event: KeyboardEvent) {
			if (event.key !== "Escape" || submitting || !token) {
				return;
			}
			event.preventDefault();
			setSubmitting(true);
			void cancelScreenCaptureRegion(token).catch((error: unknown) => {
				console.warn("failed to cancel screen capture region", error);
			});
		}

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [submitting, token]);

	function pointFromPointer(event: React.PointerEvent): Point {
		return {
			x: clamp(event.clientX, 0, window.innerWidth),
			y: clamp(event.clientY, 0, window.innerHeight),
		};
	}

	function handlePointerDown(event: React.PointerEvent<HTMLDivElement>) {
		if (submitting || event.button !== 0 || !token) {
			return;
		}
		const point = pointFromPointer(event);
		setDragStart(point);
		setDragEnd(point);
		event.currentTarget.setPointerCapture(event.pointerId);
	}

	function handlePointerMove(event: React.PointerEvent<HTMLDivElement>) {
		if (submitting || !dragStart) {
			return;
		}
		setDragEnd(pointFromPointer(event));
	}

	function handlePointerUp(event: React.PointerEvent<HTMLDivElement>) {
		if (event.currentTarget.hasPointerCapture(event.pointerId)) {
			event.currentTarget.releasePointerCapture(event.pointerId);
		}
		if (submitting || !dragStart || !dragEnd || !token) {
			return;
		}
		const nextSelection = normalizeSelection(dragStart, pointFromPointer(event));
		if (!canSubmitSelection(nextSelection)) {
			setDragStart(null);
			setDragEnd(null);
			return;
		}

		setSubmitting(true);
		void completeScreenCaptureRegion(token, nextSelection).catch((error: unknown) => {
			setSubmitting(false);
			console.warn("failed to complete screen capture region", error);
		});
	}

	function handlePointerCancel(event: React.PointerEvent<HTMLDivElement>) {
		if (event.currentTarget.hasPointerCapture(event.pointerId)) {
			event.currentTarget.releasePointerCapture(event.pointerId);
		}
		setDragStart(null);
		setDragEnd(null);
	}

	return (
		<div
			className="screen-capture-overlay"
			onPointerDown={handlePointerDown}
			onPointerMove={handlePointerMove}
			onPointerUp={handlePointerUp}
			onPointerCancel={handlePointerCancel}
		>
			<div className="screen-capture-overlay-hint">拖拽选择截图区域 · Esc 取消</div>
			{canSubmitSelection(selection) ? (
				<div
					className="screen-capture-selection"
					style={{
						left: selection.x,
						top: selection.y,
						width: selection.width,
						height: selection.height,
					}}
				>
					<span>
						{Math.round(selection.width)} × {Math.round(selection.height)}
					</span>
				</div>
			) : null}
		</div>
	);
}
