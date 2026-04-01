# 历史剪贴板设计记录（2026-04-01）

## 范围收敛

本阶段只实现：

- 后台监听系统剪贴板
- 只记录文本
- 固定少量 pinned 常用项
- 保留少量 recent 历史
- 在 launcher 中选择某条后，回贴到外部应用

明确不做：

- 全文搜索
- 富文本 / 图片 / 文件列表
- 多副本历史
- 设置页里的复杂配置项

## 决策

### 1. 数据模型

- 同一段文本只保留一份记录
- 条目结构包含：`id`、`text`、`pinned`、`last_seen_at_ms`、`pinned_at_ms`
- UI 快照固定拆成 `pinnedEntries` 和 `recentEntries`

### 2. 规则

- `Pinned` 与 `Recent` 共用同一份底层记录，不允许重复显示
- `Pinned` 不占用 `Recent` 容量
- 文本再次进入剪贴板时：
  - 已存在且已 pinned：只刷新 `last_seen_at_ms`
  - 已存在且未 pinned：刷新 `last_seen_at_ms`，回到 recent 顶部
- `unpin` 后按 `last_seen_at_ms` 回到 recent 排序

### 3. 容量

- `Pinned` 最多 5 条
- `Recent` 最多 10 条

### 4. 存储

- 历史数据写入 `clipboard-history.toml`
- 不引入 SQLite；当前没有搜索和复杂查询需求，数据库属于过度设计

### 5. 回贴链路

- 若 `Alt+V` 触发时 launcher 已在前台：
  - 选择历史项后把文本插入 launcher 输入框
  - 关闭剪贴板面板，但不隐藏 launcher
- 若 `Alt+V` 触发时 launcher 不在前台：
  - 选择历史项后先写系统剪贴板
  - 记录呼出前的前台应用
  - 隐藏 launcher
  - 主动重新激活原前台应用
  - 轮询确认目标应用重新接管前台焦点
  - 再发送平台粘贴快捷键

其中跨应用回贴继续由 Rust 后端驱动；launcher 内插入只更新前端输入状态。

### 6. 呼出方式

- 不新增独立“剪贴板窗口”
- 继续复用 launcher 主窗口，但历史剪贴板只走独立面板态
- 不再保留 `/clip` / `/paste` 动作入口
- 提供独立全局快捷键，默认 `Alt+V`
- 快捷键命中后由 Rust 先显示窗口，再向前端发事件直接打开历史剪贴板面板

## 推进记录

### 开始

- 2026-04-01：确认范围从“剪贴板管理器”收窄为“少量文本历史 + pinned + 外部回贴”

### 完成

- 2026-04-01：完成 Rust 侧 clipboard service / storage / IPC、launcher 浮层和基础键盘流
- 2026-04-01：移除 launcher 顶部 header 上的独立剪贴板入口
- 2026-04-01：新增独立全局快捷键 `Alt+V`，支持在其它应用上方直接呼出历史剪贴板浮层
- 2026-04-01：移除 `/clip` / `/paste` slash command，前端只保留独立面板态
