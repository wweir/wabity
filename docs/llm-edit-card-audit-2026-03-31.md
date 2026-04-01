# LLM 编辑卡片审计

日期：2026-03-31

范围：

- `LLM` 配置页当前编辑卡片
- 重点覆盖当前编辑工具条、四段编辑卡、模型选择器，以及它们在现有 settings frame 中的布局关系

证据来源：

- 代码检查：
  - `src/features/settings/sections/LlmSettingsSection.tsx`
  - `src/features/settings/SettingsPage.tsx`
  - `src/features/settings/settings.css`
- 运行时验证：
  - 本地 `vite` 预览
  - 浏览器 fallback 场景下新增一个空白 LLM 条目，核对编辑卡片可见性、交互数量和按钮尺寸
- 约束说明：
  - 本次不是修复，只记录问题
  - 运行时截图来自浏览器 fallback，不包含桌面端窗口壳差异；但由于当前问题主要来自前端布局和交互逻辑，结论仍然有效

## Anti-Patterns Verdict

结论：`Fail`

它不是那种“紫蓝渐变 + 发光玻璃”的典型 AI slop，但当前编辑卡片仍然有明显的模板化痕迹：

- 典型的卡片套卡片：`LLM` 外层面板里再套目录卡、条目卡、当前编辑卡，再拆成四张同构编辑卡
- 过于安全的 frosted / soft-card 语言：圆角矩形、浅灰渐变、浅边框、pill 状态标签反复出现
- 主任务没有被放到视觉第一落点：用户要编辑条目，但首屏先看到的是目录说明、目录卡、状态条和动作链接
- 说明文案重复：同一层级不断解释“这里做什么”“那里做什么”，在视觉上像是 AI 为了显得完整而把说明平均撒进每一块

这页的主要问题不是“花哨”，而是“太稳妥、太碎、太像一套被不断加卡片补丁后的企业表单”。

## Executive Summary

- 总问题数：7
- 严重级别统计：`0 Critical / 2 High / 3 Medium / 2 Low`
- 最关键问题：
  - 当前编辑卡片的首个真实字段无法进入默认首屏，主任务被目录和状态层挤到折叠线以下
  - 模型选择器的可见触发按钮没有形成完整键盘路径，打开后不会把焦点送入 listbox
  - 当前编辑区存在明显的卡片嵌套和重复说明，导致层级被摊平、扫读成本偏高
  - 当前卡片样式仍残留大量硬编码颜色和渐变，主题切换依赖后续覆盖而不是组件自身收敛
- 总体质量评分：`6.4 / 10`
- 推荐下一步：
  1. 先重排信息层级，让当前编辑的第一个输入字段进入首屏
  2. 修模型选择器的键盘焦点流
  3. 收掉当前编辑区的卡片嵌套和重复说明
  4. 最后再统一清理主题 token 和弱对比文本

## Detailed Findings by Severity

### Critical Issues

- 无

### High-Severity Issues

#### 1. 当前编辑卡片的首个可编辑字段无法进入默认首屏

- Location:
  - [src/features/settings/sections/LlmSettingsSection.tsx:343](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L343)
  - [src/features/settings/sections/LlmSettingsSection.tsx:448](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L448)
  - [src/features/settings/settings.css:1881](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L1881)
  - [src/features/settings/settings.css:2116](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L2116)
- Severity: `High`
- Category: `Responsive`
- Description:
  - 当前布局先渲染目录说明和条目卡，再进入“当前编辑”工具条，然后才是四段编辑卡。
  - 在现有固定 frame 高度里，用户选中条目后，首屏只能看到“当前编辑”标题、状态和动作，真正的表单字段要继续滚动才出现。
  - 这不是偶发截图问题。浏览器运行时验证里，`.settings-main` 的可视高度约 `619px`，但内容总高达到 `2224px`，首个可编辑字段被稳定压到折叠线以下。
