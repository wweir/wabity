import type { ActionMatch, FileSearchMatch, InputMode, QueryPayload } from "./types";

export const fileSearchDebounceMs = 50;
export const appSearchDebounceMs = 80;
export const killSearchDebounceMs = 120;
export const jsonFormatCommandAliases = ["/format", "/fmt", "/json"] as const;
export const markdownCommandAliases = ["/md", "/markdown"] as const;
export const base64CommandAliases = ["/base64"] as const;

export type SuggestionMode = "none" | "file" | "action" | "app" | "kill";

export interface FileTokenMatch {
	start: number;
	end: number;
	needle: string;
}

export interface SlashActionInputMatch {
	alias: string;
	commandToken: string;
	content: string;
	contentStart: number;
}

export interface SlashCommandTokenRange {
	start: number;
	end: number;
}

function isFileTokenBoundary(character: string | undefined) {
	return character === undefined || /\s/u.test(character) || /[([{'"]/u.test(character);
}

function countMatchingCharacters(value: string, pattern: RegExp) {
	return Array.from(value).reduce(
		(count, character) => count + (pattern.test(character) ? 1 : 0),
		0,
	);
}

function isNonEnglishLetter(character: string) {
	return /\p{Letter}/u.test(character) && !/[A-Za-z]/u.test(character);
}

function normalizeForPrefix(value: string) {
	return value.trim().toLowerCase();
}

function clamp(value: number, min: number, max: number) {
	return Math.min(Math.max(value, min), max);
}

function findTrimmedContentStart(value: string, start: number) {
	let contentStart = clamp(start, 0, value.length);
	while (contentStart < value.length && /\s/u.test(value[contentStart] ?? "")) {
		contentStart += 1;
	}

	return contentStart;
}

export function parseSlashActionInput(rawText: string, aliases: readonly string[]) {
	const normalizedText = rawText.trimStart();
	if (!normalizedText.startsWith("/")) {
		return null;
	}

	const boundaryIndex = normalizedText.search(/\s/u);
	const rawCommandToken =
		boundaryIndex === -1 ? normalizedText : normalizedText.slice(0, boundaryIndex);
	const commandToken = rawCommandToken.toLowerCase();
	const matchedAlias = aliases.find((alias) => {
		const normalizedAlias = alias.toLowerCase();
		return (
			normalizedAlias === commandToken ||
			normalizedAlias.startsWith(commandToken) ||
			commandToken.startsWith(normalizedAlias)
		);
	});

	if (matchedAlias) {
		const normalizedAlias = matchedAlias.toLowerCase();
		const inlinePayload =
			boundaryIndex === -1 && commandToken.startsWith(normalizedAlias)
				? normalizedText.slice(matchedAlias.length).trim()
				: "";

		return {
			alias: matchedAlias,
			commandToken:
				boundaryIndex === -1 && commandToken.startsWith(normalizedAlias)
					? normalizedAlias
					: commandToken,
			content: boundaryIndex === -1 ? inlinePayload : normalizedText.slice(boundaryIndex).trim(),
			contentStart:
				boundaryIndex === -1
					? matchedAlias.length
					: findTrimmedContentStart(normalizedText, boundaryIndex),
		};
	}

	for (const alias of aliases) {
		const normalizedAlias = alias.toLowerCase();
		const longestSharedPrefixLength = Math.min(commandToken.length - 1, normalizedAlias.length);
		for (let prefixLength = longestSharedPrefixLength; prefixLength >= 2; prefixLength -= 1) {
			if (!commandToken.startsWith(normalizedAlias.slice(0, prefixLength))) {
				continue;
			}

			const inlinePayload = normalizedText.slice(prefixLength).trim();
			if (!inlinePayload) {
				continue;
			}

			return {
				alias,
				commandToken: normalizedAlias.slice(0, prefixLength),
				content: inlinePayload,
				contentStart: findTrimmedContentStart(normalizedText, prefixLength),
			};
		}
	}

	return null;
}

export function findSlashCommandTokenRange(rawText: string): SlashCommandTokenRange | null {
	const leadingWhitespaceLength = rawText.match(/^\s*/u)?.[0].length ?? 0;
	if (rawText[leadingWhitespaceLength] !== "/") {
		return null;
	}

	let tokenEnd = leadingWhitespaceLength;
	while (tokenEnd < rawText.length && !/\s/u.test(rawText[tokenEnd] ?? "")) {
		tokenEnd += 1;
	}

	return {
		start: leadingWhitespaceLength,
		end: tokenEnd,
	};
}

export function findSlashCommandPrefixRange(
	rawText: string,
	commandToken: string,
): SlashCommandTokenRange | null {
	const tokenRange = findSlashCommandTokenRange(rawText);
	if (!tokenRange) {
		return null;
	}

	return {
		start: tokenRange.start,
		end: Math.min(tokenRange.start + commandToken.length, tokenRange.end),
	};
}

export function buildQuery(mode: InputMode, rawText: string): QueryPayload {
	return {
		mode,
		rawText,
		segments: rawText
			.split(/\r?\n/)
			.map((segment) => segment.trimEnd())
			.filter((segment) => segment.length > 0),
		language: null,
		sourceMetadata: {
			createdAtMs: Date.now(),
			sourceHint: "launcher_ui",
		},
	};
}

export function deriveSlashActionPayload(
	rawText: string,
	caretIndex: number,
	aliases: readonly string[],
) {
	const slashInputMatch = parseSlashActionInput(rawText, aliases);
	if (!slashInputMatch) {
		return null;
	}

	const leadingWhitespaceLength = rawText.match(/^\s*/u)?.[0].length ?? 0;
	const normalizedText = rawText.slice(leadingWhitespaceLength);
	const normalizedCaretIndex = clamp(
		caretIndex - leadingWhitespaceLength,
		0,
		normalizedText.length,
	);

	return {
		value: slashInputMatch.content,
		caretIndex: clamp(
			normalizedCaretIndex - slashInputMatch.contentStart,
			0,
			slashInputMatch.content.length,
		),
	};
}

export function deriveSelectedSlashActionPayload(
	rawText: string,
	caretIndex: number,
	aliases: readonly string[],
) {
	const matchedPayload = deriveSlashActionPayload(rawText, caretIndex, aliases);
	if (matchedPayload) {
		return matchedPayload;
	}

	const normalizedText = rawText.trimStart();
	if (!normalizedText.startsWith("/")) {
		return null;
	}

	const primaryAlias = aliases.find((alias) => alias.startsWith("/"));
	if (!primaryAlias) {
		return null;
	}

	const boundaryIndex = normalizedText.search(/\s/u);
	const rawCommandToken =
		boundaryIndex === -1 ? normalizedText : normalizedText.slice(0, boundaryIndex);
	const normalizedAlias = primaryAlias.toLowerCase();
	const normalizedCommandToken = rawCommandToken.toLowerCase();
	let sharedPrefixLength = 0;
	const maxPrefixLength = Math.min(normalizedCommandToken.length, normalizedAlias.length);
	while (
		sharedPrefixLength < maxPrefixLength &&
		normalizedCommandToken[sharedPrefixLength] === normalizedAlias[sharedPrefixLength]
	) {
		sharedPrefixLength += 1;
	}

	if (sharedPrefixLength === 0) {
		return null;
	}

	const contentStart =
		boundaryIndex === -1
			? sharedPrefixLength
			: findTrimmedContentStart(normalizedText, boundaryIndex);
	const content =
		boundaryIndex === -1
			? normalizedText.slice(sharedPrefixLength).trim()
			: normalizedText.slice(boundaryIndex).trim();
	const leadingWhitespaceLength = rawText.match(/^\s*/u)?.[0].length ?? 0;
	const normalizedCaretIndex = clamp(
		caretIndex - leadingWhitespaceLength,
		0,
		normalizedText.length,
	);

	return {
		value: content,
		caretIndex: clamp(normalizedCaretIndex - contentStart, 0, content.length),
	};
}

export function findActiveFileToken(rawText: string, caretIndex: number): FileTokenMatch | null {
	const boundedCaretIndex = clamp(caretIndex, 0, rawText.length);
	const prefix = rawText.slice(0, boundedCaretIndex);
	let tokenStart = prefix.length;

	while (tokenStart > 0 && !/\s/u.test(prefix[tokenStart - 1] ?? "")) {
		tokenStart -= 1;
	}

	const token = prefix.slice(tokenStart);
	const atOffset = token.lastIndexOf("@");
	if (atOffset < 0) {
		return null;
	}

	const start = tokenStart + atOffset;
	const previousCharacter = start > 0 ? rawText[start - 1] : undefined;
	if (!isFileTokenBoundary(previousCharacter)) {
		return null;
	}

	return {
		start,
		end: boundedCaretIndex,
		needle: rawText.slice(start + 1, boundedCaretIndex).trim(),
	};
}

export function isFileSearchReady(query: string) {
	const normalized = query.trim();
	if (!normalized) {
		return false;
	}

	const nonEnglishLetterCount = Array.from(normalized).reduce(
		(count, character) => count + (isNonEnglishLetter(character) ? 1 : 0),
		0,
	);
	if (nonEnglishLetterCount >= 1) {
		return true;
	}

	const latinCount = countMatchingCharacters(normalized, /[A-Za-z]/u);
	if (latinCount >= 2) {
		return true;
	}

	return false;
}

export function isActionQuery(query: string) {
	const normalized = query.trim();
	if (!normalized) {
		return false;
	}

	if (normalized.startsWith("/")) {
		return true;
	}

	if (normalized.startsWith("http") || normalized.includes("{")) {
		return true;
	}

	return false;
}

export function isAppSearchReady(query: string) {
	const normalized = query.trim();
	if (!normalized) {
		return false;
	}

	const nonEnglishLetterCount = Array.from(normalized).reduce(
		(count, character) => count + (isNonEnglishLetter(character) ? 1 : 0),
		0,
	);
	if (nonEnglishLetterCount >= 1) {
		return true;
	}

	const latinCount = countMatchingCharacters(normalized, /[A-Za-z]/u);
	if (latinCount >= 2) {
		return true;
	}

	return false;
}

export function isKillSearchReady(query: string) {
	const normalized = query.trim();
	if (!normalized || normalized.startsWith("pid:")) {
		return false;
	}

	const nonEnglishLetterCount = Array.from(normalized).reduce(
		(count, character) => count + (isNonEnglishLetter(character) ? 1 : 0),
		0,
	);
	if (nonEnglishLetterCount >= 1) {
		return true;
	}

	const latinCount = countMatchingCharacters(normalized, /[A-Za-z]/u);
	if (latinCount >= 2) {
		return true;
	}

	const digitCount = countMatchingCharacters(normalized, /\d/u);
	return digitCount >= 2;
}

export function deriveActionCompletion(rawText: string, match: ActionMatch | undefined) {
	if (!match) {
		return null;
	}

	const needle = normalizeForPrefix(rawText);
	if (!needle) {
		return match.descriptor.aliases[0] ?? match.descriptor.title;
	}

	const alias = match.descriptor.aliases.find((item) => item.toLowerCase().startsWith(needle));
	if (alias) {
		return alias;
	}

	if (match.descriptor.title.toLowerCase().startsWith(needle)) {
		return match.descriptor.title;
	}

	return null;
}

export function parseJsonFormatCommand(rawText: string) {
	return parseSlashActionInput(rawText, jsonFormatCommandAliases);
}

export function parseBase64Command(rawText: string) {
	return parseSlashActionInput(rawText, base64CommandAliases);
}

export function parseMarkdownCommand(rawText: string) {
	return parseSlashActionInput(rawText, markdownCommandAliases);
}

export function deriveJsonPrettyPreview(rawText: string) {
	const commandMatch = parseJsonFormatCommand(rawText);
	if (!commandMatch) {
		return null;
	}

	if (!commandMatch.content) {
		return null;
	}

	try {
		return JSON.stringify(JSON.parse(commandMatch.content), null, 2);
	} catch {
		return null;
	}
}

export function primaryActionLabel(match: ActionMatch) {
	return match.descriptor.aliases[0] ?? match.descriptor.title;
}

export function getErrorMessage(error: unknown, fallbackMessage: string) {
	if (typeof error === "string" && error.trim()) {
		return error;
	}

	if (error instanceof Error && error.message.trim()) {
		return error.message;
	}

	if (
		typeof error === "object" &&
		error !== null &&
		"message" in error &&
		typeof error.message === "string" &&
		error.message.trim()
	) {
		return error.message;
	}

	return fallbackMessage;
}

export function deriveFileCompletion(rawText: string, match: FileSearchMatch | undefined) {
	if (!match || rawText.length === 0) {
		return null;
	}

	return match.path;
}

export function replaceTextRange(rawText: string, start: number, end: number, replacement: string) {
	const safeStart = clamp(start, 0, rawText.length);
	const safeEnd = clamp(end, 0, rawText.length);

	return {
		value: `${rawText.slice(0, safeStart)}${replacement}${rawText.slice(safeEnd)}`,
		caretIndex: safeStart + replacement.length,
	};
}
