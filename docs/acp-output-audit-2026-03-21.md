# ACP Output UI Audit Report

Date: 2026-03-21
Scope: ACP 输出板块 in `src/features/launcher/components/SessionTimeline.tsx`, `src/features/launcher/components/LauncherFeedback.tsx`, `src/features/launcher/launcher.css`
Method: source inspection against current design context in `.impeccable.md`

## Audit Premise

This audit is scoped to the ACP output surface only, not the full launcher. Judgments are made against the project’s stated design context:

- Users: 懂一点技术的普通用户，桌面端快速启动和轻量 AI/检索交互
- Tone: 冷静、专业、平和
- Constraints: 重点必须一眼可懂，不能靠弱提示词；launcher 和 settings 共享稳定 token、尺寸和状态语义；窄窗口下不能崩

That matters because the current ACP output block is not failing by being “simple”. It is failing because it spends too much visual energy on secondary telemetry while under-emphasizing the answer itself.

## Anti-Patterns Verdict

Verdict: Fail

Does it look AI-generated? Yes, specifically the ACP output area does.

Specific tells:

- It uses the now-generic “agent transcript” recipe: tiny role label, thought chip, pill cloud, dark tooltip, then the real answer below.
- Visual hierarchy is flattened into a pile of similarly weighted rounded containers instead of one clear primary reading path.
- The most contrasted surface is the tool-detail panel, which is secondary information. That is backwards.
- Typography is timid to the point of indecision: most of the block lives in `10px` to `11px`, so nothing reads as the main event.
- The interaction pattern depends on hover-revealed detail and miniature pills, which looks like instrumentation UI pasted onto a launcher rather than deliberately shaped for end users.

This is not “too minimal”. It is structurally generic and over-instrumented.

## Executive Summary

- Total issues: 8
- Critical: 0
- High: 3
- Medium: 3
- Low: 2
- Overall quality score: 62/100

Most critical issues:

1. 主回答、思考块、action bar、角色标签几乎落在同一字号区间，层级塌陷。
2. action detail 面板比正文更大、更暗、更有对比，次级信息抢走主焦点。
3. thought toggle 和 action pills 交互尺寸过小，详情交互依赖 hover/click，桌面还勉强，窄窗口和辅助场景很差。
4. 输出区被限制在多层滚动和较低高度上限内，长回答、长工具输出和正文争夺同一视窗，阅读连续性差。

Recommended next steps:

1. 先重建信息层级，明确“答案正文 > 操作轨迹 > 思考过程 > 元信息”。
2. 再处理交互尺寸和详情展开模式，不要继续把关键交互塞进 `10px` pill。
3. 最后统一 output 区域的 token 和排版标尺，把残留的 feature 级硬编码颜色清掉。

## Detailed Findings By Severity

### High-Severity Issues

#### 1. Primary answer hierarchy is collapsed

- Location: `src/features/launcher/launcher.css:1288-1499`, `src/features/launcher/launcher.css:1519-1539`, `src/features/launcher/components/SessionTimeline.tsx:424-455`
- Severity: High
- Category: Anti-Pattern
- Description: `thought-toggle`, `thought-content`, `action-pill`, `session-message-role`, `session-message-content` all cluster around `10px` to `11px`. Markdown headings are only `1.15em`, `1.08em`, `1em`, `0.95em` on top of an `11px` base, so even structured answers barely separate from metadata.
- Impact: Users cannot parse what to read first. The answer, the reasoning chrome, and the instrumentation all compete at the same visual volume. This is exactly how “重点丢失” happens.
- WCAG/Standard: No direct WCAG failure, but violates the project’s own principle that core information must be obvious at a glance.
- Recommendation: Establish a clear reading hierarchy. The answer body needs a larger base size and stronger spacing cadence; thought and action telemetry must drop to a secondary scale instead of sharing the same visual lane.
- Suggested command: `/typeset`

#### 2. Secondary detail panel visually dominates the answer

