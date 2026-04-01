# LLM Settings Remediation Log

Date: 2026-03-30
Scope: `src/features/settings/sections/LlmSettingsSection.tsx`, `src/features/settings/SettingsPage.tsx`, `src/features/settings/settings.css`
Trigger: [LLM Editor Audit](./llm-editor-audit-2026-03-30.md)

## Stage Start

目标不是重新设计整套 settings，而是按审计结论把 `LLM` 页里真正阻碍使用的结构问题收掉：

1. 把首个可编辑字段拉回首屏
2. 去掉三行按钮墙
3. 修正条目目录和模型选择器语义
4. 收紧 LLM 页残留的私有颜色和重复文案

## Implemented

- 把 `LLM` 页入口从“目录 + 草稿卡 + hero + 表单”收敛成“目录 + 紧凑状态/操作条 + 表单”
- 删除右侧 hero 摘要，避免名称 / Base URL / 模型 / 类型在目录和编辑区重复出现
- 把保存动作压缩成单一主按钮；`定位第一个问题`、`恢复已保存版本`、`删除条目` 降为文本操作
- 条目目录改成 `radiogroup + radio`，并补齐方向键切换
- 模型选择器补上 `combobox + listbox + option` 语义，并支持键盘打开候选和在 option 间移动
- 内置模板选中模型后会立即关闭候选面板
- “使用模板”“配置类型”“模型来源” 拆成显式模式区，减少把会改写边界的动作伪装成普通输入框
- 用途与能力说明收成表单内单一区块，只保留约束和可用范围，不再重复目录卡里已有的身份摘要
- 把 `720px` 下 settings 页过早堆叠的问题推迟到更窄断点，优先保证 `LLM` 编辑区首屏可见
- 参照 `AI 功能` 和 `RAG` 分组，把 `LLM` 主编辑区拆成“接入模式 / 基础连接 / 模型 / 用途与能力”四段卡片，去掉单块细长长表单
- 收掉 `LLM` 页残留的硬编码错误色和模型标签色，改走 theme token
- 同步更新 `src/features/settings/README.md` 和 `ARCHITECTURE.md`

## Stage End

这次修复只覆盖 `LLM` 页，不顺手扩改 `AI 功能`、`RAG` 或其它 settings 分组。后续如果继续做 UI 收敛，应优先复用这次沉淀下来的两条规则：

1. 主编辑入口优先暴露真实字段，不再堆重复摘要
2. 自定义选择器必须先补齐语义和键盘流，再谈视觉包装
3. 分组导航的响应式回退不能先于主编辑任务的可见性
