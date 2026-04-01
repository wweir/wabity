# General Settings Audit Report

Date: 2026-03-30
Scope: `src/features/settings/sections/GeneralSettingsSection.tsx` and shared settings shell/layout it depends on
Method: source inspection + live DOM extraction + browser screenshots on local dev server (`http://127.0.0.1:1420`)

## Audit Premise

本次审计不是凭截图猜问题，也不是机械复读旧报告。

我实际检查了：

- 通用配置组件源码与共享样式
- 运行中的设置页 DOM 结构、控件尺寸、滚动容器和 aria 属性
- 浅色默认态、暗色 spot check、放大字号 spot check

设计上下文来自 `.impeccable.md`：

- 用户：懂一点点技术的普通用户
- 品牌气质：冷静、专业、平和
- 方向：统一桌面工具语言，避免玻璃拟态和 AI 模板味

## Anti-Patterns Verdict

Verdict: Fail

它已经不是“紫蓝渐变 + 玻璃卡片 + 发光描边”的廉价 AI 页面，但仍然有明显的“安全模板味”。

具体迹象：

- 左侧 rail、页头标题、顶部 quick jump 三层导航同时存在，信息重复。
- 通用页继续依赖统一的圆角卡片 + pill 导航 + 对称 split-pane 结构，过于整齐，缺少任务优先级带来的版式差异。
- 英文 kicker 加中文标题的组合在多个卡片里重复出现，像组件库默认模板，不像有明确语气控制的产品界面。
- “通知”这种线性设置也被塞进双列卡片网格，说明布局系统在主导内容，而不是内容在主导布局。

这页的问题不是“太花”，而是“太像一个谨慎的设置页生成器产物”。

## Executive Summary

- Total issues: 6
- Critical: 0
- High: 1
- Medium: 3
- Low: 2
- Overall quality score: 79/100

Most critical issues:

1. 快捷键录制按钮没有把“字段名”程序化绑定给按钮本身，屏幕阅读器只能读到当前快捷键和“重新录制”。
2. 通用页表单卡片被共享双列网格强行模板化，通知这类有主从关系的设置被并排摆放，扫描路径变差。
3. 左侧分组、页头标题和 quick jump 同时存在，导航信息重复，压缩了真正配置项的首屏空间。
4. OCR 卡头部按钮和字段标签在正常工作宽度下已经出现不自然换行，说明布局余量不健康。

Recommended next steps:

1. 先修快捷键录制的可访问命名。
2. 把“通知 / 外观 / OCR”从统一双列模板里解放出来，按内容语义重排。
3. 删除一层重复导航，给首屏留更多配置空间。

## Detailed Findings By Severity

### Critical Issues

- None.

### High-Severity Issues

#### 1. 快捷键录制按钮没有程序化绑定字段名

- Location: `src/features/settings/settingsShared.tsx:219-249`, `src/features/settings/sections/GeneralSettingsSection.tsx:143-160`
- Severity: High
- Category: Accessibility
- Description: 快捷键名称“打开启动器 / 翻译选中文本，未选中时 OCR”渲染在按钮外侧的 sibling `<span>` 中；按钮本身只有 `aria-describedby`，没有 `aria-labelledby` 或 `aria-label`。运行中的 DOM 也验证了这一点：按钮只有自己的内容 `Alt+Space / 重新录制`、`Alt+D / 重新录制`，没有字段语义。
- Impact: 视觉用户能看懂两行布局，屏幕阅读器用户却很难区分这两个按钮分别是在录哪个快捷键。它不是“提示不够友好”，而是控件名称本身不完整。
- WCAG/Standard: WCAG 2.1 A `4.1.2 Name, Role, Value`
- Recommendation: 给快捷键标签生成稳定 ID，并用 `aria-labelledby` 把按钮与“字段名 + 当前值/动作文案”绑定；`aria-describedby` 继续保留给说明和状态。
- Suggested command: `/harden`

### Medium-Severity Issues

#### 2. 共享双列表单模板过于粗暴，通知卡的主从关系被打散

