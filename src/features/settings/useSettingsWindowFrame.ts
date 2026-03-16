import { currentMonitor } from "@tauri-apps/api/window";
import { type RefObject, useEffect, useLayoutEffect, useState } from "react";
import { isDesktopRuntimeAvailable, resizeLauncherWindow } from "../../lib/tauri/client";

const preferredSettingsFrameHeight = 720;
const minimumSettingsFrameHeight = 320;
const monitorViewportMargin = 72;

function measureRootSize(rootElement: HTMLElement) {
	const rect = rootElement.getBoundingClientRect();

	return {
		width: Math.ceil(rect.width),
		height: Math.ceil(rect.height),
	};
}

function clampSettingsFrameHeight(viewportHeight: number) {
	const roundedViewportHeight = Math.max(1, Math.floor(viewportHeight));
	const boundedHeight = Math.max(
		minimumSettingsFrameHeight,
		roundedViewportHeight - monitorViewportMargin,
	);

	return Math.min(preferredSettingsFrameHeight, boundedHeight, roundedViewportHeight);
}

async function resolveViewportHeight() {
	const monitor = await currentMonitor();
	if (monitor) {
		return monitor.workArea.size.height / monitor.scaleFactor;
	}

	return window.screen.availHeight || window.innerHeight;
}

export function useSettingsWindowFrame(rootRef: RefObject<HTMLElement | null>) {
	const desktopRuntimeAvailable = isDesktopRuntimeAvailable();
	const [frameHeight, setFrameHeight] = useState(() => {
		if (typeof window === "undefined") {
			return preferredSettingsFrameHeight;
		}

		return clampSettingsFrameHeight(
			window.screen.availHeight || window.innerHeight || preferredSettingsFrameHeight,
		);
	});

	useEffect(() => {
		if (!desktopRuntimeAvailable) {
			return;
		}

		let cancelled = false;

		const syncFrameHeight = async () => {
			try {
				const viewportHeight = await resolveViewportHeight();
				if (cancelled) {
					return;
				}

				const nextFrameHeight = clampSettingsFrameHeight(viewportHeight);
				setFrameHeight((currentHeight) =>
					currentHeight === nextFrameHeight ? currentHeight : nextFrameHeight,
				);
			} catch (error: unknown) {
				console.warn("failed to resolve settings monitor height", error);
			}
		};

		void syncFrameHeight();

		const handleResize = () => {
			void syncFrameHeight();
		};

		window.addEventListener("resize", handleResize);

		return () => {
			cancelled = true;
			window.removeEventListener("resize", handleResize);
		};
	}, [desktopRuntimeAvailable]);

	useLayoutEffect(() => {
		if (!desktopRuntimeAvailable) {
			return;
		}

		const rootElement = rootRef.current;
		if (!rootElement) {
			return;
		}

		let animationFrameId = 0;

		animationFrameId = requestAnimationFrame(() => {
			void resizeLauncherWindow(measureRootSize(rootElement)).catch((resizeError: unknown) => {
				console.warn("failed to resize settings window", resizeError);
			});
		});

		return () => {
			cancelAnimationFrame(animationFrameId);
		};
	}, [desktopRuntimeAvailable, frameHeight, rootRef]);

	return frameHeight;
}