- Location: `src/features/launcher/launcher.css:1344-1374`, `src/features/launcher/launcher.css:1492-1499`, `src/features/launcher/components/SessionTimeline.tsx:253-292`
- Severity: High
- Category: Responsive
- Description: `.action-bar-detail` is sized with `width: min(840px, calc(100vw - 48px))`, uses a very dark high-contrast panel, and can reach `56vh` height. Meanwhile the answer body remains `11px` text inside the same message card. The result is a secondary inspection surface that is larger and more visually forceful than the primary response.
- Impact: Tool output steals attention from the answer. On narrow windows the panel can feel detached from the message and visually “break out” of the composition. Users end up reading telemetry before content.
- WCAG/Standard: Reflow risk and hierarchy failure; not a clean WCAG clause, but directly harmful to comprehension.
- Recommendation: Constrain detail to the message/container context, reduce tonal contrast, and treat it as subordinate disclosure rather than the boldest object in the component.
- Suggested command: `/arrange`

#### 3. Output panel uses micro-target interactions for important controls

- Location: `src/features/launcher/launcher.css:1288-1299`, `src/features/launcher/launcher.css:1376-1426`, `src/features/launcher/components/SessionTimeline.tsx:257-275`, `src/features/launcher/components/SessionTimeline.tsx:312-321`
- Severity: High
- Category: Accessibility
- Description: `thought-toggle` uses `padding: 3px 6px` at `10px`, and `action-pill` uses `padding: 2px 6px` at `10px`. These are materially below comfortable desktop target sizes, let alone touch or zoomed environments. The pills also encode meaningful details behind hover/focus/click interactions on very small surfaces.
- Impact: Important controls become precision tasks. This slows scanning, increases missed clicks, and makes the “debugging metadata” feel fragile rather than intentional.
- WCAG/Standard: WCAG 2.2 AA `2.5.8 Target Size (Minimum)` is not met for these controls.
- Recommendation: Promote important disclosures to larger controls or rows, not miniature pills. If details matter, they deserve a real interaction surface.
- Suggested command: `/adapt`

### Medium-Severity Issues

#### 4. Nested scrolling turns reading into viewport management

- Location: `src/features/launcher/layout.ts:27-32`, `src/features/launcher/LauncherPage.tsx:600-606`, `src/features/launcher/launcher.css:1233-1240`, `src/features/launcher/launcher.css:1364-1367`
- Severity: Medium
- Category: Responsive
- Description: The ACP output area is capped to `180-360px`, while tool detail inside it can independently scroll up to `56vh`. This creates stacked reading zones: launcher shell scroll, session log scroll, and detail panel scroll. Long answers and long tool outputs compete inside a very compressed vertical budget.
- Impact: Users spend effort managing scroll position instead of reading. In a launcher, that is expensive because the entire promise is fast glanceability.
- WCAG/Standard: WCAG 2.1 AA `1.4.10 Reflow` is not cleanly violated, but the reading experience is degraded by stacked scroll containers.
- Recommendation: Reduce nested scrolling. Reserve the limited height budget for the primary answer and move deep inspection into a less intrusive disclosure model.
- Suggested command: `/adapt`

#### 5. Theme token adoption in the output area is still incomplete

- Location: `src/features/launcher/launcher.css:1288-1455`, `src/features/launcher/launcher.css:1551-1677`
- Severity: Medium
- Category: Theming
- Description: The message card background was recently moved onto tokens, but the output block still contains many direct literals for thought surfaces, action pills, markdown blockquotes, callouts, borders, links, and code blocks. The output area is therefore only partially tokenized.
- Impact: Theme drift will keep reappearing locally even if global tokens improve. ACP output will remain the part of the launcher most likely to look “off” after palette adjustments.
- WCAG/Standard: Design-system consistency issue.
- Recommendation: Finish tokenizing the ACP output area so hierarchy and theme changes can be tuned centrally instead of via scattered literals.
- Suggested command: `/normalize`

#### 6. Output state resets reduce continuity during streamed updates

- Location: `src/features/launcher/components/SessionTimeline.tsx:377-390`
- Severity: Medium
- Category: Interaction
- Description: `useLayoutEffect` rebuilds `collapsedThoughts` from scratch whenever `orderedMessages` changes. If a streamed update lands after the user expands a thought block, the local disclosure state is reset to collapsed.
- Impact: The interface feels unstable while reading. Users who intentionally inspect reasoning can lose their place when the session updates.
- WCAG/Standard: No direct WCAG clause, but it undermines interaction predictability.
- Recommendation: Preserve user-toggled disclosure state across incremental message updates; only seed defaults for newly introduced thought blocks.
- Suggested command: `/harden`

