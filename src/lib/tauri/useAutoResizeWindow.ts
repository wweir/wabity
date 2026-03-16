import { type RefObject, useLayoutEffect } from "react";
import { isDesktopRuntimeAvailable, resizeLauncherWindow } from "./client";

function measureContentSize(rootElement: HTMLElement) {
	const rootRect = rootElement.getBoundingClientRect();
	let minLeft = rootRect.left;
	let minTop = rootRect.top;
	let maxRight = rootRect.right;
	let maxBottom = rootRect.bottom;

	for (const element of rootElement.querySelectorAll<HTMLElement>("*")) {
		const rect = element.getBoundingClientRect();
		if (rect.width === 0 && rect.height === 0) {
			continue;
		}

		minLeft = Math.min(minLeft, rect.left);
		minTop = Math.min(minTop, rect.top);
		maxRight = Math.max(maxRight, rect.right);
		maxBottom = Math.max(maxBottom, rect.bottom);
	}

	return {
		width: Math.ceil(Math.max(rootElement.scrollWidth, maxRight - minLeft)),
		height: Math.ceil(Math.max(rootElement.scrollHeight, maxBottom - minTop)),
	};
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

		const syncWindowSize = () => {
			cancelAnimationFrame(animationFrameId);
			animationFrameId = requestAnimationFrame(() => {
				void resizeLauncherWindow(measureContentSize(rootElement)).catch((resizeError: unknown) => {
					console.warn("failed to resize window to match page content", resizeError);
				});
			});
		};

		syncWindowSize();

		const resizeObserver =
			typeof ResizeObserver === "function" ? new ResizeObserver(syncWindowSize) : null;
		resizeObserver?.observe(rootElement);

		const mutationObserver =
			typeof MutationObserver === "function" ? new MutationObserver(syncWindowSize) : null;
		mutationObserver?.observe(rootElement, {
			subtree: true,
			childList: true,
			characterData: true,
			attributes: true,
		});

		window.addEventListener("resize", syncWindowSize);

		return () => {
			cancelAnimationFrame(animationFrameId);
			resizeObserver?.disconnect();
			mutationObserver?.disconnect();
			window.removeEventListener("resize", syncWindowSize);
		};
	}, [desktopRuntimeAvailable, rootRef]);
}
