import type {
	ActionDescriptor,
	ActionMatch,
	ExecutionRequest,
	ExecutionResult,
	ExecutionStatus,
	InstalledAppMatch,
	InputMode,
	QueryPayload,
} from "./types";
import {
	base64CommandAliases,
	deriveJsonPrettyPreview,
	jsonFormatCommandAliases,
	markdownCommandAliases,
	parseBase64Command,
	parseMarkdownCommand,
	parseSlashActionInput,
	parseJsonFormatCommand,
} from "./query";

const action = (
	id: string,
	title: string,
	summary: string,
	aliases: string[],
	keywords: string[],
	supportedInputModes: InputMode[],
	category: string,
	priority: number,
): ActionDescriptor => ({
	id,
	title,
	summary,
	aliases,
	keywords,
	supportedInputModes,
	category,
	priority,
});

const actions: ActionDescriptor[] = [
	action(
		"open_url",
		"打开链接",
		"打开当前输入的链接",
		["/open"],
		["url", "browser", "link", "open"],
		["inline", "clipboard", "selection"],
		"system",
		120,
	),
	action(
		"uppercase_text",
		"转大写",
		"把文本转换为全大写",
		["/upper"],
		["uppercase", "text", "transform"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		100,
	),
	action(
		"title_case_text",
		"转标题",
		"把每个词的首字母转为大写",
		["/title"],
		["title", "titlecase", "capitalize", "text", "transform"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		99,
	),
	action(
		"lowercase_text",
		"转小写",
		"把文本转换为全小写",
		["/lower"],
		["lowercase", "text", "transform"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		99,
	),
	action(
		"camel_case_text",
		"转驼峰",
		"把文本转换为 camelCase",
		["/camel"],
		["camel", "camelcase", "text", "transform"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		98,
	),
	action(
		"snake_case_text",
		"转下划线",
		"把文本转换为 snake_case",
		["/snake"],
		["snake", "snakecase", "underscore", "text"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		97,
	),
	action(
		"word_count",
		"统计词数",
		"统计输入中的单词数量",
		["/words"],
		["count", "words", "text"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		95,
	),
	action(
		"line_count",
		"统计行数",
		"统计输入中的行数",
		["/lines"],
		["count", "lines", "text"],
		["multiline", "ocr", "clipboard", "selection"],
		"text",
		92,
	),
	action(
		"trim_whitespace",
		"清理空白",
		"清理每行首尾和整体多余空白",
		["/trim"],
		["trim", "normalize", "text"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		90,
	),
	action(
		"unique_lines",
		"去重",
		"按行保留首次出现的内容并去重",
		["/unique"],
		["unique", "dedupe", "lines", "text"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		89,
	),
	action(
		"sort_lines",
		"排序",
		"按行进行字典序排序",
		["/sort"],
		["sort", "lines", "order", "text"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		87,
	),
	action(
		"json_pretty_print",
		"格式化 JSON",
		"格式化并缩进 JSON 内容",
		[...jsonFormatCommandAliases],
		["json", "format", "pretty"],
		["inline", "multiline", "clipboard"],
		"text",
		88,
	),
	action(
		"markdown_render",
		"渲染 Markdown",
		"按 Markdown 渲染输入内容，支持 Mermaid、MDX 安全兼容和 Obsidian 风格扩展",
		[...markdownCommandAliases],
		["markdown", "md", "mdx", "mermaid", "obsidian", "oxide"],
		["inline", "multiline", "ocr", "clipboard", "selection"],
		"text",
		89,
	),
	action(
		"base64_text",
		"Base64 编解码",
		"自动识别 Base64，命中则解码，否则编码",
		[...base64CommandAliases],
		["base64", "encode", "decode", "text"],
		["inline", "multiline", "clipboard"],
		"text",
		86,
	),
];

const actionAliasesById: Record<string, readonly string[]> = Object.fromEntries(
	actions.map((descriptor) => [descriptor.id, descriptor.aliases]),
);

export function matchActionsFallback(query: QueryPayload): ActionMatch[] {
	const normalized = query.rawText.trim().toLowerCase();

	const matches = actions
		.filter((item) => item.supportedInputModes.includes(query.mode))
		.map((item) => scoreAction(item, normalized, query.mode))
		.filter((item): item is ActionMatch => item !== null)
		.sort(
			(left, right) =>
				right.score - left.score || left.descriptor.title.localeCompare(right.descriptor.title),
		);

	return normalized.startsWith("/") ? matches : matches.slice(0, 8);
}

function scoreAction(
	descriptor: ActionDescriptor,
	normalized: string,
	mode: InputMode,
): ActionMatch | null {
	const commandToken = normalized.startsWith("/")
		? (normalized.split(/\s+/, 1)[0] ?? normalized)
		: normalized;

	if (!normalized) {
		return {
			descriptor,
			score: descriptor.priority,
		};
	}

	let score = descriptor.priority;
	const title = descriptor.title.toLowerCase();
	const aliases = descriptor.aliases.join(" ").toLowerCase();
	const keywords = descriptor.keywords.join(" ").toLowerCase();
	let matched = false;

	if (title.startsWith(commandToken)) {
		score += 90;
		matched = true;
	} else if (title.includes(commandToken)) {
		score += 55;
		matched = true;
	}

	if (aliases.includes(commandToken)) {
		score += 36;
		matched = true;
	}

	if (keywords.includes(commandToken)) {
		score += 28;
		matched = true;
	}

	if (commandToken.startsWith("/") && descriptor.aliases.includes(commandToken)) {
		score += 42;
		matched = true;
	}

	if (normalized.startsWith("http") && descriptor.id === "open_url") {
		score += 60;
		matched = true;
	}

	if (
		descriptor.id === "json_pretty_print" &&
		(parseJsonFormatCommand(normalized) || normalized.includes("{"))
	) {
		score += 44;
		matched = true;
	}

	if (descriptor.id === "base64_text" && parseBase64Command(normalized)) {
		score += 44;
		matched = true;
	}

	if (descriptor.id === "markdown_render" && parseMarkdownCommand(normalized)) {
		score += 44;
		matched = true;
	}

	if (!matched) {
		return null;
	}

	if (mode === "multiline") {
		score += 10;
	}

	return {
		descriptor,
		score,
	};
}

export function executeActionFallback(request: ExecutionRequest): ExecutionResult {
	const text = stripActionCommandPrefix(request.actionId, request.query.rawText);

	switch (request.actionId) {
		case "open_url": {
			const url = normalizeUrl(text.trim());
			if (!url) {
				return fallbackResult("error", null, "当前输入不是有效 URL。", null, []);
			}
			return fallbackResult(
				"success",
				url,
				"浏览器模式下只返回链接，不调用桌面 opener。",
				{ effect: "noop" },
				[],
			);
		}
		case "copy_text":
			return fallbackResult(
				"success",
				text,
				"浏览器模式下可由前端复制。",
				{ effect: "copy_to_clipboard" },
				["trim_whitespace", "uppercase_text"],
			);
		case "uppercase_text":
			return executeTextTransformFallback(text, (value) => value.toUpperCase(), "文本已转为大写。");
		case "title_case_text":
			return executeTextTransformFallback(text, convertTitleCase, "文本已转为标题格式。");
		case "lowercase_text":
			return executeTextTransformFallback(text, (value) => value.toLowerCase(), "文本已转为小写。");
		case "camel_case_text":
			return executeTextTransformFallback(
				text,
				(value) => convertLines(value, "camel"),
				"文本已转为驼峰。",
			);
		case "snake_case_text":
			return executeTextTransformFallback(
				text,
				(value) => convertLines(value, "snake"),
				"文本已转为下划线。",
			);
		case "word_count":
			return fallbackResult(
				"success",
				String(text.split(/\s+/).filter(Boolean).length),
				"词数统计完成。",
				null,
				[],
			);
		case "line_count":
			return fallbackResult(
				"success",
				String(
					text
						.split(/\r?\n/)
						.map((line) => line.trimEnd())
						.filter((line) => line.length > 0).length,
				),
				"行数统计完成。",
				null,
				[],
			);
		case "trim_whitespace":
			return executeTextTransformFallback(
				text,
				(value) =>
					value
						.split(/\r?\n/)
						.map((line) => line.trim())
						.join("\n")
						.trim(),
				"空白已清理。",
			);
		case "unique_lines":
			return executeTextTransformFallback(text, uniqueLines, "重复行已去重。");
		case "sort_lines":
			return executeTextTransformFallback(text, sortLines, "文本已按行排序。");
		case "json_pretty_print": {
			const preview = deriveJsonPrettyPreview(text);
			if (preview) {
				return fallbackResult("success", preview, "JSON 已格式化。", null, ["copy_text"]);
			}

			return fallbackResult("error", null, "当前输入不是有效 JSON。", null, []);
		}
		case "base64_text": {
			const payload = parseBase64Command(text)?.content ?? text.trim();
			if (!payload) {
				return fallbackResult("error", null, "请输入要编码或解码的内容。", null, []);
			}

			const decoded = tryDecodeBase64Text(payload);
			if (decoded !== null) {
				return fallbackResult("success", decoded, "Base64 已解码。", null, ["copy_text"]);
			}

			return fallbackResult("success", encodeBase64Text(payload), "文本已编码为 Base64。", null, [
				"copy_text",
			]);
		}
		case "markdown_render": {
			if (!text.trim()) {
				return fallbackResult("error", null, "请输入要渲染的 Markdown 内容。", null, []);
			}

			return fallbackResult("success", text, "Markdown 预览已生成。", { render: "markdown" }, [
				"copy_text",
			]);
		}
		default:
			return fallbackResult(
				"warning",
				text,
				"当前处于浏览器模式，桌面能力不可用。",
				{ effect: "noop" },
				[],
			);
	}
}

function stripActionCommandPrefix(actionId: string, rawText: string) {
	const aliases = actionAliasesById[actionId];
	if (!aliases) {
		return rawText;
	}

	return parseSlashActionInput(rawText, aliases)?.content ?? rawText;
}

export function searchAppsFallback(_query: string): InstalledAppMatch[] {
	return [];
}

export function launchAppFallback(path: string): ExecutionResult {
	return fallbackResult(
		"warning",
		path || null,
		"当前处于浏览器模式，不能启动本机应用。",
		{ effect: "noop" },
		[],
	);
}

function fallbackResult(
	status: ExecutionStatus,
	primaryText: string | null,
	secondaryText: string | null,
	structuredPayload: Record<string, unknown> | null,
	nextActions: string[],
): ExecutionResult {
	return {
		status,
		primaryText,
		secondaryText,
		structuredPayload,
		nextActions,
		shouldCloseLauncher: false,
	};
}

function executeTextTransformFallback(
	text: string,
	transform: (value: string) => string,
	successMessage: string,
) {
	if (!text.trim()) {
		return fallbackResult("error", null, "请输入要转换的内容。", null, []);
	}

	return fallbackResult("success", transform(text), successMessage, null, ["copy_text"]);
}

function normalizeUrl(text: string) {
	if (!text) {
		return null;
	}
	if (text.startsWith("http://") || text.startsWith("https://")) {
		return text;
	}
	if (text.includes(".") && !text.includes(" ")) {
		return `https://${text}`;
	}
	return null;
}

function convertTitleCase(text: string) {
	let transformed = "";
	let atWordStart = true;

	for (const character of text) {
		if (/\p{Letter}|\p{Number}/u.test(character)) {
			transformed += atWordStart ? character.toLocaleUpperCase() : character.toLocaleLowerCase();
			atWordStart = false;
		} else {
			transformed += character;
			atWordStart = true;
		}
	}

	return transformed;
}

function convertLines(text: string, mode: "camel" | "snake") {
	return text
		.split("\n")
		.map((line) => convertLineCase(line.replace(/\r$/u, ""), mode))
		.join("\n");
}

function convertLineCase(line: string, mode: "camel" | "snake") {
	const words = splitWords(line);
	if (words.length === 0) {
		return "";
	}

	if (mode === "snake") {
		return words.join("_");
	}

	return words
		.map((word, index) => (index === 0 ? word : `${word[0]?.toUpperCase() ?? ""}${word.slice(1)}`))
		.join("");
}

function splitWords(value: string) {
	const words: string[] = [];
	let current = "";

	for (let index = 0; index < value.length; index += 1) {
		const character = value[index] ?? "";
		if (!/[\p{Letter}\p{Number}]/u.test(character)) {
			pushWord(words, current);
			current = "";
			continue;
		}

		const previous = current[current.length - 1] ?? "";
		const next = value[index + 1] ?? "";
		const shouldSplit =
			((/[a-z0-9]/u.test(previous) && /\p{Lu}/u.test(character)) ||
				(/\p{Lu}/u.test(previous) && /\p{Lu}/u.test(character) && /\p{Ll}/u.test(next))) &&
			current.length > 0;

		if (shouldSplit) {
			pushWord(words, current);
			current = "";
		}

		current += character;
	}

	pushWord(words, current);
	return words;
}

function pushWord(words: string[], value: string) {
	if (!value) {
		return;
	}

	words.push(value.toLowerCase());
}

function uniqueLines(text: string) {
	const seen = new Set<string>();
	return text
		.split("\n")
		.map((line) => line.replace(/\r$/u, ""))
		.filter((line) => {
			if (seen.has(line)) {
				return false;
			}
			seen.add(line);
			return true;
		})
		.join("\n");
}

function sortLines(text: string) {
	return text
		.split("\n")
		.map((line) => line.replace(/\r$/u, ""))
		.sort((left, right) => left.localeCompare(right))
		.join("\n");
}

function normalizeBase64Candidate(value: string) {
	const trimmed = value.trim();
	if (!trimmed || /[^A-Za-z0-9+/=]/u.test(trimmed)) {
		return null;
	}

	const remainder = trimmed.length % 4;
	if (remainder === 1) {
		return null;
	}

	return remainder === 0 ? trimmed : `${trimmed}${"=".repeat(4 - remainder)}`;
}

function tryDecodeBase64Text(value: string) {
	const normalized = normalizeBase64Candidate(value);
	if (!normalized) {
		return null;
	}

	try {
		const binary = atob(normalized);
		const bytes = Uint8Array.from(binary, (character) => character.codePointAt(0) ?? 0);
		return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
	} catch {
		return null;
	}
}

function encodeBase64Text(value: string) {
	const bytes = new TextEncoder().encode(value);
	const binary = Array.from(bytes, (byte) => String.fromCodePoint(byte)).join("");
	return btoa(binary);
}
