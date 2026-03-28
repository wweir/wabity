# AI Settings UI Audit Report

Date: 2026-03-27
Scope: AI settings surfaces in `src/features/settings`, primarily `AI 功能` / `LLM` / `RAG`
Method: source inspection + local browser preview (`http://localhost:1420/`) with browser fallback data

## Audit Premise

这次不是泛审整个 settings，而是专盯 AI 配置相关页面为什么会给人“各种不对齐、吃垂直视野、太丑”的感受。

设计上下文来自 `.impeccable.md`：

- 用户：懂一点点技术的普通用户
- 品牌气质：冷静、专业、平和
- 明确避免：米白模板感、pill 网格、玻璃拟态、AI 工具模板味

运行时验证说明：

- 在本地浏览器预览里，AI 功能页、LLM 页、RAG 页都已做真实截图和 DOM 测量
- 当前 fallback 数据里没有已配置 provider，所以 LLM / RAG 主要验证的是布局骨架、空态和编辑框架
- 这些问题不是“数据太少才显得空”，因为大量高度和噪音来自固定 chrome、本来就存在的卡片层级和操作条

## Anti-Patterns Verdict

Verdict: Fail

这套 AI 配置 UI 很明显还带着“AI 生成的后台设置页模板味”，而且不是一点点。

具体迹象：

- 同一页同时出现左侧分组导航、主标题区、页内 jump pills、再加 section title，导航和标题重复叠了至少三层。
- `AI 功能` 页是外层详情卡里再套两张编辑卡，`LLM` / `RAG` 也在复用相同的“卡片套卡片”结构，直接命中 frontend-design 明确禁止的 nested cards。
- `RAG` 页的摘要区是标准 hero metrics 模板：大数字、短标签、状态 chip、辅助 badges，信息看起来很多，实际都不是当前任务最需要的输入。
- 英文 kicker + 中文标题 + 多个胶囊按钮 + 大量帮助文案的组合，视觉上非常像“通用 AI 配置面板生成器”。

这不是“风格保守”，而是信息层级没有被压缩，结果只能靠更多容器、更多副标题、更多 pill 去假装有层次。

## Executive Summary

- Total issues: 10
- Critical: 0
- High: 4
- Medium: 4
- Low: 2
- Overall quality score: 58/100

Most critical issues:

1. 页面 chrome 重复，主输入控件被推到首屏下半甚至折叠以下，信息密度极低。
2. AI 功能、LLM、RAG 都在复用 card-inside-card 结构，制造了大量无意义描边、留白和层级噪音。
3. AI 功能卡头部的保存动作在常见桌面宽度下直接换成三行，形成明显错位。
4. 提示词编辑器默认就是两块超高 textarea，导致首屏几乎全是框架，不是输入。

Recommended next steps:

1. 先砍重复 chrome：保留一种主导航、保留一种页内定位，不要全都要。
2. 把 AI 功能页从“外层卡 + 内层卡 + 卡头按钮墙”改成更扁平的表单布局。
3. 重做保存动作层级和位置，不要让每块卡片都自带一排长按钮。
4. 之后再清理术语、辅助文案和 RAG 指标模板。

## Detailed Findings By Severity

### Critical Issues

- None.

### High-Severity Issues

#### 1. 重复标题与重复导航直接吞掉首屏高度

- Location: `src/features/settings/SettingsPage.tsx:3894-3913`, `src/features/settings/SettingsPage.tsx:4291-4293`, `src/features/settings/settings.css:1976-1999`, `src/features/settings/settings.css:1846-1897`
- Severity: High
- Category: Responsive
- Description: AI 配置相关页面同时保留了左侧 section rail、主区“当前分组”标题、页内 jump pills、以及 section 自己的标题。以 `AI 功能` 页为例，真实预览里首个可编辑下拉框 `#translation-provider` 顶部落在 `533.85px`，而 viewport 只有 `856px` 高；在它之前已经消耗了 header、主标题、jump pills、section title、卡片头等大段固定空间。
- Impact: 用户打开页之后先看到的是“你现在在哪一页”和“你还能跳到哪一块”，而不是“你现在该改什么”。这对桌面设置页是明显的优先级错误，也直接造成“占垂直视野空间”的主观感受。
- WCAG/Standard: WCAG 2.1 AA `1.4.10 Reflow` 风险；同时违反设计上下文里“核心操作必须一眼可懂”。
- Recommendation: 左侧 rail 和页内 jump 只能保留一个主导航；如果保留 jump pills，就不要再重复 section title。主标题区也应缩成单行状态，而不是再占一块独立舞台。
- Suggested command: `/arrange`