- Location: `src/features/settings/settings.css:966-972`, `src/features/settings/settings.css:1002-1034`, `src/features/settings/sections/GeneralSettingsSection.tsx:165-319`
- Severity: Medium
- Category: Responsive
- Description: 任何包含 `.settings-item` 的 `settings-editor-card` 都会自动进入 `repeat(2, minmax(0, 1fr))` 双列网格。结果是“通知”这种应该顺着读下去的设置被横向拼接成两列：主开关旁边就是依赖它的“通知内容”，后面两个通知细项又并排出现。
- Impact: 这会降低扫描效率，也会削弱依赖关系。用户需要先理解布局规则，才能理解设置关系。对放大字号和窄宽度场景来说，这种“先模板、后内容”的布局更脆弱。
- WCAG/Standard: 无直接 WCAG 条款；属于信息组织和 reflow 风险
- Recommendation: 不要让共享卡片规则决定所有分组布局。通知、外观这类线性设置应默认单列，只有真正需要并列比较的内容才使用双列。
- Suggested command: `/arrange`

#### 3. 三层导航同时存在，首屏信息重复且模板味明显

- Location: `src/features/settings/SettingsPage.tsx:2111-2162`, `src/features/settings/settingsShared.tsx:24-30`, `src/features/settings/settings.css:1690-1775`, `src/features/settings/settings.css:1900-1916`
- Severity: Medium
- Category: Anti-Pattern
- Description: 左侧 rail 已经表达“当前在哪个分组”；进入通用页后，主区又重复渲染“通用 + 描述”；紧接着再来一排 quick jump pills 重复“快捷键 / 通知 / 外观 / OCR”。这不是信息层级，是重复堆叠。
- Impact: 首屏视觉注意力大量消耗在导航自身，而不是具体设置项。对一个配置页来说，这是典型的“框架比内容更吵”。
- WCAG/Standard: 无直接 WCAG 条款；违反 frontend-design 中“不要重复用户已经看到的信息”
- Recommendation: 左侧 rail 和页内 quick jump 二选一保强，另一层降级或删除；如果保留 quick jump，就让页头只承担标题，不要再重复一套导航语言。
- Suggested command: `/distill`

#### 4. OCR 卡头部动作和字段标签在正常宽度下已经出现不自然换行

- Location: `src/features/settings/sections/GeneralSettingsSection.tsx:329-399`, `src/features/settings/settings.css:321-337`, `src/features/settings/settings.css:357-387`, `src/features/settings/settings.css:981-985`
- Severity: Medium
- Category: Responsive
- Description: `保存 OCR 配置` 按钮放在卡片 header 右侧，但 header 只是普通 `flex` 分布，没有约束按钮禁止收缩，实拍中按钮文本已经被压成两行。字段区也依赖固定列宽，导致“识别 Provider”这类标签在当前工作宽度下就出现别扭断行。
- Impact: 这说明布局余量已经不足。现在只是“难看”，接下来一旦文案略长、字号变大或增加一个说明句，就会迅速变成真正的 reflow 问题。
- WCAG/Standard: WCAG 2.1 AA `1.4.10 Reflow` 风险项
- Recommendation: OCR 头部动作改成不收缩按钮，必要时下沉到卡片底部或独立 action row；字段标签列不要对中英混合文案使用过窄固定宽度。
- Suggested command: `/adapt`

### Low-Severity Issues

#### 5. 术语和语言层级不统一，增加不必要认知噪音

- Location: `src/features/settings/settingsShared.tsx:27-30`, `src/features/settings/sections/GeneralSettingsSection.tsx:130-140`, `src/features/settings/sections/GeneralSettingsSection.tsx:172-180`, `src/features/settings/sections/GeneralSettingsSection.tsx:279-280`, `src/features/settings/sections/GeneralSettingsSection.tsx:336-339`, `src/features/settings/sections/GeneralSettingsSection.tsx:382-404`
- Severity: Low
- Category: Accessibility
- Description: 页面主体是中文，但卡片 kicker 用 `Shortcuts / Notifications / Appearance / OCR`，字段里又混入 `LLM 条目`、`识别 Provider`。这不是术语精确，而是语言策略没有收敛。
- Impact: 对懂一点技术的用户来说，这不会造成理解失败，但会持续制造“界面没有统一想好怎么说”的感觉，降低完成度。
- WCAG/Standard: 无直接 WCAG 条款；属于可理解性和术语一致性问题
- Recommendation: 明确术语策略。协议名保留英文，普通界面语统一中文，必要时采用“中文标签 + 英文术语补充”。
- Suggested command: `/clarify`

