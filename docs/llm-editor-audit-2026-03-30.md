# LLM Editor Audit Report

Date: 2026-03-30
Scope: `LLM` 设置页的编辑区域（不含 `AI 功能` / `RAG` 独立页面）
Sources: source inspection + local browser preview (`http://localhost:1420/`) + DOM measurement at `720x837`

## Audit Premise

设计上下文来自 `.impeccable.md`：

- 用户：懂一点点技术的普通用户
- 品牌气质：冷静、专业、平和
- 约束：核心操作必须一眼可懂；设置项必须所见即所得；避免“AI 工具模板味”的 workbench 结构

这次只审 `LLM` 页右侧编辑区，以及它和条目目录之间的编辑入口关系。

## Anti-Patterns Verdict

Verdict: Fail

这块已经没有紫蓝渐变、玻璃拟态和发光边框这些最低级的 AI 套壳味，但它仍然明显像“模型配置工作台”，不是“冷静、专业、可直接编辑的设置页”。

具体问题：

- 编辑入口前置了太多 chrome：状态说明、类型 chip、三个文本动作、一个主按钮，字段反而退到后面。
- 模式切换过重：`使用模板`、`配置类型`、`模型名` 三个控件都不是普通输入，而是会联动其它规则的“模式开关”，但界面没有把影响范围表达清楚。
- 容器层级仍然像工作台：编辑面板、能力面板、模板信息、目录模型、警告框都在抢“我很重要”，主任务没有被做成第一视觉层级。

问题核心不是配色，而是入口顺序、模式切换和信息负担。

## Executive Summary

- Total issues: 6
- High: 3
- Medium: 2
- Low: 1
- Overall quality score: 61/100

Most critical issues:

1. `720x837` 视口下，新建条目后第一个可编辑字段 `#llm-provider-template` 顶部位于 `1025px`，编辑区本体 `#llm-editor` 顶部位于 `796px`，首屏仍然先看到导航和状态层，不是字段。
2. `使用模板` 和 `配置类型` 都会隐式改写多项字段，并联动影响 OCR / AI 功能 / RAG 的引用资格；当前 UI 只用短帮助文案承载这些副作用。
3. `模型名` 控件把“自由输入 / 远端 `/models` 拉取 / 模板白名单选择”三种模式叠在同一个表面里，交互心智明显分裂。

Recommended next steps:

1. 先把编辑入口重新压缩成“条目目录 + 极短状态条 + 字段”，让第一个字段回到首屏。
2. 把模板和配置类型从“普通下拉框”提升为“会改写状态的模式切换”，明确告知会改哪些字段、会影响哪些引用。
3. 把模型选择器拆清楚，不要再让一个控件同时扮演文本框、远端目录和白名单选择器。

## Detailed Findings By Severity

### Critical Issues

- None.

### High-Severity Issues

#### 1. 窄窗口下编辑区入口仍然被导航和状态层压到首屏之外

- Location: `src/features/settings/SettingsPage.tsx:2146-2200`, `src/features/settings/sections/LlmSettingsSection.tsx:275-459`, `src/features/settings/settings.css:1704-1712`, `src/features/settings/settings.css:2736-2869`
- Severity: High
- Category: Responsive
- Description: `720x837` 本地预览里，新建一个 LLM 草稿后，编辑区 `#llm-editor` 顶部位于 `796px`，第一个字段 `#llm-provider-template` 顶部位于 `1025px`。也就是说，进入“编辑态”后首屏依旧看不到第一个实际输入控件。
- Impact: 用户进入页面后仍然要先穿过分组导航、section header、条目目录和状态条，才能真正开始填字段。这直接违背“核心操作必须一眼可懂”。
- WCAG/Standard: WCAG 2.1 AA `1.4.10 Reflow` 风险；也违反 `src/features/settings/README.md` 对主编辑入口优先暴露真实字段的约束。
- Recommendation: 在窄窗口下把 `LLM` 页收敛成“目录摘要 + 立即编辑”，不要让 section nav 和条目目录共同挤占首屏。`LLM` 分组尤其不该在 `720px` 宽度还保留这么长的导航前置路径。
- Suggested command: `/arrange`