#### 2. AI 配置页存在明显的 card-inside-card 反模式

- Location: `src/features/settings/SettingsPage.tsx:4292-4406`, `src/features/settings/SettingsPage.tsx:4408-4520`, `src/features/settings/SettingsPage.tsx:4538-4685`, `src/features/settings/SettingsPage.tsx:5408-5538`, `src/features/settings/settings.css:1006-1015`, `src/features/settings/settings.css:2180-2203`
- Severity: High
- Category: Anti-Pattern
- Description: `AI 功能` 页使用 `settings-acp-detail` 作为外层卡，再把 `翻译配置`、`文档问答配置` 两块 `settings-editor-card` 塞进去。`LLM` 和 `RAG` 页面同样把“大面板 + 内层编辑卡/草稿卡/hero 卡”叠在一起。CSS 上这些容器共享同一套 card border / background 语义，视觉层级并没有真正区分。
- Impact: 用户会持续看到“框里还有框，框里再来一个框”。这不只是丑，还让视觉边界失真，读起来像一层层配置 wizard，而不是一个稳定的设置面板。
- WCAG/Standard: 无直接 WCAG 条款；这是 frontend-design 明确禁止的 nested cards 反模式。
- Recommendation: 外层 detail 容器改为纯布局容器，真正需要强调的块才保留单层 card。不要把“页面列容器”也画成卡片。
- Suggested command: `/distill`

#### 3. 卡片头保存动作在常见桌面宽度下已经错位换行

- Location: `src/features/settings/SettingsPage.tsx:4299-4338`, `src/features/settings/SettingsPage.tsx:4413-4453`, `src/features/settings/settings.css:1028-1047`, `src/features/settings/settings.css:2021-2028`
- Severity: High
- Category: Responsive
- Description: `settings-editor-card-header` 用左右两栏 `flex`，右侧 actions 允许 `wrap`，按钮又统一 `min-height: 44px`。在本地预览下，`翻译配置` 头部宽 `640px`，左侧说明块占 `301px`，右侧动作区只剩 `327px`，结果三个按钮被挤成三行，最后一个“保存翻译配置”单独悬在右下角。
- Impact: 这就是用户口中的“各种不对齐”。它不是边角问题，而是首屏最大可见块的头部直接出现锯齿形排布，视觉上像没收住的后台表单。
- WCAG/Standard: WCAG 2.1 AA `1.4.10 Reflow` 风险；也破坏了主操作层级。
- Recommendation: 这类保存动作不该塞在卡头右侧堆成按钮墙。要么下沉到统一 save bar，要么只保留一个主保存动作，把“恢复默认/恢复已保存版本”收成次级文本操作。
- Suggested command: `/arrange`

#### 4. 提示词编辑器默认高度过大，直接压垮信息密度

- Location: `src/features/settings/SettingsPage.tsx:4387-4399`, `src/features/settings/SettingsPage.tsx:4505-4517`, `src/features/settings/settings.css:321-327`
- Severity: High
- Category: Responsive
- Description: 两个 prompt textarea 默认都是 `rows={10}`。真实 DOM 里每块 textarea 高约 `210px`，单张翻译卡总高约 `731px`，文档问答卡约 `751px`，整个 `AI 功能` panel 高 `1584px`。这意味着用户在一屏内根本看不到两块配置的整体关系。
- Impact: 对“偶尔改一下模型或 prompt”的桌面用户，这种默认展开的大编辑器是典型的过度曝光。页面看起来像一张长文档，不像设置页。
- WCAG/Standard: 无直接 WCAG 条款；属于布局与任务优先级失配。
- Recommendation: prompt 默认应折叠到摘要态或更低高度，只有明确进入编辑时再展开。模型选择和保存状态应优先进入首屏。
- Suggested command: `/adapt`

### Medium-Severity Issues

