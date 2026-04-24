import type { InputMode } from "./types";

export type ShortcutTranslationSourceMode = "ocr" | "selection";

export interface ShortcutTranslationInputState {
	inputMode: InputMode;
	rawText: string;
	caretIndex: number;
}

export function deriveShortcutTranslationInputState(
	sourceText: string,
	sourceMode: ShortcutTranslationSourceMode,
): ShortcutTranslationInputState {
	return {
		inputMode: sourceMode,
		rawText: sourceText,
		caretIndex: 0,
	};
}
