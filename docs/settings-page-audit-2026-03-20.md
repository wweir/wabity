# Settings Page Audit Report

Date: 2026-03-20
Scope: settings UI in `src/features/settings`, shared tokens in `src/app/global.css`
Method: source inspection only

## Audit Premise

本次审计基于仓库当前 `settings` 实现做源码检查，没有运行时截图、自动对比度测量或真实设备录屏。因此下面结论只包含能从代码直接验证的问题，不拿猜测充数。

设计上下文来自 `.impeccable.md`：

- 用户：懂一点技术的普通用户
- 品牌气质：冷静、专业、平和
- 设计方向：统一桌面工具语言，避免玻璃拟态和 AI 模板味

## Anti-Patterns Verdict

Verdict: Fail

它已经不是最廉价的 AI 玻璃模板，但仍然保留了足够多的“安全模板味”，别人看到后说“像 AI 做的”并不奇怪。

具体迹象：

- `settings` 页面仍大量依赖重复卡片、状态 pill、摘要指标块，尤其是 RAG 摘要区直接落入“hero metric” 模板。
- LLM、ACP、MCP、Skills 四块都在复用几乎同构的卡片网格，结构上过于整齐，缺少任务优先级带来的版式差异。
- `settings.css` 里仍混有大量硬编码浅色调和后续 token 覆盖，说明视觉系统还没有真正收敛，更多是在“补样式”，不是在“用系统”。
- 文档明确要求 settings 与 launcher 共用稳定 token 和克制专业感，但当前 CSS 仍有多套来源不一致的视觉定义并存。

这不是“极简”的问题，而是“过于模板化且系统边界不干净”。

## Executive Summary

- Total issues: 8
- Critical: 0
- High: 3
- Medium: 3
- Low: 2
- Overall quality score: 71/100

Most critical issues:

1. 表单校验错误没有和具体字段建立程序化关联，屏幕阅读器无法知道哪个输入非法。
2. `settings.css` 仍存在大规模硬编码颜色和重复 selector，token 体系不是单一真相源。
3. 全局 `min-width: 520px` 仍让 settings 在窄窗口和放大字号场景下存在重排失败风险。
4. reduced motion 只禁用了滚动动画，没有覆盖设置页自己的 pulse/transition。

Recommended next steps:

1. 先修表单可访问性，再谈视觉 polish。
2. 把 settings 颜色和交互状态真正收口到 token，删掉前半段遗留样式。
3. 修窄窗口和 reduced motion，再做进一步美化。

## Detailed Findings By Severity

### Critical Issues

- None.

### High-Severity Issues

#### 1. 校验错误没有和具体字段建立程序化关联

- Location: `src/features/settings/SettingsPage.tsx:3553-3564`, `src/features/settings/SettingsPage.tsx:3895-3903`, `src/features/settings/SettingsPage.tsx:4473-4509`, `src/features/settings/SettingsPage.tsx:4797-4859`, `src/features/settings/SettingsPage.tsx:4264-4270`
- Severity: High
- Category: Accessibility
- Description: settings 页完全依赖页面上的错误列表或 section 级提示框展示问题，但字段本身没有 `aria-invalid`，也没有用 `aria-describedby` 把错误文案绑定回输入控件。全文搜索 `aria-invalid` 结果为 0。
- Impact: 视觉用户还能靠扫列表猜哪个字段错了，屏幕阅读器用户几乎只能逐个字段试错。表单有校验，但没有可访问的错误归属。
- WCAG/Standard: WCAG 2.1 A `3.3.1 Error Identification`，WCAG 2.1 A `4.1.2 Name, Role, Value`
- Recommendation: 给每个可校验字段输出稳定的错误 ID，出现错误时同步设置 `aria-invalid="true"` 和 `aria-describedby`；错误列表保留，但不能替代字段级关联。
- Suggested command: `/harden`

#### 2. token 体系不是 settings 的单一真相源

- Location: `src/features/settings/settings.css:6-1554`, `src/features/settings/settings.css:1572-1593`, `src/features/settings/settings.css:2234-2468`
- Severity: High
- Category: Theming
- Description: `settings.css` 仍残留大规模硬编码颜色和后续覆盖。简单统计显示该文件仍有 229 处 `#hex` / `rgba()` / `white` / `black` 颜色字面量。同时多个核心 selector 被重复定义，例如 `.settings-frame`、`.settings-nav-button`、`.settings-button`、`.settings-inline-code`、`.settings-select:focus`、`.settings-input.recording` 都至少出现两次以上。
- Impact: 现在之所以“看起来基本正常”，主要靠后面的 source-order 覆盖前面的遗留规则。一旦以后再插入新规则、拆文件或补主题，settings 会非常容易出现局部失真、暗色回退失败和样式回潮。
- WCAG/Standard: 设计系统一致性问题；也直接违反了 `src/features/settings/README.md` 里“settings.css 只保留特有布局，基础外观统一由 token 提供”的约束。
- Recommendation: 把 settings 视觉样式分成“保留布局/保留状态/删除遗留”三类，先删重复 selector，再把颜色全部收口到语义 token。不要继续叠覆盖层。
- Suggested command: `/normalize`

