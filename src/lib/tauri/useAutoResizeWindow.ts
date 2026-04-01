import { type RefObject, useLayoutEffect, useRef } from "react";
import {
	beginTransientWindowInteraction,
	endTransientWindowInteraction,
	isDesktopRuntimeAvailable,
	resizeLauncherWindow,
} from "./client";

const WINDOW_RESIZE_SETTLE_MS = 80;

export function measureContentSize(rootElement: HTMLElement) {
	const rootRect = rootElement.getBoundingClientRect();
	let minLeft = 0;
	let minTop = 0;
	let maxRight = Math.max(rootElement.clientWidth, rootElement.scrollWidth);
	let maxBottom = Math.max(rootElement.clientHeight, rootElement.scrollHeight);

	for (const element of rootElement.children) {
		if (!(element instanceof HTMLElement)) {
			continue;
		}

		const rect = element.getBoundingClientRect();
		if (rect.width === 0 && rect.height === 0) {
			continue;
		}

		const left = rect.left - rootRect.left;
		const top = rect.top - rootRect.top;
		const right = rect.right - rootRect.left;
		const bottom = rect.bottom - rootRect.top;

		minLeft = Math.min(minLeft, left);
		minTop = Math.min(minTop, top);
		maxRight = Math.max(maxRight, right, left + element.scrollWidth);
		maxBottom = Math.max(maxBottom, bottom, top + element.scrollHeight);
	}

	return {
		width: Math.ceil(maxRight - minLeft),
		height: Math.ceil(maxBottom - minTop),
	};
}

function sameMeasuredSize(
	left: { width: number; height: number } | null,
	right: { width: number; height: number },
) {
	return left?.width === right.width && left.height === right.height;
}

function normalizeMeasuredSize(
	lastMeasuredSize: { width: number; height: number } | null,
	nextMeasuredSize: { width: number; height: number },
	allowShrink: boolean,
) {
	if (allowShrink || !lastMeasuredSize) {
		return nextMeasuredSize;
	}

	return {
		width: Math.max(lastMeasuredSize.width, nextMeasuredSize.width),
		height: Math.max(lastMeasuredSize.height, nextMeasuredSize.height),
	};
}

interface UseAutoResizeWindowOptions {
	enabled?: boolean;
	allowShrink?: boolean;
	onResizeSettled?: () => void;
	resetKey?: string | number;
}

export function useAutoResizeWindow(
	rootRef: RefObject<HTMLElement | null>,
	options: UseAutoResizeWindowOptions = {},
) {
	const desktopRuntimeAvailable = isDesktopRuntimeAvailable();
	const { enabled = true, allowShrink = true, onResizeSettled, resetKey } = options;
	const lastMeasuredSizeRef = useRef<{ width: number; height: number } | null>(null);
	const lastResetKeyRef = useRef<string | number | undefined>(resetKey);

	useLayoutEffect(() => {
		const shouldResetMeasuredSize = lastResetKeyRef.current !== resetKey;
		lastResetKeyRef.current = resetKey;
		if (shouldResetMeasuredSize) {
			lastMeasuredSizeRef.current = null;
		}

		if (!desktopRuntimeAvailable || !enabled) {
			return;
		}

		const rootElement = rootRef.current;
		if (!rootElement) {
			return;
		}

		let animationFrameId = 0;
		let settleTimeoutId: number | null = null;
		let lastMeasuredSize = lastMeasuredSizeRef.current;
		let resizeInFlight = false;
		let pendingSync = false;
		let transientInteractionActive = false;

		const beginResizeInteraction = () => {
			if (transientInteractionActive) {
				return;
			}

			transientInteractionActive = true;
			void beginTransientWindowInteraction().catch((error: unknown) => {
				console.warn("failed to begin transient interaction for auto resize", error);
			});
		};

		const endResizeInteraction = () => {
			if (!transientInteractionActive) {
				return;
			}

			transientInteractionActive = false;
			void endTransientWindowInteraction().catch((error: unknown) => {
				console.warn("failed to end transient interaction for auto resize", error);
			});
		};

		// Tauri window resizing feeds back into DOM/layout observers. Coalescing that loop
		// avoids hot resize churn when large translate/QA results land at once.
		const unlockResizeLoop = () => {
			if (settleTimeoutId !== null) {
				window.clearTimeout(settleTimeoutId);
			}

			settleTimeoutId = window.setTimeout(() => {
				resizeInFlight = false;
				settleTimeoutId = null;
				endResizeInteraction();
				if (pendingSync) {
					pendingSync = false;
					syncWindowSize();
					return;
				}

				onResizeSettled?.();
			}, WINDOW_RESIZE_SETTLE_MS);
		};

		const syncWindowSize = () => {
			cancelAnimationFrame(animationFrameId);
			animationFrameId = requestAnimationFrame(() => {
				if (resizeInFlight) {
					pendingSync = true;
					return;
				}

				const measuredSize = measureContentSize(rootElement);
				const nextSize = normalizeMeasuredSize(lastMeasuredSize, measuredSize, allowShrink);
				if (sameMeasuredSize(lastMeasuredSize, nextSize)) {
					return;
				}

				lastMeasuredSize = nextSize;
				lastMeasuredSizeRef.current = nextSize;
				beginResizeInteraction();
				resizeInFlight = true;
				void resizeLauncherWindow(nextSize)
					.catch((resizeError: unknown) => {
						console.warn("failed to resize window to match page content", resizeError);
					})
					.finally(() => {
						unlockResizeLoop();
					});
			});
		};

		const resizeObserver =
			typeof ResizeObserver === "function" ? new ResizeObserver(syncWindowSize) : null;
		const refreshObservedElements = () => {
			if (!resizeObserver) {
				return;
			}

			resizeObserver.disconnect();
			resizeObserver.observe(rootElement);
			for (const childElement of rootElement.children) {
				if (childElement instanceof HTMLElement) {
					resizeObserver.observe(childElement);
				}
			}
		};
		refreshObservedElements();

		const mutationObserver =
			typeof MutationObserver === "function"
				? new MutationObserver(() => {
						refreshObservedElements();
						syncWindowSize();
					})
				: null;
		mutationObserver?.observe(rootElement, { childList: true });

		window.addEventListener("resize", syncWindowSize);
		syncWindowSize();

		return () => {
			cancelAnimationFrame(animationFrameId);
			if (settleTimeoutId !== null) {
				window.clearTimeout(settleTimeoutId);
			}
			endResizeInteraction();
			resizeObserver?.disconnect();
			mutationObserver?.disconnect();
			window.removeEventListener("resize", syncWindowSize);
		};
	}, [allowShrink, desktopRuntimeAvailable, enabled, onResizeSettled, resetKey, rootRef]);
}
