import { currentMonitor } from "@tauri-apps/api/window";
import { type RefObject, useEffect, useLayoutEffect, useState } from "react";
import { isDesktopRuntimeAvailable, resizeLauncherWindow } from "../../lib/tauri/client";

const preferredSettingsFrameWidth = 920;
const preferredSettingsFrameHeight = 720;
const minimumSettingsFrameWidth = 360;
const minimumSettingsFrameHeight = 320;
const monitorViewportMargin = 72;

interface ViewportSize {
	width: number;
	height: number;
}

interface SettingsFrameSize {
	width: number;
	height: number;
}

function clampSettingsFrameDimension(
	viewportDimension: number,
	minimumDimension: number,
	preferredDimension: number,
) {
	const roundedViewportDimension = Math.max(1, Math.floor(viewportDimension));
	const boundedDimension = Math.max(
		minimumDimension,
		roundedViewportDimension - monitorViewportMargin,
	);

	return Math.min(preferredDimension, boundedDimension, roundedViewportDimension);
}

function clampSettingsFrameSize(viewportSize: ViewportSize): SettingsFrameSize {
	return {
		width: clampSettingsFrameDimension(
			viewportSize.width,
			minimumSettingsFrameWidth,
			preferredSettingsFrameWidth,
		),
		height: clampSettingsFrameDimension(
			viewportSize.height,
			minimumSettingsFrameHeight,
			preferredSettingsFrameHeight,
		),
	};
}

async function resolveViewportSize(desktopRuntimeAvailable: boolean): Promise<ViewportSize> {
	if (desktopRuntimeAvailable) {
		const monitor = await currentMonitor();
		if (monitor) {
			const scaleFactor = monitor.scaleFactor;

			return {
				width: monitor.workArea.size.width / scaleFactor,
				height: monitor.workArea.size.height / scaleFactor,
			};
		}
	}

	return {
		width: window.innerWidth || window.screen.availWidth || preferredSettingsFrameWidth,
		height: window.innerHeight || window.screen.availHeight || preferredSettingsFrameHeight,
	};
}

export function useSettingsWindowFrame(rootRef: RefObject<HTMLElement | null>) {
	const desktopRuntimeAvailable = isDesktopRuntimeAvailable();
	const [frameSize, setFrameSize] = useState<SettingsFrameSize>(() => {
		if (typeof window === "undefined") {
			return {
				width: preferredSettingsFrameWidth,
				height: preferredSettingsFrameHeight,
			};
		}

		return clampSettingsFrameSize({
			width: window.innerWidth || window.screen.availWidth || preferredSettingsFrameWidth,
			height: window.innerHeight || window.screen.availHeight || preferredSettingsFrameHeight,
		});
	});

	useEffect(() => {
		let cancelled = false;

		const syncFrameSize = async () => {
			try {
				const viewportSize = await resolveViewportSize(desktopRuntimeAvailable);
				if (cancelled) {
					return;
				}

				const nextFrameSize = clampSettingsFrameSize(viewportSize);
				setFrameSize((currentFrameSize) =>
					currentFrameSize.width === nextFrameSize.width &&
					currentFrameSize.height === nextFrameSize.height
						? currentFrameSize
						: nextFrameSize,
				);
			} catch (error: unknown) {
				console.warn("failed to resolve settings monitor size", error);
			}
		};

		void syncFrameSize();

		const handleResize = () => {
			void syncFrameSize();
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
			void resizeLauncherWindow(frameSize).catch((resizeError: unknown) => {
				console.warn("failed to resize settings window", resizeError);
			});
		});

		return () => {
			cancelAnimationFrame(animationFrameId);
		};
	}, [desktopRuntimeAvailable, frameSize, rootRef]);

	return frameSize;
}
