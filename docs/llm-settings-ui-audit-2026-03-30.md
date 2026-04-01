# LLM Settings UI Audit Report

Date: 2026-03-30
Scope: `LLM` 配置页
Sources: source inspection + local browser preview (`http://localhost:1420/`) + DOM measurement at `1394x856`, `720x837`, `560x837`

## Audit Premise

设计上下文来自 `.impeccable.md`：

- 用户：懂一点点技术的普通用户
- 品牌气质：冷静、专业、平和
- 明确避免：AI 工具模板味、玻璃拟态、pill 网格、为了“像 AI 产品”而堆装饰

这次只审 `LLM` 页，不把 `AI 功能`、`RAG` 的旧问题机械套到这里。

## Anti-Patterns Verdict

Verdict: Fail

这页已经没有紫蓝渐变、玻璃感、hero metrics 这类最廉价的 AI 模板味，但它仍然明显带着“通用模型配置工作台”的生成式结构痕迹：

- 先是条目目录卡，再是草稿状态卡，再是 hero 卡，最后才到真正可编辑字段。结构像在展示一个 workbench，而不是在做设置。
- 同一个条目的名称、Base URL、模型、类型，在左侧卡片和右侧 hero 里重复出现；这不是信息设计，是重复 chrome。
- protocol/type/status 大量依赖 chip 和胶囊标签承载层级，容易回到“看起来信息很多，实际主任务被埋掉”的模板套路。

它的问题不在配色，而在信息入口顺序和容器层级。

## Executive Summary

- Total issues: 8
- Critical: 0
- High: 3
- Medium: 3
- Low: 2
- Overall quality score: 68/100

Most critical issues:

1. 首个可编辑字段被目录卡、草稿卡和 hero 卡压到首屏之外，`720px` 和 `560px` 视口都必须先滚动。
2. 草稿操作区在真实视口下稳定退化成三行按钮墙，主次操作失去层级。
3. 条目目录使用 `aria-pressed` 表达单选状态，语义错误，会误导辅助技术。

Recommended next steps:

1. 先把“目录 + 状态 + 编辑”重排成真正的设置流，把第一个输入字段拉回首屏。
2. 纠正条目目录和模型选择器的选择语义，不要再拿按钮状态硬扛列表选择。
3. 收掉 hero 和重复摘要，只保留一层必要概览。

## Detailed Findings By Severity

### Critical Issues

- None.

### High-Severity Issues

#### 1. 首个可编辑字段被固定 chrome 压到首屏之外

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:227-339`, `src/features/settings/sections/LlmSettingsSection.tsx:342-397`, `src/features/settings/settings.css:935-967`, `src/features/settings/settings.css:2026-2053`, `src/features/settings/settings.css:2837-2875`
- Severity: High
- Category: Responsive
- Description: 用户进入有条目的 `LLM` 页后，先看到的是条目目录、草稿状态卡和 hero 概览，真正第一个编辑控件 `#llm-provider-template` 被压得很靠后。真实测量里，字段顶部相对 `#settings-panel-llm` 的位置在 `1394px` 视口是 `745.85px`，在 `720px` 视口是 `960.69px`，在 `560px` 视口是 `959.48px`。
- Impact: 中等和窄窗口下，用户打开页之后不能立即开始配置，必须先滚过一整屏“你在编辑什么”的说明。这直接违背“核心操作必须一眼可懂”。
- WCAG/Standard: WCAG 2.1 AA `1.4.10 Reflow` 风险；也违反项目设计上下文第 1 条。
- Recommendation: 让编辑字段先于 hero 暴露，或者把 hero 收成单行摘要。目录与草稿状态不能同时占据主内容上半屏。
- Suggested command: `/arrange`

