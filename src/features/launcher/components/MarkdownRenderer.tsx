import { useEffect, useId, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import type { Components } from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkFrontmatter from "remark-frontmatter";
import remarkGfm from "remark-gfm";
import remarkMdx from "remark-mdx";

interface MarkdownRendererProps {
	content: string;
	className?: string;
	pending?: boolean;
}

interface MarkdownNode {
	type: string;
	value?: string;
	name?: string | null;
	attributes?: unknown[];
	children?: MarkdownNode[];
	data?: {
		hName?: string;
		hProperties?: Record<string, unknown>;
	};
	url?: string;
}

interface MarkdownParent {
	children: MarkdownNode[];
}

interface MdxAttributeValueExpression {
	type: "mdxJsxAttributeValueExpression";
	value?: string;
}

interface MdxAttribute {
	type: "mdxJsxAttribute";
	name?: string;
	value?: string | MdxAttributeValueExpression | null;
}

interface MdxExpressionAttribute {
	type: "mdxJsxExpressionAttribute";
	value?: string;
}

const imageExtensionPattern = /\.(?:avif|gif|jpe?g|png|svg|webp)$/iu;
const highlightPattern = /==([^=\n][^=\n]*?)==/gu;
const wikiLinkPattern = /(!?)\[\[([^[\]\n]+?)\]\]/gu;
const mdxFlowNodeTypes = new Set(["mdxjsEsm", "mdxFlowExpression", "mdxJsxFlowElement"]);
const mdxInlineNodeTypes = new Set(["mdxTextExpression", "mdxJsxTextElement"]);
const markdownSanitizeSchema = {
	...defaultSchema,
	tagNames: [
		...(defaultSchema.tagNames ?? []),
		"div",
		"details",
		"summary",
		"mark",
		"span",
		"svg",
		"path",
		"input",
	],
	attributes: {
		...(defaultSchema.attributes ?? {}),
		"*": [
			...((defaultSchema.attributes?.["*"] as string[] | undefined) ?? []),
			"className",
			"title",
		],
		a: [...((defaultSchema.attributes?.a as string[] | undefined) ?? []), "href", "target", "rel"],
		code: [...((defaultSchema.attributes?.code as string[] | undefined) ?? []), "className"],
		div: [
			...((defaultSchema.attributes?.div as string[] | undefined) ?? []),
			"className",
			"dataCallout",
		],
		details: [
			...((defaultSchema.attributes?.details as string[] | undefined) ?? []),
			"className",
			"open",
			"dataCallout",
		],
		input: [
			...((defaultSchema.attributes?.input as string[] | undefined) ?? []),
			"type",
			"checked",
			"disabled",
		],
		mark: [...((defaultSchema.attributes?.mark as string[] | undefined) ?? []), "className"],
		path: [
			...((defaultSchema.attributes?.path as string[] | undefined) ?? []),
			"d",
			"fill",
			"stroke",
			"strokeWidth",
			"strokeLinecap",
			"strokeLinejoin",
		],
		pre: [...((defaultSchema.attributes?.pre as string[] | undefined) ?? []), "className"],
		span: [...((defaultSchema.attributes?.span as string[] | undefined) ?? []), "className"],
		summary: [...((defaultSchema.attributes?.summary as string[] | undefined) ?? []), "className"],
		svg: [
			...((defaultSchema.attributes?.svg as string[] | undefined) ?? []),
			"className",
			"xmlns",
			"viewBox",
			"width",
			"height",
			"fill",
			"stroke",
			"strokeWidth",
			"strokeLinecap",
			"strokeLinejoin",
			"ariaHidden",
			"focusable",
		],
	},
};

let mermaidLoader: Promise<typeof import("mermaid").default> | null = null;

function isMarkdownNode(value: unknown): value is MarkdownNode {
	return typeof value === "object" && value !== null && "type" in value;
}

function isMarkdownParent(value: unknown): value is MarkdownParent {
	return typeof value === "object" && value !== null && "children" in value;
}

function isMdxAttribute(value: unknown): value is MdxAttribute {
	return (
		typeof value === "object" &&
		value !== null &&
		"type" in value &&
		(value as { type?: string }).type === "mdxJsxAttribute"
	);
}

function isMdxExpressionAttribute(value: unknown): value is MdxExpressionAttribute {
	return (
		typeof value === "object" &&
		value !== null &&
		"type" in value &&
		(value as { type?: string }).type === "mdxJsxExpressionAttribute"
	);
}

function slugifySegment(value: string) {
	return value
		.normalize("NFKD")
		.toLowerCase()
		.replace(/[\u0300-\u036f]/gu, "")
		.replace(/[^a-z0-9/_-]+/gu, "-")
		.replace(/-{2,}/gu, "-")
		.replace(/^[-/]+|[-/]+$/gu, "");
}

function slugifyPath(value: string) {
	return value
		.split("/")
		.map((segment) => slugifySegment(segment))
		.filter(Boolean)
		.join("/");
}

function createTextNode(value: string): MarkdownNode {
	return {
		type: "text",
		value,
	};
}

function createHtmlNode(value: string): MarkdownNode {
	return {
		type: "html",
		value,
	};
}

function escapeHtml(value: string) {
	return value
		.replace(/&/gu, "&amp;")
		.replace(/</gu, "&lt;")
		.replace(/>/gu, "&gt;")
		.replace(/"/gu, "&quot;");
}

function parseWikiLink(rawContent: string) {
	const pipeIndex = rawContent.indexOf("|");
	const normalizedContent = rawContent.replace(/\\/gu, "/").trim();
	const rawTarget =
		pipeIndex === -1 ? normalizedContent : normalizedContent.slice(0, pipeIndex).trim();
	const alias = pipeIndex === -1 ? null : normalizedContent.slice(pipeIndex + 1).trim() || null;
	if (!rawTarget) {
		return null;
	}

	const [targetPart, anchorPart] = rawTarget.split("#");
	const target = targetPart?.trim() ?? "";
	const anchor = anchorPart?.trim() ?? "";
	const label = alias ?? (target || anchor || rawTarget);
	return {
		target,
		anchor,
		label,
		isImage: imageExtensionPattern.test(target),
	};
}

function createWikiLinkNode(rawContent: string): MarkdownNode | null {
	const parsed = parseWikiLink(rawContent);
	if (!parsed || parsed.isImage) {
		return null;
	}

	const sluggedTarget = parsed.target ? slugifyPath(parsed.target) : "";
	const sluggedAnchor = parsed.anchor ? slugifySegment(parsed.anchor) : "";
	const url = parsed.target
		? `/wiki/${sluggedTarget}${sluggedAnchor ? `#${sluggedAnchor}` : ""}`
		: sluggedAnchor
			? `#${sluggedAnchor}`
			: "#";

	return {
		type: "link",
		url,
		children: [createTextNode(parsed.label)],
		data: {
			hProperties: {
				className: ["markdown-wiki-link"],
				title: rawContent,
			},
		},
	};
}

function replaceInlinePatterns(value: string): MarkdownNode[] {
	const nodes: MarkdownNode[] = [];
	let lastIndex = 0;
	const combinedPattern = /(!?)\[\[([^[\]\n]+?)\]\]|==([^=\n][^=\n]*?)==/gu;

	for (const match of value.matchAll(combinedPattern)) {
		const matchIndex = match.index ?? 0;
		if (matchIndex > lastIndex) {
			nodes.push(createTextNode(value.slice(lastIndex, matchIndex)));
		}

		const [fullMatch, embedMarker, wikiContent, highlightedText] = match;
		if (highlightedText) {
			nodes.push(createHtmlNode(`<mark>${escapeHtml(highlightedText)}</mark>`));
		} else if (embedMarker === "!") {
			nodes.push({
				type: "inlineCode",
				value: fullMatch,
			});
		} else {
			nodes.push(createWikiLinkNode(wikiContent) ?? createTextNode(fullMatch));
		}

		lastIndex = matchIndex + fullMatch.length;
	}

	if (lastIndex < value.length) {
		nodes.push(createTextNode(value.slice(lastIndex)));
	}

	return nodes.length > 0 ? nodes : [createTextNode(value)];
}

function transformInlinePatterns(children: MarkdownNode[]) {
	for (let index = 0; index < children.length; index += 1) {
		const child = children[index];
		if (child.type === "text" && typeof child.value === "string") {
			const hasHighlights = highlightPattern.test(child.value);
			const hasWikiLinks = wikiLinkPattern.test(child.value);
			highlightPattern.lastIndex = 0;
			wikiLinkPattern.lastIndex = 0;
			if (!hasHighlights && !hasWikiLinks) {
				continue;
			}

			const replacements = replaceInlinePatterns(child.value);
			children.splice(index, 1, ...replacements);
			index += replacements.length - 1;
			continue;
		}

		if (
			child.children &&
			!["code", "inlineCode", "link", "linkReference", "definition", "html"].includes(child.type)
		) {
			transformInlinePatterns(child.children);
		}
	}
}

function walkMarkdownTree(
	parent: MarkdownParent,
	visitor: (node: MarkdownNode, index: number, parent: MarkdownParent) => void,
) {
	for (let index = 0; index < parent.children.length; index += 1) {
		visitor(parent.children[index], index, parent);
		const currentChild = parent.children[index];
		if (currentChild?.children) {
			walkMarkdownTree({ children: currentChild.children }, visitor);
		}
	}
}

function formatMdxAttribute(attribute: unknown) {
	if (isMdxAttribute(attribute)) {
		if (!attribute.name) {
			return null;
		}

		if (attribute.value === null || attribute.value === undefined) {
			return attribute.name;
		}

		if (typeof attribute.value === "string") {
			return `${attribute.name}="${attribute.value}"`;
		}

		if (attribute.value.type === "mdxJsxAttributeValueExpression") {
			return `${attribute.name}={${attribute.value.value ?? ""}}`;
		}
	}

	if (isMdxExpressionAttribute(attribute)) {
		return `{...${attribute.value ?? ""}}`;
	}

	return null;
}

function serializeMdxChildren(children: MarkdownNode[] | undefined): string {
	if (!children || children.length === 0) {
		return "";
	}

	return children
		.map((child) => {
			if (child.type === "text") {
				return child.value ?? "";
			}
			if (child.type === "inlineCode") {
				return `\`${child.value ?? ""}\``;
			}
			if (child.type === "mdxTextExpression") {
				return `{${child.value ?? ""}}`;
			}
			if (child.type === "mdxJsxTextElement" || child.type === "mdxJsxFlowElement") {
				return serializeMdxNode(child);
			}
			if (child.children) {
				return serializeMdxChildren(child.children);
			}
			return child.value ?? "";
		})
		.join("");
}

function serializeMdxNode(node: MarkdownNode) {
	switch (node.type) {
		case "mdxjsEsm":
			return node.value ?? "";
		case "mdxFlowExpression":
		case "mdxTextExpression":
			return `{${node.value ?? ""}}`;
		case "mdxJsxFlowElement":
		case "mdxJsxTextElement": {
			const tagName = node.name || "Fragment";
			const attributes = (node.attributes ?? [])
				.map(formatMdxAttribute)
				.filter((value): value is string => typeof value === "string" && value.length > 0);
			const openingTag = `<${tagName}${attributes.length > 0 ? ` ${attributes.join(" ")}` : ""}`;
			const serializedChildren = serializeMdxChildren(node.children);
			if (!serializedChildren) {
				return `${openingTag} />`;
			}

			return `${openingTag}>${serializedChildren}</${tagName}>`;
		}
		default:
			return node.value ?? "";
	}
}

function remarkExtendedMarkdown() {
	return (tree: unknown) => {
		if (!isMarkdownParent(tree)) {
			return;
		}

		transformInlinePatterns(tree.children);

		walkMarkdownTree(tree, (currentNode, index, parent) => {
			if (!isMarkdownNode(currentNode)) {
				return;
			}

			if (currentNode.type === "yaml" || currentNode.type === "toml") {
				parent.children.splice(index, 1, {
					type: "code",
					lang: currentNode.type,
					value: currentNode.value ?? "",
				} as MarkdownNode);
				return;
			}

			if (!mdxFlowNodeTypes.has(currentNode.type) && !mdxInlineNodeTypes.has(currentNode.type)) {
				return;
			}

			parent.children.splice(index, 1, {
				type: mdxFlowNodeTypes.has(currentNode.type) ? "code" : "inlineCode",
				lang: "mdx",
				value: serializeMdxNode(currentNode),
			} as MarkdownNode);
		});

		walkMarkdownTree(tree, (currentNode) => {
			if (!Array.isArray(currentNode.children) || currentNode.type !== "blockquote") {
				return;
			}

			const firstParagraph = currentNode.children[0];
			if (
				firstParagraph?.type !== "paragraph" ||
				!Array.isArray(firstParagraph.children) ||
				firstParagraph.children.length === 0
			) {
				return;
			}

			const firstTextNode = firstParagraph.children[0];
			if (firstTextNode?.type !== "text" || typeof firstTextNode.value !== "string") {
				return;
			}

			const match = firstTextNode.value.match(/^\[!([a-z0-9_-]+)\]([+-])?(?:\s+(.*))?$/iu);
			if (!match) {
				return;
			}

			const [, rawType, rawCollapse, rawTitle] = match;
			const title = rawTitle?.trim();
			const calloutType = rawType.toLowerCase();
			currentNode.data = {
				...(currentNode.data ?? {}),
				hName: "div",
				hProperties: {
					...(currentNode.data?.hProperties ?? {}),
					className: [
						"callout",
						`callout-${calloutType}`,
						rawCollapse ? "callout-foldable" : "",
						rawCollapse === "-" ? "callout-collapsed" : "",
					].filter(Boolean),
					dataCallout: calloutType,
				},
			};
			firstParagraph.children = title
				? [
						{
							type: "strong",
							children: [createTextNode(title)],
						},
					]
				: [
						{
							type: "strong",
							children: [createTextNode(calloutType)],
						},
					];
		});
	};
}

async function loadMermaid() {
	if (!mermaidLoader) {
		mermaidLoader = import("mermaid").then((module) => {
			module.default.initialize({
				startOnLoad: false,
				securityLevel: "strict",
				theme: "neutral",
			});
			return module.default;
		});
	}

	return mermaidLoader;
}

function MermaidBlock({ chart }: { chart: string }) {
	const blockId = useId();
	const [svg, setSvg] = useState<string | null>(null);
	const [error, setError] = useState<string | null>(null);
	const graphId = useMemo(
		() => `wabity-mermaid-${blockId.replace(/[^a-zA-Z0-9_-]/gu, "")}`,
		[blockId],
	);

	useEffect(() => {
		let cancelled = false;
		setSvg(null);
		setError(null);

		void loadMermaid()
			.then(async (mermaid) => {
				const rendered = await mermaid.render(graphId, chart);
				if (!cancelled) {
					setSvg(rendered.svg);
				}
			})
			.catch((renderError: unknown) => {
				if (!cancelled) {
					setError(renderError instanceof Error ? renderError.message : "Mermaid 渲染失败");
				}
			});

		return () => {
			cancelled = true;
		};
	}, [chart, graphId]);

	if (error) {
		return (
			<div className="mermaid-block mermaid-block-error">
				<p className="mermaid-status">Mermaid 渲染失败</p>
				<pre>{chart}</pre>
				<p className="mermaid-error-detail">{error}</p>
			</div>
		);
	}

	if (!svg) {
		return (
			<div className="mermaid-block mermaid-block-loading">
				<p className="mermaid-status">Mermaid 渲染中...</p>
				<pre>{chart}</pre>
			</div>
		);
	}

	return <div className="mermaid-block" dangerouslySetInnerHTML={{ __html: svg }} />;
}

const markdownComponents: Components = {
	a({ href, children, ...props }) {
		const normalizedHref = href ?? "";
		const isExternal = /^(?:https?:|mailto:)/iu.test(normalizedHref);
		if (!isExternal) {
			return (
				<span className="markdown-link-local" title={normalizedHref || undefined}>
					{children}
				</span>
			);
		}

		return (
			<a href={normalizedHref} target="_blank" rel="noreferrer noopener" {...props}>
				{children}
			</a>
		);
	},
	pre({ children, ...props }) {
		const child = Array.isArray(children) ? children[0] : children;
		if (
			child &&
			typeof child === "object" &&
			"props" in child &&
			typeof child.props === "object" &&
			child.props !== null &&
			"className" in child.props &&
			typeof child.props.className === "string" &&
			/\blanguage-mermaid\b/iu.test(child.props.className)
		) {
			return <>{children}</>;
		}

		return <pre {...props}>{children}</pre>;
	},
	code({ className, children, ...props }) {
		const languageMatch = /language-([a-z0-9_-]+)/iu.exec(className ?? "");
		const language = languageMatch?.[1]?.toLowerCase() ?? null;
		const content = String(children).replace(/\n$/u, "");
		if (language === "mermaid") {
			return <MermaidBlock chart={content} />;
		}

		return (
			<code className={className} {...props}>
				{children}
			</code>
		);
	},
};

export function MarkdownRenderer({
	content,
	className = "markdown-body",
	pending = false,
}: MarkdownRendererProps) {
	return (
		<div className={`${className}${pending ? " pending" : ""}`}>
			<ReactMarkdown
				remarkPlugins={[remarkGfm, remarkFrontmatter, remarkMdx, remarkExtendedMarkdown]}
				rehypePlugins={[rehypeRaw, [rehypeSanitize, markdownSanitizeSchema], rehypeHighlight]}
				components={markdownComponents}
			>
				{content}
			</ReactMarkdown>
		</div>
	);
}
