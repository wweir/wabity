# settings feature

设置页负责渲染并维护 Wabity 的本地配置 UI：通用偏好、功能绑定、模型资源、知识库、Agent 配置和关于信息。

## 文档入口

- [`SETTINGS_IA.md`](./SETTINGS_IA.md)：页面信息架构、一级导航、分组职责、跨页依赖模型。
- [`SETTINGS_BEHAVIOR.md`](./SETTINGS_BEHAVIOR.md)：保存语义、后端命令、模型 / OCR / 知识库 / Agent / MCP 行为规则。
- [`SETTINGS_UI.md`](./SETTINGS_UI.md)：视觉层级、布局、响应式、表单和组件模式。

## 文件边界

- `SettingsPage.tsx`：页面壳层、状态编排、持久化、校验和导航行为。
- `SettingsSectionViews.tsx`：section 导出入口。
- `sections/*.tsx`：各一级分组的展示组件。
- `sectionViewShared.tsx`：section 之间共享的轻量展示 helper 和字段 ref 类型；不承载跨分组状态源。
- `settingsShared.tsx`：跨分组复用的元数据、共享辅助组件和展示 helper。
- `settingsState.ts`：默认状态、草稿快照、配置校验和纯函数状态转换。
- `settingsTypes.ts`：settings feature 内部类型、草稿校验类型和本地默认值。
- `useSettingsSectionNavigation.ts`：一级分组 tab 键盘流、块级 jump 和主内容滚动观测。
- `useSettingsPersistence.ts`：分组保存、草稿恢复和落盘后的本地状态对齐。
- `useSettingsWindowFrame.ts`：设置窗口尺寸、frame 和桌面壳适配。
- `settings.css`：样式入口，只导入 `styles/` 子文件。
- `styles/settings.*.css`：设置页特有布局、表单状态、卡片变体、LLM 局部规则和响应式规则。

## 开发入口

常见修改位置：

- 改导航标签、quick jump 或分组说明：`settingsShared.tsx`、`SettingsPage.tsx`，并同步 [`SETTINGS_IA.md`](./SETTINGS_IA.md)。
- 改保存语义、草稿恢复、跨页引用：`useSettingsPersistence.ts`、`settingsState.ts`，并同步 [`SETTINGS_BEHAVIOR.md`](./SETTINGS_BEHAVIOR.md)。
- 改具体分组表单：`sections/*.tsx`。
- 改视觉样式或响应式：`styles/settings.*.css`，并同步 [`SETTINGS_UI.md`](./SETTINGS_UI.md)。

## 验证

设置页改动至少运行：

```bash
bun run build
bun run lint
node /Users/wweir/.agents/skills/impeccable/scripts/detect.mjs --json src/features/settings
```

命名和迁移类改动还应按计划文档中的 stale-label patterns 检查旧标签、旧 RAG 主操作、旧 Agent 暴露文案是否残留。不要把这些旧文案重新写进 feature-local 文档。
