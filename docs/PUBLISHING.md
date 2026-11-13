# 手动发布到 GitHub

## 创建仓库并上传源码

在 GitHub 创建空仓库，建议名称 `WuwaTappo-Full-Community`。不要勾选自动生成 README、License 或 .gitignore，本地已包含这些文件。复制 GitHub 显示的仓库 URL，并在项目根目录执行：

```bash
git remote add origin <你的仓库URL>
git push -u origin main
git push -u origin official-1.0.3
git push -u origin dev
```

如果已经设置 `origin`，先用 `git remote -v` 检查，不要重复添加。仓库默认分支设为 `main`；`main` 已包含官方 1.0.3 适配。`dev` 从 `main` 派生，供持续开发和自动开发版发布；`official-1.0.3` 保留为历史适配分支。

`release-assets/` 存放本地构建的 1.2.1 与 1.2.2-alpha.1 安装包和源码归档，已加入忽略规则。`dist/`、官方 EXE、解包前端、node_modules、Rust target、缓存和日志均不应推送到 Git 历史。安装包用 GitHub Release 附件分发。优先用上面的 Git 命令上传，以保留两个分支；网页上传时仅使用已整理的源码归档内容，不要上传整个本地项目文件夹。

## 启用 CI 和官方更新检查

1. 在 Actions 页面启用工作流。
2. 如使用官方更新 PR 自动化，到 Settings → Actions → General 开启 **Allow GitHub Actions to create and approve pull requests**；只使用普通构建无需此设置。
3. 手动运行 CI，确认从空缓存也能准备前端、测试并打包。
4. 每日检查只发现并验证候选更新，不自动合并或发布。

本地也可以在适配分支执行：

```bash
npm run official:update                 # 只检测官方版本
npm run official:update -- --apply      # 下载、适配、测试、构建并打包
```

`--apply` 失败会恢复 `upstream-cn.json` 和已提取前端；未知官方版本会在 FullAtlas 版本 / SHA-256 / 契约门禁处停止。构建成功后检查 `macos/.work/official-update-result.json`，再提交资源锁和适配代码。不要把 Windows EXE 或临时构建缓存上传到源码仓库。

构建工作流的下载镜像指向当前仓库自己的 `upstream-resources` Release。没有镜像时首先从官方源下载；不要复制别人的凭据或私有下载链接。

## 发布 1.2.1 预览版

先在本机验收登录、仅本次会话、输入权限、图鉴开关、助手开关以及普通联机；自动测试不能替代这些检查。确认准备发布的源码位于 `main` 后：

```bash
git switch main
git tag v1.2.1
git push origin v1.2.1
```

Release 工作流构建完两个平台后**直接发布**。要人工把关就在推标签之前做完——推出去就没有草稿缓冲区了（1.2.3 之前是草稿 + 预发布，需要手动点 Publish）。

如果不使用 Actions，也可在 GitHub 手动创建 `v1.2.1` 预发布，附上 `release-assets/` 内的两个文件，并使用 [发布说明](RELEASE_NOTES_1.2.1.md)。手动创建不会补齐四件产物，也没人替你校验版本号，只适合应急。这两个本地产物来自迁移前的 1.0.2 整合构建，不是 1.0.3 分支产物；源代码后续若有行为变更，必须重新构建附件。

## 后续版本

新版本需要同步 `package.json`、`package-lock.json`、Cargo、Tauri 配置与助手显示版本，并重新完成测试和实机验收。官方版本锁记录的是官方 Windows 资源版本，与整合版发布版本独立。对未知官方版本不要绕过版本 / SHA-256 门禁。

## 发布整合版

当前整合版版本号为 **1.2.8**，适配内容和待验收项见 [实验记录](macos/OFFICIAL_1.0.3.md)。`dev` 从 `main` 派生并持续构建。

**发版只能靠推标签触发，不要在网页上手动创建 Release。** GitHub 对「创建 Release 时顺带新建的 tag」不发 `push` 事件，工作流根本不会启动；手建的同名 Release 还会占住列表位置，让自动产出的附件藏进草稿里看不见。正确顺序：

```bash
# 1. 版本号必须和 package.json 一致；package-lock / Cargo / 两份 tauri.conf 一起改
# 2. 提交并推送
git add -A && git commit -m "release: 1.2.3" && git push
# 3. 推标签，触发 Release 工作流
git tag v1.2.3 && git push origin v1.2.3
```

推完标签就不用再动手：`publish` 会构建两个平台、校验四件产物齐全，然后直接发布。版本号决定发布形态：

| 标签 | Release 状态 |
|---|---|
| `v1.2.3` | 正式发布，自动成为 Latest |
| `v1.3.0-alpha.1` / `-beta.1` / `-rc.1` | Pre-release，不抢 Latest |

四件产物缺一不可：

`WuwaTappo-Full-CN-Community-<ver>-macOS-arm64.dmg`、`WuwaTappo-Full-Global-Community-<ver>-macOS-arm64.dmg`、
`WuwaTappo-Full-CN-Community-Windows-x64-<ver>.exe`、`WuwaTappo-Full-Global-Community-Windows-x64-<ver>.exe`。

标签名与 `package.json` 版本不一致、或缺任何一个平台产物，`publish` 会直接失败，不会产出半套。

标签已经存在但工作流没跑（或跑在旧版工作流上），用 `Actions → Release → Run workflow` 填标签补跑。同 tag 的 Release 已存在时，重跑会补齐文件、同步说明并把草稿转正，所以「网页上手建错了」也能靠补跑修回来。

`dev` 分支每次 CI 成功后会自动更新 [dev 预发布](https://github.com/alexzhong233/WuwaTappo-Full-Community/releases/tag/dev)，与上面这条正式发版链互不影响。
