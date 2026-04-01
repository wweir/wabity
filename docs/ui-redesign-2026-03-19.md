# UI 重设计记录

日期：2026-03-19
阶段：审计问题修复 + 统一视觉重构
范围：`src/app`、`src/features/launcher`、`src/features/settings`

## 目标

- 面向“懂一点点技术的普通用户”
- 气质收敛为冷静、专业、平和
- 去掉常见 AI 工具模板味和泛滥的 frosted glass 语言
- 让 launcher 与 settings 使用同一套 UI / UX 规则

## 设计决策

### 1. 视觉方向

- 采用偏冷的中性色背景和轻度蓝灰 accent，而不是米白玻璃感
- 保留明显层级，但减少装饰性 blur、发光和悬浮噱头
- 交互面统一使用同一组圆角、边框、阴影和文字层级 token

### 2. 交互原则

- 主输入区仍是 launcher 的唯一核心操作面，不额外制造仪表盘感
- settings 导航明确按“常驻分组导航 + 当前分组块级跳转”建模，而不是一排会滚走的顶部 pill
- 小控件尺寸整体抬高，优先保证鼠标、触控板和缩放后的可点性
- 保存动作跟随当前分组标题停留在主内容区顶部，不再让用户回到底部浮动条确认
- 配置块跳转需要同时提供“可点击”和“可感知当前位置”两层反馈；窄窗口下要有持续可见的快捷跳转条
- 自定义控件必须保留真实语义；不能再用只读输入框冒充动作按钮，也不能把次级按钮塞进 button-like 卡片里

### 3. 可访问性与语义修复

- launcher 主输入框补程序化名称
- 单行补全浮层补 `combobox + listbox + option` 关系
- settings 分组切换补 `tablist/tab/tabpanel`
- timeline 中的“思考”折叠改为真实按钮并暴露展开态

### 4. 性能调整

- launcher 窗口自适应测量不再全树扫描所有后代节点
- 改为观察 shell 自身和直接浮层子节点，降低频繁输入和浮层切换时的布局测量成本

## 已实施

- `src/app/appearance.ts`：新增外观应用逻辑，统一处理主题和字号
- `src/app/App.tsx`：应用启动时读取设置并同步外观；`auto` 主题会监听系统深浅色变化
- `src/features/launcher/launcher.css`、`src/features/settings/settings.css`：frame 底色和 overlay 改为由全局 surface token 推导，只保留极小透明安全边，不再在暗色主题下叠浅色玻璃层把背景冲白
- `src/features/settings/settings.css`：设置页内部卡片、空态、安装指引、内联代码和表单动作按钮也全部切到 surface / text token，暗色主题下不再残留浅底卡片或深色标题字
- `src/features/settings/SettingsPage.tsx`：设置保存时立即应用外观；一级导航改为真实 tabs
- `src/features/settings/SettingsPage.tsx`：设置页改成左侧常驻导航、块级 jump rail 和主编辑区内的草稿操作卡，替换原先顶部 tabs + 底部保存条组合
- `src/features/settings/SettingsPage.tsx`：块级 jump rail 增加当前位置高亮，并在窄窗口下补一个紧凑跳转条
- `src/features/settings/SettingsPage.tsx`：快捷键录制改成可键盘触发的显式按钮；LLM 条目卡片拆分主选择和“设为默认”两个同级动作
- `src/features/settings/SettingsPage.tsx`：非激活设置分组不再继续常驻挂载，切换时只渲染当前分组
- `src/features/settings/useSettingsWindowFrame.ts`：浏览器态也会响应窗口变化重新收敛尺寸
- `src/features/launcher/components/LauncherComposer.tsx`：主输入区补 label、状态描述和补全关系
- `src/features/launcher/components/CompletionPopup.tsx`：补全列表补稳定 `id` 和 option 语义
- `src/features/launcher/components/SessionTimeline.tsx`：思考折叠改为 button
- `src/lib/tauri/useAutoResizeWindow.ts`：改成轻量测量策略
- `src/app/global.css`、`src/features/launcher/launcher.css`、`src/features/settings/settings.css`：统一视觉 token 与页面风格

