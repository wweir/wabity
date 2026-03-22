export const launcherFrameMinWidth = 440;
export const launcherFrameMaxWidth = 920;
export const multilineInputMinHeight = 132;
export const completionPanelMaxWidth = 360;
export const completionPanelVerticalGap = 8;
const fallbackAvailableScreenHeight = 900;

let measurementCanvas: HTMLCanvasElement | null = null;

export function readPx(value: string) {
	const parsed = Number.parseFloat(value);
	return Number.isFinite(parsed) ? parsed : 0;
}

export function clamp(value: number, min: number, max: number) {
	return Math.min(Math.max(value, min), max);
}

export function getAvailableScreenHeight() {
	if (typeof window === "undefined") {
		return fallbackAvailableScreenHeight;
	}

	return window.screen?.availHeight || window.innerHeight || fallbackAvailableScreenHeight;
}

export function resolveLauncherScrollableHeights() {
	const availableScreenHeight = getAvailableScreenHeight();

	return {
		inputMaxHeight: clamp(Math.floor(availableScreenHeight * 0.26), multilineInputMinHeight, 280),
		outputMaxHeight: clamp(Math.floor(availableScreenHeight * 0.4), 220, 420),
	};
}

export function clampCaretIndex(rawText: string, index: number) {
	return clamp(index, 0, rawText.length);
}

export function longestVisibleLine(value: string, placeholder: string) {
	const lines = (value || placeholder).split(/\r?\n/).map((line) => line.trimEnd());
	return lines.reduce(
		(longest, line) => (line.length > longest.length ? line : longest),
		lines[0] ?? "",
	);
}

export function measureTextWidth(text: string, styles: CSSStyleDeclaration) {
	if (typeof document === "undefined") {
		return text.length * readPx(styles.fontSize);
	}

	measurementCanvas ??= document.createElement("canvas");
	const context = measurementCanvas.getContext("2d");
	if (!context) {
		return text.length * readPx(styles.fontSize);
	}

	context.font =
		styles.font ||
		`${styles.fontStyle} ${styles.fontVariant} ${styles.fontWeight} ${styles.fontSize} / ${styles.lineHeight} ${styles.fontFamily}`;
	return context.measureText(text).width;
}

export function measureCaretPosition(inputElement: HTMLTextAreaElement, caretIndex: number) {
	if (typeof document === "undefined") {
		return {
			left: 0,
			top: 0,
			lineHeight: readPx(getComputedStyle(inputElement).lineHeight),
		};
	}

	const styles = getComputedStyle(inputElement);
	const mirror = document.createElement("div");
	const marker = document.createElement("span");
	const styleNames = [
		"boxSizing",
		"width",
		"fontFamily",
		"fontSize",
		"fontStretch",
		"fontStyle",
		"fontVariant",
		"fontWeight",
		"letterSpacing",
		"lineHeight",
		"paddingTop",
		"paddingRight",
		"paddingBottom",
		"paddingLeft",
		"borderTopWidth",
		"borderRightWidth",
		"borderBottomWidth",
		"borderLeftWidth",
		"textTransform",
		"textIndent",
		"tabSize",
		"whiteSpace",
		"wordBreak",
		"overflowWrap",
	] as const;

	mirror.style.position = "absolute";
	mirror.style.visibility = "hidden";
	mirror.style.pointerEvents = "none";
	mirror.style.top = "0";
	mirror.style.left = "0";
	mirror.style.whiteSpace = "pre-wrap";
	mirror.style.wordBreak = "break-word";
	mirror.style.overflowWrap = "break-word";

	for (const styleName of styleNames) {
		mirror.style[styleName] = styles[styleName];
	}

	mirror.textContent = inputElement.value.slice(0, caretIndex);
	marker.textContent = inputElement.value.slice(caretIndex, caretIndex + 1) || "\u200b";
	mirror.appendChild(marker);
	document.body.appendChild(mirror);

	const left = marker.offsetLeft - inputElement.scrollLeft;
	const top = marker.offsetTop - inputElement.scrollTop;
	const lineHeight = readPx(styles.lineHeight) || readPx(styles.fontSize) * 1.35;

	document.body.removeChild(mirror);

	return {
		left,
		top,
		lineHeight,
	};
}