#### 2. `使用模板` 与 `配置类型` 是高影响模式切换，但 UI 把它们伪装成普通下拉框

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:459-520`, `src/features/settings/SettingsPage.tsx:1806-1893`, `src/features/settings/settingsState.ts:510-547`, `src/features/settings/settingsState.ts:637-699`, `src/features/settings/settingsState.ts:741-776`
- Severity: High
- Category: Interaction / Logic
- Description: 这两个下拉框都不是单纯填值。`使用模板` 会改写 `name/baseUrl/modelType/protocol/model/supportsMultimodal/supportsStateful`；`配置类型` 会改写 `modelType/protocol/supportsStateful`。同时，`reconcileLlmSettings`、`reconcileOcrSettings`、`reconcileRagSettings` 又会根据改写后的能力自动清掉翻译 / 问答 / OCR / RAG 的引用资格。
- Impact: 用户看到的是“我改了一个选择器”，系统实际做的是“重写条目身份并联动其它设置页引用”。这不是轻量表单，而是模式切换。现在没有任何影响范围预告，用户只能在别处分组里发现“为什么引用没了”。
- WCAG/Standard: 无直接 WCAG 条款；违反项目设计原则第 1 条和第 5 条，“不能靠弱提示词撑交互”“设置项必须所见即所得”。
- Recommendation: 把这两个控件改成显式模式切换器。选中前应说明会改哪些字段、哪些能力会失效、哪些其它分组引用会被撤销；至少要在控件下方显示结构化影响摘要，而不是散落在帮助文案里。
- Suggested command: `/clarify`

#### 3. `模型名` 控件同时承担手填、远端目录和白名单三种角色，交互心智分裂

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:657-849`, `src/features/settings/SettingsPage.tsx:514-592`
- Severity: High
- Category: Accessibility / Interaction
- Description: 当前“模型名”表面上是一个输入框，但实际有三种模式：
  - 普通条目：可手填，也可点右侧按钮请求 `/models`
  - 模板条目：输入框只读，右侧按钮变成白名单展开器
  - 已拉取模型后：按钮又退化成“打开当前缓存列表”
    同一个控件在不同模式下改变可编辑性、数据来源和按钮语义，只有帮助文字在解释。
- Impact: 用户必须先推断“我现在面对的是哪一种模型选择模式”，才能决定是输入、点击、刷新还是改模板。对普通用户来说，这是明显的交互过载。
- WCAG/Standard: WCAG 2.1 A `3.2.4 Consistent Identification` 风险；同一视觉控件在不同模式下承担不同职责。
- Recommendation: 把“模型值”和“模型来源”分开。模板白名单和远端目录不应继续共用同一套输入外观；至少要把模式标签放到控件标题层，而不是塞到帮助文本里。
- Suggested command: `/harden`

### Medium-Severity Issues

