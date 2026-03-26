import type { InputMode } from "./types";

export function usesInlineInputControl(mode: InputMode) {
	return mode === "inline";
}

export function usesTextareaInputControl(mode: InputMode) {
	return !usesInlineInputControl(mode);
}