## 2026-03-20 第二阶段

阶段：深色主题规范化 + 去噪 + 窄窗口适配

### 本轮修正

- 把全局 surface / text / primary button token 重新收敛成语义配对，减少深色主题里的“灰、闷、脏”
- launcher 顶部 picker、会话切换和动作条按钮统一移除原生 `button` 外观，修掉 WebKit 默认浅色按钮皮肤把深色 token 覆盖掉的问题
- launcher 动作条去掉额外浅底胶囊感，主按钮、翻译按钮和设置按钮直接按语义 token 区分强弱，不再保留“按钮贴在浅底贴纸上”的层级
- settings 删除重复说明块，保留“侧栏导航 + quick jump + 主内容”三段式；导航、jump rail、editor card 和表单动作全部回收到同一套深色 surface token
- launcher 最小宽度降到 `440px`，launcher / settings 在 `720px` 和 `560px` 断点下进一步放开折行和单列回流，优先保证输入区和主操作尺寸

### 2026-03-20 补充调整

- settings 的滚动容器从整页外层改为右侧主内容区，左侧分组导航不再跟随右侧配置项滚动
- 右侧 quick jump 和草稿操作卡去掉侵占式 sticky 行为，改回普通流内块；小窗口里不再持续吃掉可视高度

### 2026-03-21 补充调整

- 提示词、LLM、RAG、ACP Agent、MCP 五个分组把保存/恢复入口从分组头部下沉到主编辑区内的草稿操作卡，避免页面头部和实际编辑表单分离
- “放弃草稿”文案改成“恢复已保存版本”，明确这是回退到已落盘状态，不是删除配置

### 2026-03-28 补充调整

- RAG 页移除独立草稿状态大卡和 hero 流程卡；保存、恢复、重建和定位问题动作改为右侧主编辑区顶部的紧凑状态/操作条，首屏优先让位给 Embedding、目录和忽略规则输入
- RAG 左侧摘要卡不再使用大数字指标和后缀 chip 墙，改成更克制的摘要行和文本后缀说明，减少 dashboard 模板味
- RAG 根布局不再复用会强制单列的通用表单容器；顶部 quick jump 只保留编辑区和扫描结果，避免对首屏已可见摘要重复导航
- AI 功能、LLM 编辑区和 ACP 预设区默认都改为单列主流程；当前 settings 宽度下不再依赖长期左右分栏去承载表单、能力说明或安装提示
- 主内容区 header 不再让 quick jump 和标题说明抢同一行；jump pills 允许换行，避免继续压缩本来就窄的编辑区
- 短字段型表单恢复组件级左右排布：字段名和说明在左，`input/select/combobox` 及其帮助文本在右，用横向空间换回更短的页面高度；`textarea` 和多行目录列表仍保持上下堆叠
- 配置卡片内部的字段列表按卡片职责决定是否双列：RAG、LLM、ACP、MCP 这类编辑卡里，多组短字段可以按两列排；header、按钮区、模型选择器、`textarea`、目录列表和长说明块自动跨满整行。AI 功能页的任务卡继续保持单列任务流，不再套用共享双列
- settings 里的用途摘要、模板说明和安装提示开始去次卡化：优先平铺成普通区块，只在确实需要强调时保留卡片；quick jump 也收成更短标签和更轻的 pill，减少首屏噪声

### 验证重点

- 深色主题下 launcher 顶部按钮和 settings 导航不再出现浅色系统按钮皮肤
- launcher 主按钮、翻译按钮和设置按钮触达尺寸统一回到 `44px`
- settings 导航、jump rail、editor card 与表单控件都回到一致的深色层级，不再残留大面积浅底贴纸

## 收尾验证

- 需要继续以 `format`、`lint` 为基线确认没有引入前端回归
- 如需再次审计，优先复测 launcher 主输入框、settings tabs、600px 宽度下的 settings reflow，以及补全浮层的键盘导航