#### 5. 帮助文案重复堆叠，靠解释而不是结构传达信息

- Location: `src/features/settings/SettingsPage.tsx:4303-4306`, `src/features/settings/SettingsPage.tsx:4367-4377`, `src/features/settings/SettingsPage.tsx:4401-4404`, `src/features/settings/SettingsPage.tsx:4417-4419`, `src/features/settings/SettingsPage.tsx:4482-4495`
- Severity: Medium
- Category: Anti-Pattern
- Description: 单个配置块里同时存在卡头说明、字段下状态说明、无可用模型说明、textarea 下补充说明。翻译卡和问答卡还在重复同一种句式，只是把“翻译”换成“回答阶段”。
- Impact: 页面需要滚很多，但真正新增的信息很少。用户不是没看到，而是在反复读同一个边界条件。
- WCAG/Standard: 无直接 WCAG 条款；违反 frontend-design 里“不要重复用户已经能看到的信息”。
- Recommendation: 每块只保留一层说明。状态性信息靠就近状态标签，规则性信息收进折叠说明，不要在每个字段下继续堆句子。
- Suggested command: `/clarify`

#### 6. LLM 空态仍然沿用双栏编辑器框架，导致空页面也显得臃肿

- Location: `src/features/settings/SettingsPage.tsx:4538-4685`, `src/features/settings/settings.css:382-387`, `src/features/settings/settings.css:655-667`
- Severity: Medium
- Category: Responsive
- Description: 即使一个 LLM 条目都没有，页面仍然先渲染“模型列表 + 当前编辑”双栏框架，再在左边放空态、右边放草稿状态和空编辑态。真实预览里没有数据时，首屏仍然被两大块容器切开。
- Impact: 用户刚进入就被要求理解一个完整编辑工作台，而不是先完成“新增一个条目”这个唯一主任务。空态没有变简单，反而更像复杂系统没配置好。
- WCAG/Standard: 无直接 WCAG 条款
- Recommendation: 无条目时把 LLM 页收敛成单列 onboarding；只有存在条目后再进入左右分栏编辑模式。
- Suggested command: `/onboard`

#### 7. RAG 摘要区落入典型 hero-metrics 模板

- Location: `src/features/settings/SettingsPage.tsx:5421-5485`, `src/features/settings/settings.css:185-235`, `src/features/settings/settings.css:621-659`
- Severity: Medium
- Category: Anti-Pattern
- Description: `RAG` 页把 Embedding 状态、四个计数指标、支持后缀 chip、流程 badges、hero meta 一起堆在首屏。大数字指标块和 badge 群本身很显眼，但多数信息对“下一步该干什么”帮助有限。
- Impact: 用户会被“0 / 16 / 6 / 0”这类统计吸走注意力，但真正需要做的动作其实是选 Embedding、选目录、存配置。这正是 frontend-design 点名要避免的 hero metric 模板。
- WCAG/Standard: 无直接 WCAG 条款
- Recommendation: 首屏优先放输入和状态，统计信息降级到次级摘要或折叠区域。RAG 不是 dashboard，不需要仪表盘叙事。
- Suggested command: `/distill`

#### 8. 英文 kicker 与中文主标签混用，版面节奏杂乱

- Location: `src/features/settings/SettingsPage.tsx:4301`, `src/features/settings/SettingsPage.tsx:4415`, `src/features/settings/SettingsPage.tsx:5414`, `src/features/settings/SettingsPage.tsx:5541`
- Severity: Medium
- Category: Accessibility
- Description: 页面主体语言是中文，但频繁插入 `Translation`、`RAG Answer`、`RAG Index`、`Index Pipeline` 这类英文 kicker。它们不是专有名词必须保留英文的场景，更多是在制造“专业感”。
- Impact: 这会让页面节奏显得碎，尤其在已经有很多按钮、chip、辅助文案时，更容易强化模板味而不是提升可理解性。
- WCAG/Standard: 无直接 WCAG 条款；属于可理解性与术语一致性问题。
- Recommendation: 协议名、API 名保留英文即可；section kicker 和普通描述优先中文，避免无意义双语切换。
- Suggested command: `/clarify`

### Low-Severity Issues

#### 9. 动作锚点不统一，右侧按钮宽度忽长忽短