#### 2. 草稿操作区在真实视口下退化成三行按钮墙

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:303-339`, `src/features/settings/settingsShared.tsx:112-133`, `src/features/settings/settings.css:2047-2053`, `src/features/settings/settings.css:2837-2875`
- Severity: High
- Category: Responsive
- Description: `SettingsDraftActionCard` 里的 `定位问题 / 恢复已保存版本 / 保存 LLM 配置` 三个动作在真实页面里都已经不是一行操作条。`1394px` 视口下三颗按钮分别落在 3 行；`720px` 和 `560px` 视口下三颗按钮都被拉成整行宽按钮，形成 248px 高的操作墙。
- Impact: 主操作不再突出，次级动作和主保存动作长得一样、占的地方一样大。用户在“先看状态还是先改字段”之间被迫停顿，页面节奏被按钮墙打断。
- WCAG/Standard: WCAG 2.1 AA `1.4.10 Reflow` 风险；同时违反 frontend-design 对按钮层级的要求。
- Recommendation: 只保留一个主保存动作，把“定位问题/恢复已保存版本”下沉成文本操作或次级菜单；不要让卡头承担三颗并列按钮。
- Suggested command: `/arrange`

#### 3. 条目目录把单选关系错误建模成 `aria-pressed`

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:232-241`
- Severity: High
- Category: Accessibility
- Description: 左侧条目目录本质是“多项里选一个当前编辑对象”，但实现使用了普通按钮加 `aria-pressed={isSelected}`。`aria-pressed` 语义是切换按钮，不表达互斥单选集合，也不提供列表位置和选中关系。
- Impact: 屏幕阅读器会把它读成一组彼此独立的开关按钮，而不是“当前选中的条目列表”。这会让非视觉用户更难理解当前编辑上下文。
- WCAG/Standard: WCAG 2.1 A `4.1.2 Name, Role, Value`
- Recommendation: 改成真正的单选模式，例如 `radiogroup + radio`、`listbox + option`，或与页面结构一致的 `tablist + tab`。
- Suggested command: `/harden`

### Medium-Severity Issues

#### 4. 目录卡和 hero 重复呈现同一批关键信息

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:242-290`, `src/features/settings/sections/LlmSettingsSection.tsx:342-374`
- Severity: Medium
- Category: Anti-Pattern
- Description: 当前选中条目的名称、Base URL、模型名、配置类型、用途徽章，先在目录卡片里展示一遍，切到右侧又在 hero 里再展示一遍。新增草稿时，标题甚至都还是同一个“LLM 条目”。
- Impact: 这不是必要冗余，而是在用重复信息撑层级。结果是可编辑表单被进一步往下挤，页面看起来像“先看摘要，再看另一个摘要，再编辑”。
- WCAG/Standard: 无直接 WCAG 条款；命中 frontend-design 里“不要重复用户已经能看到的信息”。
- Recommendation: 目录负责切换，hero 只保留真正新增的信息；如果 hero 不提供额外决策价值，就直接删掉。
- Suggested command: `/distill`

#### 5. 模型选择器只有“打开/关闭”，没有完整的 combobox 语义与键盘流

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:595-665`, `src/features/settings/SettingsPage.tsx:589-605`, `src/features/settings/SettingsPage.tsx:801-817`
- Severity: Medium
- Category: Accessibility
- Description: 当前模型选择器用输入框 + 按钮 + `role="listbox"` 面板拼了一个自定义控件，但输入框没有 `role="combobox"`，键盘行为也只有 `ArrowDown` 打开、`Escape` 关闭，没有对 option 的箭头漫游或 `aria-activedescendant`。
- Impact: 视觉用户还能靠鼠标点，但键盘用户和辅助技术用户拿不到完整的选择模型，也很难判断焦点现在是在输入框还是在候选列表。
- WCAG/Standard: WCAG 2.1 A `4.1.2 Name, Role, Value`，WCAG 2.1 A `2.1.1 Keyboard`
- Recommendation: 要么退回原生 `select`/`datalist`，要么把它做成真正符合 WAI-ARIA 预期的 combobox。
- Suggested command: `/harden`

#### 6. LLM 专属样式仍残留硬编码语义颜色，theme token 没有收紧到底

- Location: `src/features/settings/settings.css:617-625`, `src/features/settings/settings.css:655-657`
- Severity: Medium
- Category: Theming
- Description: `settings-llm-provider-card-issue-badge` 和 `settings-llm-provider-card-model-label` 仍直接写死 `#7a4a3d`、`#57604d` 等颜色，而不是走统一 token。其它大部分设置页元素已经被后段 token 覆盖，但这两处没有。
- Impact: 当前主题也许还能看，但后续调 token、切浅深色、统一状态语义时，这些局部硬编码会先脱队，制造“局部像老版本”的撕裂感。
- WCAG/Standard: 无直接 WCAG 条款；违反项目“共享稳定 token、尺寸和状态语义”的设计原则。
- Recommendation: 把条目问题态和模型标签态都收回到状态/文本 token，不要让 LLM 页自己维护私有语义色。
- Suggested command: `/normalize`

