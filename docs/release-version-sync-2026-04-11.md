# Release Version Sync

## 背景

2026-04-11 的 GitHub Release workflow 因为版本源未同步而失败：

- tag: `v0.2.0`
- `package.json`: `0.1.0`
- `src-tauri/tauri.conf.json`: `0.1.0`
- `src-tauri/Cargo.toml`: `0.1.0`

这不是 action 本身出错，而是仓库版本治理失效。发布流程本来就在阻止“tag 指向一个不存在的应用版本”。

## 当前版本源

当前发版至少涉及这些版本源：

1. `package.json`
2. `package-lock.json`
3. `src-tauri/tauri.conf.json`
4. `src-tauri/Cargo.toml`
5. `src-tauri/Cargo.lock`

补充说明：

- ACP 初始化时对外暴露的 client version 不再写死，而是复用构建期注入的 `WABITY_APP_VERSION`，避免再出现代码硬编码版本漂移。
- GitHub release workflow 当前显式校验 `package.json`、`package-lock.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`；`src-tauri/Cargo.lock` 虽然不在 action 里校验，但仍必须同步，否则仓库内版本事实会继续分裂。

## 发布约束

固定顺序：

1. 先修改所有版本源
2. 本地跑最小发布前校验
3. 提交版本变更
4. 再创建并推送 `vX.Y.Z` tag

禁止顺序：

1. 先打 tag
2. 再补版本文件

这个错误顺序会导致 CI 立即失败，而且失败是正确的，不应该通过放宽校验掩盖。

## CI 打包约束

GitHub Actions 上的 macOS runner 不保证存在稳定的前台 Finder 自动化会话，因此 CI 不再依赖 Tauri 生成的 `bundle_dmg.sh` 去执行 Finder 美化。

当前策略分流为：

1. 本地手工构建：继续走 `bundle_dmg.sh` + 仓库补丁，保留背景图、图标坐标和辅助文件布局
2. CI 发版构建：只先产出 `.app`，再用 `hdiutil create` 生成简化 `dmg`

这个退化是故意的。发布 CI 的首要目标是稳定地产出可下载工件，不是把 Finder 排版美化做到极致。

## 最小操作流程

```bash
npm version <version> --no-git-tag-version

# 同步 Rust/Tauri 版本文件
# - src-tauri/tauri.conf.json
# - src-tauri/Cargo.toml
# - src-tauri/Cargo.lock

git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "build: release <version>"
git tag "v<version>"
git push origin main --follow-tags
```

## 本次处理

- 把应用版本统一提升到 `0.2.0`
- 保留 release workflow 的严格版本校验
- 移除 ACP client info 中的硬编码版本，改为复用构建注入版本