- Impact:
  - 主任务是“编辑当前条目”，但首屏不展示第一个输入控件，直接拉高进入成本。
  - 用户在新增条目后的第一反应应该是“开始填字段”，现在却先被迫处理状态条、动作链接和层层说明。
  - 这也削弱了你前面已经明确推进过的“字段优先、首屏可编辑”设计原则。
- WCAG/Standard:
  - 不直接命中 WCAG 条款
  - 违反当前仓库自己的 settings 设计约束：主编辑区应优先保证可编辑字段首屏可见
- Recommendation:
  - 收缩目录头和当前编辑工具条的垂直占用
  - 把“当前编辑”标题和状态压成更紧凑的一行摘要
  - 保证“接入模式”卡里的第一个控件至少在默认窗口高度下进入首屏
  - 如果做不到，就把目录区进一步折叠或切成更弱的摘要态
- Suggested command: `/arrange`

#### 2. 模型选择器通过按钮打开时没有完整的键盘焦点流

- Location:
  - [src/features/settings/SettingsPage.tsx:481](/Users/wweir/Sites/Mine/wabity/src/features/settings/SettingsPage.tsx#L481)
  - [src/features/settings/SettingsPage.tsx:575](/Users/wweir/Sites/Mine/wabity/src/features/settings/SettingsPage.tsx#L575)
  - [src/features/settings/SettingsPage.tsx:619](/Users/wweir/Sites/Mine/wabity/src/features/settings/SettingsPage.tsx#L619)
  - [src/features/settings/sections/LlmSettingsSection.tsx:829](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L829)
- Severity: `High`
- Category: `Accessibility`
- Description:
  - `focusLlmModelOption()` 存在，但只在输入框 `ArrowUp/ArrowDown` 路径里调用。
  - 通过右侧按钮打开模型列表时，`handleToggleLlmModelMenu()` 只切换 open state，不会把焦点移到 listbox 或首个 option。
  - 选项本身 `tabIndex={-1}`，因此“按 Tab 到按钮 -> 按 Enter 打开列表”不是一条完整的键盘可达路径。
- Impact:
  - 键盘用户会遇到可见触发器能打开列表，但打开后无法顺势进入选项的断链体验。
  - 这对模型选择这种核心字段不是边角问题，而是主流程的可访问性缺口。
- WCAG/Standard:
  - WCAG 2.1.1 `Keyboard`
  - WCAG 4.1.2 `Name, Role, Value`
- Recommendation:
  - 按按钮打开后立即把焦点送到当前选中项或首项
  - 关闭时把焦点回收到触发按钮或输入框
  - 保持“输入框箭头打开”和“按钮点击打开”两条路径的一致行为
- Suggested command: `/harden`

### Medium-Severity Issues

#### 3. 当前编辑区是明显的“卡片套卡片”结构，层级被摊平

- Location:
  - [src/features/settings/sections/LlmSettingsSection.tsx:343](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L343)
  - [src/features/settings/sections/LlmSettingsSection.tsx:507](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L507)
  - [src/features/settings/sections/LlmSettingsSection.tsx:509](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L509)
  - [src/features/settings/settings.css:845](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L845)
  - [src/features/settings/settings.css:893](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L893)
- Severity: `Medium`
- Category: `Responsive`
- Description:
  - 当前页面是“外层 LLM 容器 -> 目录卡 -> 条目卡 -> 当前编辑卡 -> 四张同构编辑卡”的连续嵌套。
  - 视觉语言几乎不变：同类圆角、同类边框、同类浅底、同类 pill。结果不是建立层级，而是把所有块都压成同样重要。
- Impact:
  - 用户需要花更多时间判断“这块是摘要、这块是表单、这块是辅助说明还是主动作”。
  - 页面看起来完整，但主次不清，符合 `frontend-design` 里明确禁止的 nested cards anti-pattern。
- WCAG/Standard:
  - 不直接命中 WCAG
  - 命中设计反模式：`Don't wrap everything in cards` / `Don't nest cards inside cards`
- Recommendation:
  - 把当前编辑区收敛为更少的层级
  - 优先保留一层真正承担输入任务的容器
  - 能并入字段标题的说明就不要再额外包一个模式卡
- Suggested command: `/distill`

#### 4. 当前编辑区仍残留大量硬编码颜色与渐变，主题依赖“后续覆盖”而不是组件自洽

- Location:
  - [src/features/settings/settings.css:430](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L430)
  - [src/features/settings/settings.css:845](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L845)
  - [src/features/settings/settings.css:868](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L868)
  - [src/features/settings/settings.css:983](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L983)
  - [src/features/settings/settings.css:1021](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L1021)
  - [src/features/settings/settings.css:1084](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L1084)
  - [src/features/settings/settings.css:1588](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L1588)
- Severity: `Medium`
- Category: `Theming`
- Description:
  - 当前编辑卡片相关规则里同时存在 token 化样式和旧的硬编码色值、渐变、阴影。
  - 这些旧规则目前很多被后面的 token 规则覆盖，所以未必每条都会直接漏到最终画面上；问题在于组件本身已经失去单一可信样式源。
- Impact:
  - 主题切换和后续维护会变得脆弱：一旦顺序变化、拆文件或局部复用，旧颜料就会重新漏出来。
  - 审计和调试成本显著升高，因为“当前颜色来自哪里”不再直观。
- WCAG/Standard:
  - 不直接命中 WCAG
  - 违反主题一致性与 token 收敛原则
- Recommendation:
  - 当前编辑区相关样式先做一次来源收敛
  - 删除被覆盖的旧硬编码规则，而不是继续叠补丁
  - 让编辑卡片、按钮、胶囊、toggle、文本链接都只走 token
- Suggested command: `/normalize`

#### 5. 次级动作尺寸明显小于页面自己的 44px 触达标准

- Location:
  - [src/features/settings/settings.css:1588](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L1588)
  - [src/features/settings/settings.css:2171](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L2171)
  - [src/features/settings/sections/LlmSettingsSection.tsx:467](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L467)
- Severity: `Medium`
- Category: `Responsive`
- Description:
  - 运行时测量显示，当前编辑工具条里的三个文本按钮高度都只有 `24px`：
    - `定位当前条目的第一个问题`
    - `恢复已保存版本`
    - `删除条目`
  - 同一页的主按钮明确被拉到了 `44px`，说明设计系统自己已经承认了更大的触达目标，但次级动作没有跟上。
- Impact:
  - 鼠标还勉强可用，但触控板、触屏和低精度指针下更容易误点或漏点。
  - 视觉上也像“文字链接”而不是“可执行动作”，会削弱危险操作和恢复操作的重量感。
- WCAG/Standard:
  - 不稳定触发 WCAG 2.5.8 `Target Size (Minimum)` 风险
  - 明确不符合项目自己的 `44px` 触达标准
- Recommendation:
  - 至少把编辑工具条里的动作做成最小 `44px` 高度的 text-button 或 ghost-button
  - 危险动作和“定位问题”动作不要继续共用普通行内链接语气
- Suggested command: `/adapt`

### Low-Severity Issues

#### 6. Section kicker 对比度过低，11px 小字在白底上只有 3.32:1

- Location:
  - [src/features/settings/settings.css:124](/Users/wweir/Sites/Mine/wabity/src/features/settings/settings.css#L124)
- Severity: `Low`
- Category: `Accessibility`
- Description:
  - `.settings-section-kicker` 使用 `#8a8f82`，在白底上的对比度约 `3.32:1`。
  - 这类文字尺寸只有 `11px`，而且被广泛用于当前编辑区各段卡片的上方标签。
- Impact:
  - 对低视力用户来说，这些“接入模式 / 基础连接 / 模型 / 用途与能力”分段标签会显得发灰、难扫。
  - 虽然它们是次级信息，不会直接阻断流程，但会降低信息分组的辨识效率。
- WCAG/Standard:
  - WCAG 1.4.3 `Contrast (Minimum)` 风险
- Recommendation:
  - 提升 kicker 的对比度，或者减少它对分组识别的职责，把真正的分组层级交给更清晰的标题和间距
- Suggested command: `/polish`

#### 7. 说明文案重复，当前编辑区在垂直方向上浪费了太多“解释性空间”

- Location:
  - [src/features/settings/sections/LlmSettingsSection.tsx:349](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L349)
  - [src/features/settings/sections/LlmSettingsSection.tsx:517](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L517)
  - [src/features/settings/sections/LlmSettingsSection.tsx:616](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L616)
  - [src/features/settings/sections/LlmSettingsSection.tsx:766](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L766)
  - [src/features/settings/sections/LlmSettingsSection.tsx:984](/Users/wweir/Sites/Mine/wabity/src/features/settings/sections/LlmSettingsSection.tsx#L984)
- Severity: `Low`
- Category: `Responsive`
- Description:
  - 目录头解释一次“先选条目”，每张卡片头又继续解释“这里决定什么”，模型源区和用途区还在进一步重复边界。
  - 这些说明不是完全没用，但数量已经超过当前表单密度真正需要的水平。
- Impact:
  - 页面高度被辅助说明持续拉长，直接加剧了首屏不可编辑的问题。
  - 也让页面呈现出明显的“每块都要解释自己”的 AI 模板味。
- WCAG/Standard:
  - 不直接命中 WCAG
- Recommendation:
  - 把会重复出现的规则收敛到更少位置
  - 留下真正会影响决策的约束，删掉“重复描述这块区域在做什么”的句子
- Suggested command: `/clarify`

## Patterns & Systemic Issues

- 核心问题不是单个字段，而是“主任务没有压到首屏”。
- 当前编辑区存在明显的层级通胀：每加一类信息就加一层卡、一段标题、一行说明。
- 主题层存在典型的“新 token 规则叠在旧硬编码规则上面”的补丁式演化。
- settings 页已经在多个地方强调过 `44px` 触达，但 LLM 编辑卡片的次级动作没有纳入同一标准。

## Positive Findings

- 条目目录已经用了真实的 `radiogroup / radio` 语义，而不是伪单选按钮。这是正确方向。
- 字段级错误基本都有 `aria-invalid` 和 `aria-describedby`，说明表单校验并不是“只在底部报总错误”。
- 模型字段至少已经具备 `combobox / listbox / option` 语义骨架，没有退回成纯视觉伪下拉。
- 运行时里主按钮、模型拉取按钮等核心 CTA 已经达到 `44px` 高度，说明系统级触达标准不是空谈。

## Recommendations by Priority

1. Immediate
   - 先重排 LLM 页面结构，让当前编辑卡片里的第一个字段进入默认首屏
   - 修模型选择器的按钮打开路径，把焦点送进 listbox

2. Short-term
   - 把当前编辑工具条里的文本动作统一升级到可点击面积更大的次级按钮
   - 收掉当前编辑区的卡片套卡片结构，减少层级

3. Medium-term
   - 清理当前编辑区残留的硬编码颜色、渐变和重复规则
   - 压缩重复说明文案，把垂直空间让回给真实字段

4. Long-term
   - 做一轮全 settings 页的 token 收敛，避免类似“旧颜料被新 token 遮住”的样式债继续扩散

## Suggested Commands for Fixes

- `/arrange`
  - 解决首屏不可编辑、层级拥堵、结构重排问题
- `/harden`
  - 修模型选择器的键盘焦点流和交互边界
- `/adapt`
  - 统一次级动作的触达尺寸和窄窗口表现
- `/distill`
  - 收掉卡片嵌套和冗余说明
- `/normalize`
  - 清理硬编码颜色和重复主题规则
- `/clarify`
  - 精简重复说明文案
- `/polish`
  - 修弱对比文字和末端视觉细节