- Location: `src/features/settings/SettingsPage.tsx:4308-4337`, `src/features/settings/SettingsPage.tsx:4422-4451`, `src/features/settings/SettingsPage.tsx:4649-4676`, `src/features/settings/settings.css:1041-1046`
- Severity: Low
- Category: Responsive
- Description: 有些块右侧是三颗按钮，有些是两颗，有些最后一颗单独右靠。按钮本身既不等宽，也没有稳定锚点。即使功能相似，看起来也像不同人拼起来的。
- Impact: 单个问题不致命，但会放大整体“不齐”的观感。
- WCAG/Standard: 无直接 WCAG 条款
- Recommendation: 统一保存动作体系与停靠位置，不要让每个面板自己决定按钮栈形状。
- Suggested command: `/polish`

#### 10. 页内 jump pills 可横向滚动但隐藏滚动条，发现性偏弱

- Location: `src/features/settings/settings.css:1852-1863`, `src/features/settings/settings.css:1892-1897`
- Severity: Low
- Category: Responsive
- Description: compact jump list 使用横向滚动，同时把 scrollbar 全隐藏。AI 功能页只有两个按钮还不明显，但 RAG 三个按钮时已经开始依赖横向可滚。
- Impact: 这不会立即阻断，但会让用户不知道后面还有内容，尤其在更窄窗口下更明显。
- WCAG/Standard: 无直接 WCAG 条款
- Recommendation: 窄宽度下改成自动换行或显式分段，而不是靠隐藏滚动条的横滑容器。
- Suggested command: `/adapt`

## Patterns And Systemic Issues

- 主区 chrome 太多：左侧分组、主标题、jump pills、section title 同时存在。
- AI 配置区普遍在用套娃卡片，而不是清晰的表单层级。
- 解释性文案替代了结构设计，导致页面越来越长。
- 保存动作没有统一模式，哪里都想放一排按钮。
- `RAG` 和 `LLM` 还在沿用 dashboard / editor workbench 心智，不像轻量桌面工具设置页。

## Positive Findings

- 控件基础触达尺寸基本达标。当前看到的主按钮、切换按钮、jump pills 普遍达到 `44px` 高度，这一点比旧版更稳。
- 表单仍主要使用原生 `label + select/textarea/input`，没有为“像设计稿”牺牲基本语义。
- 桌面暗色基底和整体 token 方向本身没有跑偏，问题主要在结构层和信息密度层，不是颜色灾难。
- 相关页面已经有稳定的 section id 和 block ref，后续做跳转、折叠、统一 save bar 比较容易，不需要推翻状态管理。

## Recommendations By Priority

### Immediate

1. 删掉一层导航和一层标题，先把首个核心字段拉回首屏。
2. 去掉 AI 功能页的外层卡片壳，只保留单层编辑块。
3. 把每块卡头的三按钮操作条改成统一保存模式。

### Short-term

1. 收缩 prompt 编辑器默认高度，改成摘要 + 展开编辑。
2. 合并重复帮助文案，把状态说明收回就近字段。
3. 让 LLM 空态变成单列 onboarding，而不是空双栏工作台。

### Medium-term

1. 重做 RAG 首屏，去掉 hero metrics 叙事。
2. 统一 AI 配置页的中文术语策略，删掉装饰性英文 kicker。
3. 建一套“设置页动作条”规范，不再每个 panel 自己排按钮。

### Long-term

1. 把 settings 从“后台模板页”收敛成“桌面工具控制面板”。
2. 让 AI 配置页以任务流组织，而不是以容器数量组织。

## Suggested Commands For Fixes

- `/arrange`：先处理主标题、jump bar、卡头动作错位和首屏垂直节奏。
- `/distill`：去掉 nested cards、RAG metric dashboard 化、重复容器。
- `/adapt`：压缩 prompt 编辑器默认高度，处理窄宽度下的 jump pills 和动作条重排。
- `/clarify`：删重复说明，统一中英文术语与帮助文案。
- `/onboard`：重做 LLM 空态，避免无数据时仍然展示完整工作台。
- `/polish`：统一按钮锚点、宽度和细节对齐。
- `/audit`：修完后再做一次回归审计，确认没有引入新的 reflow 和对齐问题。