#### 6. 视觉语言仍偏保守模板，而不是明确的桌面工具表达

- Location: `src/features/settings/SettingsPage.tsx:2111-2162`, `src/features/settings/settings.css:1666-1671`, `src/features/settings/settings.css:1731-1775`, `src/features/settings/settings.css:2467-2495`
- Severity: Low
- Category: Anti-Pattern
- Description: 当前页面大量依赖圆角容器、浅描边、均质化 spacing 和 pill 导航。它已经比典型 AI UI 克制很多，但仍然更像“安全的设置模板”，而不是一个真正做过任务优先级取舍的桌面工具页面。
- Impact: 不会直接伤害可用性，但会削弱产品记忆点，也会让“通用配置”这种高频页面显得过于中性。
- WCAG/Standard: 无直接 WCAG 条款
- Recommendation: 不要继续加更多小卡片和 pill。优先通过版式、标题层级、留白和动作位置建立重点。
- Suggested command: `/critique`

## Patterns And Systemic Issues

- `settings-editor-card` 的共享布局抽象过粗，正在让所有分组被同一种双列模板支配。
- settings 页当前更擅长“搭一个整齐的框架”，不擅长“让当前分组内容自己说话”。
- 可访问性基础比旧版好，但快捷键这种自定义控件还没有把语义闭环做完整。
- 视觉语言整体克制，但仍然带着明显的组件模板感，而不是内容驱动的层级差异。

## Positive Findings

- 左侧分组导航使用了正确的 `tablist / tab / tabpanel` 语义，并实现了键盘切换。位置：`src/features/settings/SettingsPage.tsx:2113-2140`
- 触达尺寸基线比之前健康：按钮、quick jump、nav button 都统一到了 `44px` 级别；实测快捷键录制按钮也是 `44px` 高。位置：`src/features/settings/settings.css:2415-2430`
- reduced motion 已经补齐到 settings 范围，而不是只停掉滚动动画。位置：`src/features/settings/settings.css:2870-2876`
- 暗色主题 spot check 没看到明显的 token 断裂或局部浅色残留，说明 light/dark 共享 token 现在基本一致。
- 通用页没有发现单独成立的性能问题：没看到明显的布局抖动、昂贵动画或资源型浪费。

## Recommendations By Priority

### Immediate

1. 修快捷键录制按钮的可访问命名，把字段语义和按钮绑定起来。
2. 取消“通知 / 外观 / OCR”对共享双列模板的被动继承，按内容语义重排。
3. 处理 OCR 头部动作与字段标签的换行问题，避免现有宽度下就显得吃紧。

### Short-term

1. 删除一层重复导航，让首屏优先显示可配置内容。
2. 统一通用页的语言策略，清理中英混用的非必要位置。

### Medium-term

1. 重新定义 settings 的视觉节奏，不要让 pill 和 rounded card 成为默认答案。
2. 把“共享布局规则”从全局粗粒度回收到更具体的内容场景，减少模板绑架。

### Long-term

1. 让 settings 成为“内容优先”的示范页面，而不是“组件模板最完整”的页面。

## Suggested Commands For Fixes

- `/harden`：修快捷键录制按钮的可访问命名和控件语义。
- `/arrange`：重排通知 / 外观 / OCR 的表单结构，去掉不必要的双列模板。
- `/adapt`：处理 OCR 区域在当前宽度和大字号下的换行与重排。
- `/distill`：删除重复导航层，减弱模板感。
- `/clarify`：统一术语与语言层级。
- `/critique`：进一步审视这页的视觉语言是否还停留在“安全模板”。
