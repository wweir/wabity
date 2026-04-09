# `/kill` slash command 设计记录

## 目标

在 launcher 里新增 `/kill`：

- 支持按应用名称、进程名称或 `pid` 终止运行中的目标
- 输入阶段支持当前运行目标补全
- 尽量复用现有 launcher suggestion / action 执行链，不扩散到无关模块

## 决策

1. `/kill` 只搜索“当前正在运行”的目标，不复用已安装应用索引
2. 补全候选统一显式展示 `pid`
3. 前端确认候选后，输入框统一回写稳定 payload：`pid:<id>`
4. 执行只允许两种解析：
   - `pid:<id>`：精确终止单个进程
   - 精确名称命中单个运行目标：允许执行
5. 名称命中多个运行目标时拒绝执行，并要求用户改用补全后的 `pid`
6. 默认优雅终止：
   - Unix 使用 `SIGTERM`
   - 其他平台走库提供的普通 kill
7. 必须阻止终止当前 Wabity 自己
8. 补全搜索不在每次输入时重新全量枚举系统进程；后端首次命中时同步建立短 TTL 的运行进程快照，查询优先读取快照，过旧后再异步补刷新
9. macOS 下 `.app` 本地化显示名按 bundle path 缓存，避免在搜索热路径里反复执行 `mdls`
10. 真正执行 `/kill` 时仍重新读取最新进程列表做目标解析，不能直接信任补全缓存

## 分层落点

- `src-tauri/src/domain/process.rs`
  - 定义 `RunningProcessMatch`
- `src-tauri/src/services/process.rs`
  - 负责进程枚举、补全搜索、目标解析和终止执行
- `src-tauri/src/state/mod.rs`
  - 把 `kill_process` 作为运行时能力单独分发，不塞进纯文本 `executor`
- `src/features/launcher`
  - 新增 `/kill` 动作描述
  - 增加 `kill` suggestion mode
  - 候选选择只回写 `pid:<id>`，不直接执行

## 原因

- “应用名”和“进程名”不是同一个概念，直接复用 application 索引会把“已安装但没在运行”的目标混进来
- 按模糊名称直接 kill 风险过高，尤其是 `node`、`python`、`Electron Helper` 这类高重复名称
- 用 `pid` 作为最终 payload，前后端执行语义最稳定，也能让歧义处理显式化

## 范围外

- 不支持 `force kill`
- 不支持一次性终止多个同名进程
- 不做“最近终止目标”历史
- 不把 `/kill` 扩展成完整进程管理器
