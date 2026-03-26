import { type RefObject, useEffect, useLayoutEffect, useState } from "react";
import type { FloatingPanelOffset } from "./types";

interface UseFloatingPanelOffsetOptions {
	open: boolean;
	triggerRef: RefObject<HTMLElement | null>;
	shellRef: RefObject<HTMLElement | null>;
	horizontalAlign: "left" | "right";
	gap: number;
	minWidth: number;
	extraWidth: number;
	maxWidth?: number;
	initialWidth: number;
	watchKey?: string | number;
}

function clamp(value: number, min: number, max: number) {
	return Math.min(Math.max(value, min), max);
}

function resolveAnchoredOffset({
	triggerRect,
	shellRect,
	horizontalAlign,
	gap,
	minWidth,
	extraWidth,
	maxWidth,
}: {
	triggerRect: DOMRect;
	shellRect: DOMRect;
	horizontalAlign: "left" | "right";
	gap: number;
	minWidth: number;
	extraWidth: number;
	maxWidth?: number;
}): FloatingPanelOffset {
	const unconstrainedWidth = Math.ceil(triggerRect.width + extraWidth);
	const desiredWidth =
		typeof maxWidth === "number"
			? clamp(unconstrainedWidth, minWidth, maxWidth)
			: Math.max(minWidth, unconstrainedWidth);
	const shellWidth = Math.floor(shellRect.width);
	const width =
		shellWidth > 0 ? clamp(desiredWidth, Math.min(minWidth, shellWidth), shellWidth) : desiredWidth;
	const maxX = Math.max(0, shellRect.width - width);
	const x =
		horizontalAlign === "right"
			? clamp(triggerRect.right - shellRect.left - width, 0, maxX)
			: clamp(triggerRect.left - shellRect.left, 0, maxX);

	return {
		x,
		y: Math.max(0, triggerRect.bottom - shellRect.top + gap),
		width,
	};
}

export function useFloatingPanelOffset({
	open,
	triggerRef,
	shellRef,
	horizontalAlign,
	gap,
	minWidth,
	extraWidth,
	maxWidth,
	initialWidth,
	watchKey,
}: UseFloatingPanelOffsetOptions): FloatingPanelOffset {
	const [offset, setOffset] = useState<FloatingPanelOffset>({
		x: 0,
		y: 0,
		width: initialWidth,
	});

	useLayoutEffect(() => {
		if (!open) {
			return;
		}

		const triggerElement = triggerRef.current;
		const shellElement = shellRef.current;
		if (!triggerElement || !shellElement) {
			return;
		}

		const nextOffset = resolveAnchoredOffset({
			triggerRect: triggerElement.getBoundingClientRect(),
			shellRect: shellElement.getBoundingClientRect(),
			horizontalAlign,
			gap,
			minWidth,
			extraWidth,
			maxWidth,
		});

		setOffset((currentOffset) =>
			currentOffset.x === nextOffset.x &&
			currentOffset.y === nextOffset.y &&
			currentOffset.width === nextOffset.width
				? currentOffset
				: nextOffset,
		);
	}, [open, triggerRef, shellRef, horizontalAlign, gap, minWidth, extraWidth, maxWidth, watchKey]);

	return offset;
}

interface UseDismissOnPointerDownOutsideOptions {
	open: boolean;
	triggerRef: RefObject<HTMLElement | null>;
	panelRef: RefObject<HTMLElement | null>;
	onDismiss: () => void;
}

export function useDismissOnPointerDownOutside({
	open,
	triggerRef,
	panelRef,
	onDismiss,
}: UseDismissOnPointerDownOutsideOptions) {
	useEffect(() => {
		if (!open || typeof document === "undefined") {
			return;
		}

		function handlePointerDown(event: MouseEvent) {
			const targetNode = event.target as Node;
			if (triggerRef.current?.contains(targetNode) || panelRef.current?.contains(targetNode)) {
				return;
			}

			onDismiss();
		}

		document.addEventListener("mousedown", handlePointerDown);
		return () => {
			document.removeEventListener("mousedown", handlePointerDown);
		};
	}, [open, panelRef, triggerRef, onDismiss]);
}