#### 4. 编辑区顶部状态条仍然比字段更像“主内容”

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:380-457`, `src/features/settings/settings.css:767-782`
- Severity: Medium
- Category: Anti-Pattern
- Description: 在第一个字段之前，顶部状态条已经包含 1 段状态说明、1 个类型 chip、3 个文本动作和 1 个主按钮。其 DOM 文本密度明显高于任何单个字段。
- Impact: 页面虽然删掉了旧的 hero 卡，但还是把“编辑区入口”做成了操作横幅。用户先读“状态 + 动作”，再读字段，认知顺序仍然反了。
- WCAG/Standard: 无直接 WCAG 条款；命中 frontend-design 里“不要重复用户已能看到的信息”“不要让所有按钮都像主操作”。
- Recommendation: 顶部入口只保留最少状态，例如“未保存 / 已保存”加一个主保存动作。定位问题、恢复版本、删除条目都应弱化，不该继续占住第一屏的主注意力。
- Suggested command: `/distill`

#### 5. 视觉层级仍然偏“暗色后台工作台”，不够克制也不够直接

- Location: `src/features/settings/settings.css:798-857`, `src/features/settings/settings.css:2199-2214`, `src/features/settings/settings.css:2533-2565`
- Severity: Medium
- Category: Theming / Anti-Pattern
- Description: 编辑面板、能力面板、模板信息、目录模型卡等容器大量复用同类 `surface-raised` 深色面板和圆角边框。结构虽然统一，但主次关系不够明确，看起来像把许多“可放东西的盒子”堆进来，而不是把编辑流程做成清晰层级。
- Impact: 这就是“太丑”的主要来源之一。它不花哨，但也不克制；视觉上像一套通用后台组件，而不是为这个任务定制的设置流。
- WCAG/Standard: 无直接 WCAG 条款；违反 `.impeccable.md` 中“统一的桌面工具语言，偏克制的专业感，而不是通用 AI 工具模板味”。
- Recommendation: 收掉次级盒子数量，减少同层级容器竞争，让真正的表单面板成为唯一主层，能力说明和模板引导退回普通区块。
- Suggested command: `/quieter`

### Low-Severity Issues

#### 6. 规则解释仍然散落在四个位置，用户需要自己拼装 mental model

- Location: `src/features/settings/sections/LlmSettingsSection.tsx:485-489`, `src/features/settings/sections/LlmSettingsSection.tsx:517-519`, `src/features/settings/sections/LlmSettingsSection.tsx:835-848`, `src/features/settings/sections/LlmSettingsSection.tsx:895-995`
- Severity: Low
- Category: Clarify / Information Architecture
- Description: 条目规则目前分散在模板帮助文案、配置类型帮助文案、模型名帮助文案、能力面板说明和 RAG warning 之间。单条文案都不长，但合起来需要用户在多个块之间来回拼装：“模板会锁什么”“chat 为什么没有多模态”“embedding 为什么影响 RAG”“当前条目为什么会进入 OCR 列表”。
- Impact: 不至于阻断操作，但会持续制造“逻辑混乱”的主观感受。用户不是在填表，而是在读分散说明书。
- WCAG/Standard: 无直接 WCAG 条款
- Recommendation: 给条目建立一层稳定的能力摘要，用结构表达规则，不要再把关键规则拆到多个帮助段里。
- Suggested command: `/clarify`

## Patterns And Systemic Issues

- 这块的主要问题已经不是语义缺失，而是“模式切换过重”。单个字段会改写多项状态，但 UI 仍然把它包装成普通输入。
- 当前实现仍然在用状态条和辅助说明保护复杂逻辑，而不是先削减逻辑表面。
- 视觉问题来自层级竞争，不来自装饰。它不是太花，而是太像通用工作台。
- 条目能力与跨分组引用之间的关系是真正复杂度来源，但当前没有固定表达面，只能靠零散说明兜底。

## Positive Findings

- 基础可达性比之前好：条目目录已经是 `radiogroup + radio`，模型选择器也补上了 `combobox + listbox + option` 语义。
- 颜色系统基本收到了全局 token，没有重新滑回紫蓝渐变或 frosted glass 那套廉价 AI 审美。
- 性能层面没有看到明显问题：未发现高频布局抖动、昂贵动画或大资源误用。
- 字段级错误基本都绑定到了具体控件，而不是只在分组底部堆一份总错误。

## Recommendations By Priority

### Immediate

1. 让窄窗口下的第一个真实字段回到首屏。
2. 把 `使用模板`、`配置类型` 从“普通下拉框”升级为显式模式切换。
3. 重做 `模型名` 控件的模式边界，拆清手填、远端目录和模板白名单。

### Short-term

1. 收缩顶部状态条，只保留最低限度状态和主保存动作。
2. 减少次级面板数量，让表单面板成为唯一主层。

### Medium-term

1. 为 LLM 条目建立统一的“能力与影响范围”摘要，不再把规则拆在多个帮助块里。
2. 把条目能力变化对 OCR / AI 功能 / RAG 的影响做成结构化提示，而不是靠用户事后发现。

### Long-term

1. 为 settings 内所有“模式切换型字段”建立统一模式，避免再把高副作用选择器包装成普通输入控件。

## Suggested Commands For Fixes

- `/arrange`：重做窄窗口下的入口顺序，把第一个字段拉回首屏。
- `/clarify`：收敛模板 / 类型 / 能力 / 引用影响的规则表达。
- `/harden`：重构模型选择器模式边界，避免一个控件承载三套心智。
- `/distill`：压缩顶部状态条，把非主任务动作降权。
- `/quieter`：减少次级容器竞争，收敛“暗色后台工作台”观感。
