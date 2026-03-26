# Operation Trail Visual Weight Audit

Date: 2026-03-26
Scope: 问答结果中的“操作轨迹”UI，主要涉及 `src/features/launcher/components/SessionTimeline.tsx` 与 `src/features/launcher/launcher.css`
Method: 基于 `.impeccable.md`、`src/features/launcher/README.md` 的当前设计约束进行源码审计

## Audit Premise

这次不是泛化地审整个 launcher，而是专盯“问答之后出现的操作轨迹”为什么视觉比重过高。

当前项目的设计上下文已经写得很清楚：

- 用户不是重度开发者，不是要一个 debug console
- 品牌气质是冷静、专业、平和
- ACP 输出区的阅读优先级已经被明文规定为“答案正文 > 操作轨迹 > 思考过程 > 角色元信息”

所以这里的判断标准不复杂：只要操作轨迹比答案更先被看到、更像主内容、或者更像一个独立 debug 模块，它就是失败。

## Anti-Patterns Verdict

Verdict: Fail

这块局部区域仍然明显带有“AI agent / telemetry UI 模板味”，具体表现在：

- 在消息卡片内部又嵌一层独立 rounded container，再塞一团 pill，典型的“cards inside cards”。
- 用大写 kicker `操作轨迹` + 计数 `N 项` 的套路，把次级信息包装成了一个完整子模块。
- 详情区用了全块深底高对比面板，视觉上比正文更像“重点”。
- 彩色 pill 云本身就有很强的分类感和仪表盘气质，和“冷静、专业、平和”的普通用户桌面工具方向不匹配。

这不是简单的“颜色偏深”。这是结构、顺序、容器和对比一起把操作轨迹抬成了主角。

## Executive Summary

- Total issues: 6
- Critical: 0
- High: 3
- Medium: 2
- Low: 1
- Overall quality score: 58/100

Most critical issues:

1. `actions` 在 assistant message 中先于正文渲染，信息层级从 DOM 顺序开始就错了。
2. `action-bar-detail` 使用深色高对比大面板，详情展开后会立即夺走主视觉焦点。
3. `action-bar` 自己就是一个完整容器模块，内部再叠彩色 pills，导致“次级轨迹”拥有了过强的形状语言。
4. 操作轨迹大量使用硬编码颜色和局部视觉规则，后续想整体“降噪”时无法靠 token 统一收敛。

Recommended next steps:

1. 先重排信息顺序，保证正文先于操作轨迹出现。
2. 再削弱容器与详情面板，而不是只调单个颜色值。
3. 最后把 action bar 视觉决策收回 token 和系统层级，避免反复局部漂移。

## Detailed Findings By Severity

### High-Severity Issues

#### 1. 操作轨迹在结构上先于正文出现

- Location: `src/features/launcher/components/SessionTimeline.tsx:401-405`, `src/features/launcher/components/SessionTimeline.tsx:526-536`
- Severity: High
- Category: Responsive
- Description: `normalizeAssistantBlocks` 固定按 `thought -> actions -> content` 生成 block 顺序，后续渲染时也按这个顺序输出。这意味着在 assistant 消息里，操作轨迹天然先于正文出现。
- Impact: 用户会先看到“系统做了什么”，再看到“系统说了什么”。这直接违背 launcher README 已定义的阅读优先级，也解释了为什么操作轨迹总是在抢第一眼注意力。
- WCAG/Standard: 不直接对应某条 WCAG，但属于信息层级错误，损害快速理解。
- Recommendation: 操作轨迹必须在结构上降级为正文之后的次级块，否则任何样式降噪都只是表面补丁。
- Suggested command: `/arrange`

#### 2. 详情面板是当前消息里最重的视觉块

- Location: `src/features/launcher/launcher.css:1452-1483`, `src/features/launcher/components/SessionTimeline.tsx:301-313`
- Severity: High
- Category: Theming
- Description: `.action-bar-detail` 使用接近不透明的深底、亮前景和明显阴影。展开后，它的对比、体量和质感都强于 assistant 正文。
- Impact: 用户一旦 hover 或点击 pill，阅读焦点就会从答案正文跳到工具输入/输出明细。这让“轨迹详情”从辅助说明变成了实际主内容。
- WCAG/Standard: 不属于 formal contrast failure，但明显违背“主次信息对比应服务阅读优先级”的设计原则。
- Recommendation: 详情应被处理成正文上下文中的弱展开区，而不是独立的高对比 inspection panel。
- Suggested command: `/quieter`

#### 3. 操作轨迹被设计成了完整子模块，而不是正文旁注

- Location: `src/features/launcher/launcher.css:1415-1450`, `src/features/launcher/components/SessionTimeline.tsx:260-300`
- Severity: High
- Category: Anti-Pattern
- Description: `action-bar` 具有独立背景、边框、内边距、标题区、计数摘要和内部 pill 云。它在消息卡内形成了第二套完整层级。
- Impact: 次级信息拥有了完整模块化包装，用户自然会把它当成和正文同级的阅读入口。结果不是“轨迹可见”，而是“轨迹太像主功能”。
- WCAG/Standard: 无直接 WCAG 条款，但明显命中 frontend-design 中反对的 nested cards / pill-grid 模式。
- Recommendation: 如果目标是降视觉比重，首先要拆掉“自成一区”的模块感，减少容器感和标签感。
- Suggested command: `/distill`

