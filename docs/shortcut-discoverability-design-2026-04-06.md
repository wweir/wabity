# 快捷键与入口可发现性设计

## 背景

当前 launcher 的主入口是全局快捷键，但用户在以下场景里缺少清晰反馈：

- 启动快捷键注册失败后，只能从日志看问题
- launcher 空态缺少低噪音的快捷键提示

这会把“入口不可达”变成排障问题，而不是产品内可恢复问题。

## 本次目标

本轮只做三件事：

1. 启动与运行时维护快捷键注册状态，并前台暴露失败信息
2. 在 launcher 空态增加低噪音快捷键提示

明确不做：

- 新增菜单栏 / TrayIcon
- 新增首次启动 onboarding
- 扩展更多快捷键

## 方案

### 1. 快捷键运行时状态

Rust 侧 `ShortcutRuntimeState` 额外维护每个快捷键的运行时状态：

- 已保存快捷键值
- 当前是否注册成功
- 最近一次失败消息

启动注册不再因为 `toggle_launcher` 失败直接中断整个应用；失败时应用继续启动，并把主窗口拉起，让用户能直接看到错误提示并进入设置页修复。

前端通过 `get_shortcut_runtime_status` 和 `shortcut-runtime-status-changed` 消费该状态。

### 2. launcher 空态提示

仅在 launcher 真正空态时显示一行轻量提示：

- `Esc` 关闭
- 历史剪贴板快捷键
- “快捷键可在设置中修改”链接

一旦用户开始输入、出现结果或进入其它交互态，提示立即消失，不干扰主流程。

## 实现落点

- `src-tauri/src/state/mod.rs`
- `src-tauri/src/commands/launcher.rs`
- `src-tauri/src/app.rs`
- `src/lib/tauri/types.ts`
- `src/lib/tauri/client/settings.ts`
- `src/features/launcher/LauncherPage.tsx`
- `src/features/launcher/components/LauncherComposer.tsx`
- `src/features/launcher/launcher.css`

## 当前状态

- 2026-04-06：已完成首版实现，启动阶段会同步快捷键运行时状态；launcher 空态增加轻量提示。
- 2026-04-10：移除设置页“当前快捷键”总览卡，通用页只保留快捷键录制与保存入口。