#### 3. 窄窗口重排仍受全局最小宽度限制

- Location: `src/app/global.css:204-208`, `src/features/settings/settings.css:2485-2623`
- Severity: High
- Category: Responsive
- Description: settings 已经写了 `720px` 和 `560px` 两档响应式规则，但共享 frame 仍强制 `min-width: 520px`。这意味着窄窗口策略只能在“还没窄到 520px 之前”部分生效，遇到更小宽度、系统缩放或更大字体时，容器会先卡死，再谈内部重排。
- Impact: 仓库自己要求“窄窗口下不崩”，但当前实现仍把一个硬下限放在最外层。结果是断点和真实窗口约束互相打架。
- WCAG/Standard: WCAG 2.1 AA `1.4.10 Reflow`
- Recommendation: 把共享 frame 的最小宽度降到更低，或者在 settings 场景允许覆盖该最小值；否则下面的响应式 CSS 永远不是最终裁决者。
- Suggested command: `/adapt`

### Medium-Severity Issues

#### 4. reduced motion 实现不完整，设置页自己的动画仍会继续跑

- Location: `src/app/global.css:404-410`, `src/features/settings/settings.css:1531-1545`
- Severity: Medium
- Category: Accessibility
- Description: 全局 `prefers-reduced-motion` 只把 `scroll-behavior` 置回 `auto`，但没有停用 settings 页自身的动画。快捷键录制态仍使用无限 `pulse` 动画，并通过 `box-shadow` 扩散实现。
- Impact: 对运动敏感用户来说，“系统已请求减少动态效果”不会真正生效。更差的是，这个动画不是用 `transform/opacity`，而是更重的 `box-shadow` 扩散。
- WCAG/Standard: WCAG 2.3.3 `Animation from Interactions` 相关风险；同时违反 frontend-design 里“减少动画时必须尊重 reduced motion”的要求。
- Recommendation: 在 reduced motion 下关闭所有 pulse/transition，录制态改为静态高亮或文字状态。
- Suggested command: `/harden`

#### 5. 一批主交互控件仍低于 44px 目标尺寸

- Location: `src/features/settings/settings.css:1852-1858`, `src/features/settings/settings.css:2253-2254`
- Severity: Medium
- Category: Responsive
- Description: 一级分组按钮和 `settings-button-compact` 仍只有 `40px` 高。源码里大量“新增”“填入表单”“加入内置 MCP”“查看 Skill”之类的实际主动作都挂在 compact button 上。
- Impact: 对鼠标精度一般、触屏、轨迹板、系统缩放或运动障碍用户来说，这些控件仍偏紧。你不能一边宣称窄窗口可用，一边把关键操作做成 40px。
- WCAG/Standard: WCAG 2.2 AA `2.5.8 Target Size (Minimum)` 的风险项；同时不满足常见 44x44 触达基线。
- Recommendation: 至少把 compact button 和顶部分组切换统一提升到 44px；真正非交互的 chip 才可以更小。
- Suggested command: `/adapt`

#### 6. 信息架构已经改善，但视觉组织仍过度模板化

- Location: `src/features/settings/SettingsPage.tsx:3407-3479`, `src/features/settings/SettingsPage.tsx:3964-4007`, `src/features/settings/SettingsPage.tsx:4340-4389`, `src/features/settings/SettingsPage.tsx:4660-4689`, `src/features/settings/SettingsPage.tsx:5033-5068`
- Severity: Medium
- Category: Anti-Pattern
- Description: LLM、ACP、MCP、Skills 都在使用高度相似的“左侧卡片网格 + 右侧详情”结构；RAG 摘要区还用了典型的大数字指标卡。这些都属于 frontend-design 明确点名的模板味来源。
- Impact: 从可用性角度它没有坏到不能用，但产品记忆点被磨平了。所有分组都像同一套组件生成出来的变体，而不是按任务重要性组织出来的界面。
- WCAG/Standard: 无直接 WCAG 条款；这是明确的设计反模式问题。
- Recommendation: 保留信息架构，减少重复卡片和指标块，把层级差异更多交给版式、排版和对比，而不是继续堆容器。
- Suggested command: `/distill`

### Low-Severity Issues

#### 7. 交互状态样式有收口，但文件层面的重复定义仍然过多

