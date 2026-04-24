import test from "node:test";
import assert from "node:assert/strict";
import { deriveShortcutTranslationInputState } from "../src/features/launcher/shortcutTranslation.ts";

test("keeps shortcut selection text in launcher input and places caret at start", () => {
	const result = deriveShortcutTranslationInputState("selected text", "selection");

	assert.equal(result.inputMode, "selection");
	assert.equal(result.rawText, "selected text");
	assert.equal(result.caretIndex, 0);
});

test("keeps OCR text in launcher input with multiline mode semantics", () => {
	const result = deriveShortcutTranslationInputState("line 1\nline 2", "ocr");

	assert.equal(result.inputMode, "ocr");
	assert.equal(result.rawText, "line 1\nline 2");
	assert.equal(result.caretIndex, 0);
});
