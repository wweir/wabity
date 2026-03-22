import { type RefObject, useLayoutEffect } from "react";
import { isDesktopRuntimeAvailable, resizeLauncherWindow } from "./client";

const WINDOW_RESIZE_SETTLE_MS = 80;

function measureContentSize(rootElement: HTMLElement) {
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

export function useAutoResizeWindow(rootRef: RefObject<HTMLElement | null>) {
	const desktopRuntimeAvailable = isDesktopRuntimeAvailable();

	useLayoutEffect(() => {
		if (!desktopRuntimeAvailable) {
			return;
		}

		const rootElement = rootRef.current;
		if (!rootElement) {
			return;
		}

		let animationFrameId = 0;
		let settleTimeoutId: number | null = null;
		let lastMeasuredSize: { width: number; height: number } | null = null;
		let resizeInFlight = false;
		let pendingSync = false;

		// Tauri window resizing feeds back into DOM/layout observers. Coalescing that loop
		// avoids hot resize churn when large translate/QA results land at once.
		const unlockResizeLoop = () => {
			if (settleTimeoutId !== null) {
				window.clearTimeout(settleTimeoutId);
			}

			settleTimeoutId = window.setTimeout(() => {
				resizeInFlight = false;
				settleTimeoutId = null;
				if (pendingSync) {
					pendingSync = false;
					syncWindowSize();
				}
			}, WINDOW_RESIZE_SETTLE_MS);
		};

		const syncWindowSize = () => {
			cancelAnimationFrame(animationFrameId);
			animationFrameId = requestAnimationFrame(() => {
				if (resizeInFlight) {
					pendingSync = true;
					return;
				}

				const nextSize = measureContentSize(rootElement);
				if (sameMeasuredSize(lastMeasuredSize, nextSize)) {
					return;
				}

				lastMeasuredSize = nextSize;
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
			resizeObserver?.disconnect();
			mutationObserver?.disconnect();
			window.removeEventListener("resize", syncWindowSize);
		};
	}, [desktopRuntimeAvailable, rootRef]);
}