- Location: `src/features/settings/settings.css:94-107`, `src/features/settings/settings.css:1524-1545`, `src/features/settings/settings.css:1638-1666`, `src/features/settings/settings.css:1852-1866`, `src/features/settings/settings.css:2201-2206`, `src/features/settings/settings.css:2404-2413`
- Severity: Low
- Category: Performance
- Description: 同一控件的基础态、focus、active、token 覆盖经常分散在文件前后多个位置。最终运行时代价不大，但维护和审计成本很高，也会放大未来回归概率。
- Impact: 这是质量债，不是立即的用户阻断问题。但 settings 页继续迭代时，最容易先坏的就是这种“靠记忆维护 source-order”的 CSS。
- WCAG/Standard: 无直接 WCAG 条款
- Recommendation: 清理 selector 重复，把同一组件状态集中到同一段；不要继续在文件后半段“补丁式 override”。
- Suggested command: `/polish`

#### 8. 中英文标签混用仍然偏杂

- Location: `src/features/settings/SettingsPage.tsx:3389`, `src/features/settings/SettingsPage.tsx:3510`, `src/features/settings/SettingsPage.tsx:4986`, `src/features/settings/SettingsPage.tsx:4845-4857`
- Severity: Low
- Category: Accessibility
- Description: settings 顶层是中文产品界面，但内部同时混用 `LLM Models`、`Skills`、`Base URL`、`API Key`、`Command`、`Args`、`Env` 等标签。技术用户能懂，但整体语言层级不统一。
- Impact: 这不会立刻阻断使用，但会降低页面的完成度和认知流畅度，尤其对“懂一点技术的普通用户”来说，切换成本更高。
- WCAG/Standard: 无直接 WCAG 条款；属术语一致性和可理解性问题。
- Recommendation: 明确一条术语策略：专有协议名保留英文，字段标签尽量中文化，必要时用 “中文主标签 + 英文术语补充”。
- Suggested command: `/clarify`

## Patterns And Systemic Issues

- 颜色系统没有真正收口。settings 仍同时存在“旧浅色字面量”和“新 token 覆盖”两套来源。
- CSS 依赖 source-order 修正自身。重复 selector 过多，说明样式层正在以补丁方式演化。
- 表单错误处理是“看得见”，不是“机器能理解”。这会持续在所有分组复制问题。
- 响应式主要在内部布局层处理，但最外层 frame 还保留硬门槛，导致“局部自适应、整体不自适应”。
- 视觉语言正在从模板味退出，但仍保留过多卡片网格、状态 pill 和指标摘要块。

## Positive Findings

- 一级导航的 `tablist/tab/tabpanel` 和方向键切换是对的，语义和键盘支持都明显比旧实现成熟。位置：`src/features/settings/SettingsPage.tsx:2909-2929`
- 大多数表单仍然使用原生 `label + input/select/textarea`，没有为了“更像设计稿”牺牲可访问性。
- 非激活分组不再常驻渲染，这一点对 settings 这种大页面是正确的收敛。
- 长路径、命令和 metadata 文本普遍做了 `overflow-wrap` 或可滚动处理，没有再出现典型的路径溢出问题。
- 这次 skills 页去掉了顶部 jump rail，并补了显式“查看 Skill”动作，信息结构比之前更直接。

## Recommendations By Priority

### Immediate

1. 修复字段级错误关联：`aria-invalid`、`aria-describedby`、错误 ID、聚焦后的错误播报。
2. 处理最外层 `min-width: 520px`，让窄窗口规则真正生效。
3. 给 reduced motion 一条完整分支，停掉 pulse 和多余 transition。

### Short-term

1. 收口 `settings.css` 的 token 和颜色来源，删除前半段遗留定义。
2. 把所有 compact button 和顶层分组按钮统一到 44px。
3. 把校验错误从 section 列表下沉到字段附近，减少来回扫描。

### Medium-term

1. 重新梳理 RAG/LLM/ACP/MCP 的视觉层级，减少指标卡和统一卡片网格。
2. 整理术语体系，统一中英文策略。
3. 把同一组件的状态样式集中到单段，消除 source-order 依赖。

### Long-term

1. 给 settings 建立更明确的视觉节奏，不要继续靠“更多卡片 + 更多 chip”制造层级。
2. 让 settings 真正成为共享 token 的示范面，而不是遗留样式最多的页面。

## Suggested Commands For Fixes

- `/harden`：修字段错误关联和 reduced motion，解决 2 个可访问性中高优问题。
- `/normalize`：清理 token/颜色来源，解决 1 个高优和 1 个低优系统问题。
- `/adapt`：修 frame 最小宽度和 40px 控件尺寸，解决 2 个响应式问题。
- `/distill`：减掉模板化卡片和指标块，处理 1 个中优反模式问题。
- `/clarify`：统一设置页术语和标签语言。
- `/polish`：清理 selector 重复和样式组织混乱。