### Low-Severity Issues

#### 7. 空条目默认标题导致同页出现两个相同的 `LLM 条目` 标题

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:213`, `src/features/settings/sections/LlmSettingsSection.tsx:347-349`
- Severity: Low
- Category: Accessibility
- Description: 新增但未命名时，左侧目录标题是 `LLM 条目`，右侧 hero 也是 `LLM 条目`。在内容提取和屏幕阅读场景里，这会形成两个几乎没有区分度的同级标题。
- Impact: 不会阻断操作，但会降低页面大纲的辨识度，尤其是在新建草稿这个最需要引导命名的阶段。
- WCAG/Standard: WCAG 2.1 AA `2.4.6 Headings and Labels` 风险
- Recommendation: 新建态标题应强调“未命名条目”或“新建条目”，而不是回退成通用标题。
- Suggested command: `/clarify`

#### 8. 帮助文案仍偏多，结构还在靠解释补洞

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:214-216`, `src/features/settings/sections/LlmSettingsSection.tsx:410-414`, `src/features/settings/sections/LlmSettingsSection.tsx:752-761`, `src/features/settings/sections/LlmSettingsSection.tsx:779-781`, `src/features/settings/sections/LlmSettingsSection.tsx:836-848`
- Severity: Low
- Category: Anti-Pattern
- Description: 页面已经比旧版收敛，但仍然在目录说明、字段帮助、用途卡、能力卡里重复解释“这个条目能做什么、该怎么选”。很多说明本来可以靠结构或字段名表达。
- Impact: 单个段落都不算长，但叠起来会把“设置页”继续推回“说明书页”的感觉。
- WCAG/Standard: 无直接 WCAG 条款
- Recommendation: 每块只保留一层真正必要的说明，其它信息交给更强的标签和更短的状态摘要。
- Suggested command: `/clarify`

## Patterns And Systemic Issues

- `LLM` 页已经从旧版“大而乱”收敛，但仍然保留了 workbench 式入口顺序：先摘要，再状态，再摘要，最后才是字段。
- 目录与编辑区都在重复输出条目身份信息，导致主表单一直被往下推。
- 语义问题集中在“把复杂选择控件做成按钮集合”这一类实现方式上，而不是基本表单控件缺 label。
- 主题问题不是整体失控，而是局部遗留：大部分颜色已经 token 化，少量 LLM 私有颜色还没收干净。

## Positive Findings

- 没有发现横向滚动问题。真实测量中 `1394 / 720 / 560` 三档视口的 `scrollWidth` 都等于 viewport 宽度。
- 基础表单语义总体是对的：大部分输入都具备 `label`，错误字段也补了 `aria-invalid` 和 `aria-describedby`。
- 触达尺寸基本达标：主按钮和关键操作控件统一设成至少 `44px` 高，桌面窄窗口下也没有缩成难点状态。
- 颜色方向本身是克制的，没有回到紫蓝渐变、玻璃感、浮夸 glow 那套 AI 模板审美。
- 没发现明显性能级问题，例如布局抖动、在动画中改 `width/height`、或无意义的大资源加载。

## Recommendations By Priority

### Immediate

1. 把第一个编辑字段拉回首屏，先处理信息入口顺序。
2. 去掉草稿卡里的三按钮并列方案，重建主次动作层级。
3. 纠正条目目录的单选语义。

### Short-term

1. 收掉 hero 与目录之间的重复信息。
2. 修正模型选择器的键盘与 ARIA 语义。
3. 收紧 LLM 页残留的硬编码颜色。

### Medium-term

1. 把 `LLM` 页彻底从“模型工作台”收敛成“模型设置页”。
2. 继续删说明性文案，让结构本身表达规则。

### Long-term

1. 为 settings 里的“目录选择器 / 草稿动作条 / 自定义选择器”建立统一模式，避免每个分组各自发明一套。

## Suggested Commands For Fixes

- `/arrange`：解决首屏入口顺序和草稿操作条布局，覆盖 2 个高优问题。
- `/harden`：修正目录单选语义和模型选择器键盘/ARIA，覆盖 2 个可达性问题。
- `/distill`：删除目录卡与 hero 的重复摘要，覆盖 1 个结构性反模式。
- `/normalize`：把 LLM 页残留的语义色收回 token，覆盖 1 个主题问题。
- `/clarify`：压缩重复说明文案和新建态标题，覆盖 2 个低优问题。