### Medium-Severity Issues

#### 4. 彩色 pill 云放大了分类感和仪表盘感

- Location: `src/features/launcher/launcher.css:1485-1577`
- Severity: Medium
- Category: Theming
- Description: `tool / plan / mode / config / info / error` 分别使用不同背景色，形成一组高可扫描的类别 chips。对一个普通用户桌面 launcher 来说，这种分类密度偏“系统监控 UI”而不是“答案辅助信息”。
- Impact: 即使不展开详情，用户的注意力也会先被一排有色 chips 抓走。它们比自然语言正文更容易形成视觉分组。
- WCAG/Standard: 无直接 WCAG 违规，但属于视觉优先级反转。
- Recommendation: 降低类别视觉差异，让“可读答案”而不是“动作类型分组”成为主扫描路径。
- Suggested command: `/quieter`

#### 5. Action bar 仍依赖硬编码颜色，无法系统性降权

- Location: `src/features/launcher/launcher.css:1415-1577`
- Severity: Medium
- Category: Theming
- Description: action bar、detail、pill title、step marker 和各类 pill 底色仍有大量局部 RGBA/hex 字面量，而不是统一走语义 token。
- Impact: 每次想降低视觉比重，都只能局部猜测式调色。它不但难维护，还会继续和全局主题节奏脱节。
- WCAG/Standard: Design system consistency issue.
- Recommendation: 先把 action bar 收回 token 体系，再谈统一降噪，否则每次都是 feature 级打补丁。
- Suggested command: `/normalize`

### Low-Severity Issues

#### 6. 标题和计数属于重复性 UI 噪音

- Location: `src/features/launcher/components/SessionTimeline.tsx:262-265`, `src/features/launcher/launcher.css:1426-1443`
- Severity: Low
- Category: Accessibility
- Description: `操作轨迹` kicker 和 `N 项` summary 占了一整行头部信息，但这两个信息都不是用户完成阅读任务所必需的。它们更像开发向栏目标题，而不是用户向辅助提示。
- Impact: 在本来就紧张的消息卡空间里，多出一行标题 chrome，会进一步压缩正文呼吸感，并抬高轨迹的“模块存在感”。
- WCAG/Standard: 无直接 WCAG 违规。
- Recommendation: 若保留，应显著降级；更合理的是让轨迹本身以更弱的结构融入正文尾部，而不是先起一个栏目标题。
- Suggested command: `/clarify`

## Patterns And Systemic Issues

- 问题核心不是某个颜色太深，而是“结构顺序 + 独立容器 + 彩色分类 + 深色详情”一起抬高了轨迹的权重。
- action bar 当前更像开发者 telemetry 模块，不像普通用户阅读链路里的辅助说明。
- README 已经定义了正确优先级，但实现仍停留在旧的 agent transcript 心智模型里。
- 这块样式没有完全 token 化，导致视觉层级难以系统治理。

## Positive Findings

- `tool-call` 和 `tool-update` 会按 `correlationId` 融合，避免操作轨迹进一步碎片化。
- 详情默认不是常驻展开，这个方向是对的，说明实现层面并没有执意把所有内部信息全量暴露。
- 交互项使用了真实 `button` 和 `aria-expanded`，基础可达性比 click-only `div` 好。
- 项目文档已经明确“答案正文 > 操作轨迹”，说明产品判断本身没有问题，主要是落地没有跟上。

## Recommendations By Priority

### Immediate

1. 调整 assistant block 顺序，让正文先于操作轨迹渲染。
2. 去掉 detail 面板的高对比 inspection 感，把它降为弱展开区。
3. 拆弱 action bar 的独立模块感，不要再让它像消息里的第二张卡片。

### Short-term

1. 降低 pill 之间的颜色差异，避免用户先扫到分类而不是答案。
2. 清理标题、计数等栏目化 chrome。
3. 用 token 接管 action bar 颜色与前景，而不是继续堆局部字面量。

### Medium-term

1. 重新定义“操作轨迹”的角色：是辅助凭证，不是并列内容区。
2. 让轨迹更多依赖排版、间距和弱分隔，而不是独立容器和 pill 语言。

### Long-term

1. 明确问答面板到底是用户答案界面，还是 agent 调试界面。现在实现还在两者之间摇摆。

## Suggested Commands For Fixes

- Use `/arrange` to 重建正文与操作轨迹的结构顺序和布局权重。
- Use `/quieter` to 降低 detail panel、pill 分类色和整体对比。
- Use `/distill` to 去掉“消息卡里再套一张 telemetry 卡”的冗余容器感。
- Use `/normalize` to 把 action bar 收回全局 token 体系。
- Use `/clarify` to 清理“操作轨迹 / N 项”这类对用户没有直接价值的栏目化文案。
