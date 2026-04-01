# LLM 编辑卡片修复记录

日期：2026-03-31

阶段记录：

1. 开始修复

- 输入：`docs/llm-edit-card-audit-2026-03-31.md`
- 目标：只修当前编辑卡片相关问题，不扩到整个 settings 页
- 采用 skill：
  - `arrange`：压缩当前编辑入口和卡片纵向占用
  - `harden`：补模型选择器按钮打开路径的键盘焦点流
  - `clarify`：删减重复说明文案
  - `normalize`：把当前编辑区动作和块级样式收束回现有 token

2. 完成修复

- 代码范围：
  - `src/features/settings/sections/LlmSettingsSection.tsx`
  - `src/features/settings/SettingsPage.tsx`
  - `src/features/settings/settings.css`
- 文档同步：
  - `ARCHITECTURE.md`
  - `src/features/settings/README.md`

落地内容：

- 当前编辑入口改成更紧凑的单层状态条：条目名、保存状态和动作收敛在同一区域，减少首屏被状态说明挤占
- 缩短目录头、状态提示和四段编辑卡说明文案，只保留会影响决策的约束
- 弱化“卡片套卡片”观感：模式字段、模型来源和用途摘要改成更平的区块，不再继续叠次卡
- 当前编辑工具条和模板外链动作统一提升到 `44px` 触达尺寸
- 模型选择器补齐按钮打开路径的焦点流：
  - 按按钮打开时，焦点进入当前选中项或首项
  - 通过输入框方向键打开时，行为与按钮路径一致
  - 选中模型后，焦点回到输入框

未在本轮处理：

- 非 LLM 分组的说明文案压缩

## 第二轮修复（同日）

输入：`/audit LLM 配置页`

### 落地内容

**token 化清债**

- LLM provider card（border/bg/shadow/hover）全部从硬编码 rgba/hex 迁到 `--surface-*`、`--border-*`、`--accent-*` token
- combobox-panel/option/option-active/option-hover 同步迁移
- status-chip / status-chip-strong 同步迁移
- 编辑器 hero / hero-type-llm / hero-type-embedding / usage-card / model-card / capability-card 基础层全部 token 化
- combobox floating shadow 从 `rgba(...)` 改为 `var(--shadow-floating)`
- provider card focus-visible 从 `rgba(42, 92, 150, 0.45)` 改为 `var(--focus-ring)`

**减少视觉噪音**

- provider card 移除 hover translateY、多层 inset box-shadow 和 radial-gradient 装饰背景
- selected/invalid card 不再用 gradient 背景，改为纯 token surface 色
- 编辑器内部四个 panel（模板与类型、名称接入密钥、来源模型名、用途能力）从卡片样式改为 `border-bottom` 分隔的平面段落
- 每个 panel header 从 kicker + title + chip + help-text（4 行）精简为 title + chip（1 行）

**A11y**

- radiogroup 内的 `<article>` 改为 `<div>`，消除屏幕阅读器的冗余 landmark

**死代码清理**

- 移除以下未被 TSX 引用的 CSS 类（约 130 行）：
  - `settings-llm-editor-hero` 及 `-llm`、`-embedding`、`-copy`、`-meta`、`-badges` 变体
  - `settings-llm-hero-type` 及 `-llm`、`-embedding` 变体
  - `settings-llm-model-card-grid`、`settings-llm-model-card`、`-header`、`-title`
  - `settings-llm-form-grid` 及子选择器
  - `settings-llm-capability-card`、`settings-llm-capability-grid`
  - `settings-llm-capability-panel-passive`
  - `settings-llm-provider-card-model-grid`、`settings-llm-provider-card-model` 及 `-llm`、`-embedding`、`-label` 变体
- 同步清理了后半部分覆盖层和 media query 中对上述死类的引用

### 设计决策

- **拒绝双栏布局**：settings 窗口本身已足够窄（max-width 920px，扣去 sidebar 后更窄），sidebar + detail 并排会让两列都过挤。保持单列垂直堆叠。
- **编辑器 panel 不再是卡片**：这些 panel 嵌在 detail card 内部，再加 border-radius/background/shadow 只会制造"卡片套卡片"。改为 border-bottom 分隔更干净。

### 未处理

- 全 settings 页其他分区（RAG、ACP、MCP）的硬编码样式仍有残留，但已被后半部分覆盖层 token 化，暗色模式可用
- 48-prop 组件拆分属于结构重构，不在本轮范围

## 第三轮修复（同日）

输入：`LLM 配置页面，上面的 LLM 列表信息密度太低，下面的 LLM 编辑卡片，不要左右分栏`

### 文档更新

- `ARCHITECTURE.md`
  - 明确 `LLM` 条目目录必须是高密度单选清单，不允许再退回成稀疏摘要卡
  - 明确 `LLM` 编辑卡内部字段保持单列表单流，不再允许局部左右分栏
- `src/features/settings/README.md`
  - 同步实现约束，避免以后只记得“单列主流程”，却忘了“目录高密度 + 卡内单列”

### 落地内容

- 目录条目压缩成两层信息：
  - 第一层：名称 + 用途/问题徽章
  - 第二层：协议/模型 + 压缩后的接入摘要
- 编辑卡内部撤掉固定 `11rem + 1fr` 标签列，改成标签、控件、帮助、错误统一单列垂直流
- 模型字段仍保留输入框和触发按钮的局部横排，但整个字段块不再是左右分栏
- 模板外链和模型按钮统一维持 `44px` 触达高度，不再局部回退到 `40px`

### 设计决策

- 当前 settings frame 最大宽度只有 `920px`，扣掉外层 padding 和目录容器之后，LLM 编辑卡继续做左右分栏只会制造更长的视线跳跃，不会真正提升效率。
- `LLM` 目录的任务是“快速找到并切换条目”，不是“在目录里读完整介绍”；信息应该压缩成可扫的摘要，而不是继续堆三段元数据。