### Low-Severity Issues

#### 7. Role labels are too faint and too small to orient the stream

- Location: `src/features/launcher/launcher.css:1458-1465`, `src/features/launcher/components/SessionTimeline.tsx:409-427`
- Severity: Low
- Category: Accessibility
- Description: The role label uses `11px` uppercase text with muted color and no structural reinforcement beyond a small line at the top of each card.
- Impact: The stream becomes harder to scan quickly, especially when the user alternates between their own prompts and long agent responses.
- WCAG/Standard: Likely below ideal contrast for microtext; exact ratio not measured in-browser here, so this is a visual-audit finding rather than a formal contrast failure.
- Recommendation: Either strengthen the role marker visually or stop making it carry orientation by itself; the current treatment is too weak for the job.
- Suggested command: `/typeset`

#### 8. The output area still looks like nested “telemetry cards”

- Location: `src/features/launcher/launcher.css:1243-1353`, `src/features/launcher/launcher.css:1632-1640`
- Severity: Low
- Category: Anti-Pattern
- Description: The ACP output composes message card, thought chip, thought panel, action bar, detail panel, code blocks, and callouts as multiple rounded containers with similar corner language and border behavior. Even where each piece is individually acceptable, the aggregate effect is “cards inside cards inside cards”.
- Impact: The interface feels busier and more synthetic than the brand direction allows. It reads like agent instrumentation UI, not a calm desktop tool.
- WCAG/Standard: No direct WCAG violation.
- Recommendation: Flatten the hierarchy. Not every semantic block needs its own container; typography and spacing should do more of the work.
- Suggested command: `/distill`

## Patterns And Systemic Issues

- Small-type compression is systemic. The entire ACP output strip is trying to fit too much onto a micro scale.
- Secondary instrumentation is over-designed relative to the primary answer. Tool telemetry has stronger shape language than the answer itself.
- Container nesting is doing the job that typography and layout should do. That creates noisy hierarchy.
- Token adoption is incomplete. The output area is still partly on semantic tokens and partly on old literal colors.
- The component is desktop-only in its assumptions, but even on desktop it demands precision interactions that are too fine for relaxed use.

## Positive Findings

- The message model is structurally sane: assistant output is grouped by message boundary instead of exploding every chunk into separate rows.
- Tool call and tool result merging by `correlationId` is the right data reduction choice. It prevents even worse timeline noise.
- Thought content is collapsed by default, which is directionally correct for keeping reasoning secondary.
- `SessionTimeline` is lazily loaded from `LauncherFeedback`, so the product is not paying the UI cost until the surface is needed.
- The recent move of message card backgrounds onto tokens was the correct repair direction. The remaining problem is incomplete follow-through, not wrong intent.

## Recommendations By Priority

### Immediate

1. Rebuild information hierarchy so the answer body becomes the obvious primary element.
2. Demote tool detail from a dominant dark panel to a subordinate disclosure.
3. Raise interactive hit areas for thought and action controls to sane desktop ergonomics.

### Short-term

1. Remove nested-scroll pressure from the output area.
2. Preserve disclosure state across stream updates.
3. Finish tokenizing thought/action/markdown surfaces inside ACP output.

### Medium-term

1. Rework markdown typography so headings, lists, code, and callouts create real hierarchy.
2. Reduce the number of rounded containers and let spacing/typography carry more structure.

### Long-term

1. Decide whether ACP output is a user-facing answer surface or a debugging console. Right now it tries to be both and succeeds at neither.

## Suggested Commands For Fixes

- Use `/typeset` to rebuild answer-first typography and restore hierarchy.
- Use `/arrange` to rebalance composition, spacing rhythm, and the relationship between正文、思考、工具轨迹.
- Use `/adapt` to fix target sizes and reduce nested-scroll pressure.
- Use `/normalize` to finish tokenizing the ACP output area.
- Use `/harden` to preserve disclosure state and stabilize interaction behavior during streaming updates.
- Use `/distill` to remove telemetry-card clutter and flatten redundant containers.
