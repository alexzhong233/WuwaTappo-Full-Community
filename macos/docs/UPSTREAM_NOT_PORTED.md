# 上游功能「没搬过来」的详情备份

对照快照：`Averionx/WuwaTappo-FullAtlas` @ `98f5ded`（1.3.8，2026-11-06 快照）。
本地基线：本仓库 `main`（控制台页签 / 右键菜单 / 更新进度 / 自启托管已落地）。
用途：以后有人问「上游那个功能我们怎么没有」，先看这份，别再重扫一遍源码。

结论先说：**上游有、我们既没有也没等价实现的，是 0 项。** 下面 8 条都是「单 exe 架构的产物」或「代码拆法不同」。

---

## 1. 独立的「更新中」置顶小窗口

**上游位置**
- `src/assistant_dashboard.js:53` `createUpdateProgress(ui,parent)`：进度条组件（label + track + fill + detail）
- `src/assistant_dashboard.js:170-249` `createAssistantUpdateGate(d)`：更新窗口整块 UI 与置顶逻辑
- `src/desktop_assistant.js:104-121` `applyUpdateStatus(result)`：决定要不要开这个窗口
- 窗口本体由原生端命令 `open_update_window` 创建（注入点在 `tools/integrate_assistant.py:121`）

**上游行为（细节）**
- 第二个原生窗口，加载同一份前端，`location.hash` 为 `#/update`；尺寸固定 500×560，不可缩放，没有最小化/最大化。
- 关闭一律拦死：`onCloseRequested` 全部 `preventDefault()`，只能点「退出软件」离开。
- 位置锁：记录初始 `outerPosition`，`onMoved` 里一旦发现被拖动就 `setPosition` 弹回原位。
- 置顶：`setAlwaysOnTop(true)` + `onFocusChanged` 失焦立刻重新置顶 + `setInterval(keepOnTop, 500)` 双保险，防止被桌宠或游戏窗口盖住。
- 键盘：`Escape` 被吞掉；`Tab` 只在「立即更新 / 退出软件」两个按钮间循环（焦点陷阱）。
- 内容：标题「发现新版本，请更新」、`当前 vX → 最新 vY`、独立滚动区的更新说明、「账号、点数、装扮、开箱记录与设置均会保留」、两个按钮。
- 进度文案在 `downloading` 阶段显示 `12.3 / 80.0 MB · 15%`，其余阶段只显示百分比。
- `preview` 模式：窗口照开，但 `required=false` 时标题改「更新弹窗预览」，不触发下载，用来预览样式。
- 进度数据来自 `/update/status` 的 `installation`，且必须 `installation.version === result.version` 才显示，防止上次任务串台。
- 所有原生 API 调用失败不中断，收集到 `failed[]` 后 POST `/diagnostic`（`kind: update_window_setup`）上报。

**我们现在**
进度条做在面板「更新」页里（`.wt-console[data-embedded=true]` 内嵌模式），2 秒轮询 `/update/status`，含阶段文案、百分比、剩余大小；桌宠退出重启后进度仍能读回。

**为什么不搬**
上游是「单 exe，主窗口就是桌宠本体」，更新时主窗口要被替换，只能在旁边另开一个窗口显示进度。我们是「本地服务 + 面板窗口」，面板一直活着，再开窗口只是多一个要维护的原生窗口和一套置顶/焦点陷阱逻辑。

**将来要搬的话**
Rust 侧加 `open_update_window` 命令（always-on-top、无装饰、固定尺寸、禁 resize），前端 `createUpdateProgress` 可直接复用。mac 用 `tauri::WindowBuilder`，注意 `onFocusChanged` 在 mac 上是 `WindowEvent::Focused`。

---

## 2. 强制更新门禁

**上游位置**
- `src/desktop_assistant.js:11` `updateGate / lastUpdateStatus / previewLatched`
- `src/desktop_assistant.js:104-121` `applyUpdateStatus`
- `src/desktop_assistant.js:112` 创建 `createAssistantUpdateGate`
- `src/desktop_assistant.js:181-186` `pollPolicy()`（窗口开着时轮询从 15 秒变 500 毫秒）
- `src/local_host.py:224` `ASSISTANT.update_required = result.get('required') is True`

**上游行为（细节）**
- `/update/check` 返回 `required=true` 时，宿主把 `update_required` 置真。
- 桌宠主循环每秒 `tick` 一次，见到 `result.update_required` 而本地状态还是假，就补拉一次 `/update/status` 并弹窗。
- `updateRequired` 为真时全面停摆：不按 F24（`if(result.allowed && !updateRequired.value)`）、`listenJoin` 拦掉进房、`joinHistoryRoom` 直接抛「请先完成版本更新」。
- 窗口只能靠退出软件关闭，等于强制中断使用。

**我们现在**
`local_host.py` 里 `ASSISTANT.update_required = False` 硬置；`/update/check` 的 `required` 恒为 false；面板只提示「有新版本」，功能不锁。

**为什么不搬**
这是上游单 exe 自更新链的保险丝——升级包没别的兜底，只能靠锁死逼用户升级。我们的更新是可选更新，一旦升级包出问题，锁死等于把用户按在不可用状态里。

**将来要搬的话**
`desktop_updater.check()` 本来就算 `required` 字段，只是没用；前端 `updateRequired` 那套判断可以整体移植。前提是先有「更新失败可回滚」和「离线可用」两条，否则不建议打开。

---

## 3. 官方公告 HTML 解析

**上游位置**
- `src/desktop_updater.py:27-58` `class ReleaseNotesParser(HTMLParser)`
- `src/desktop_updater.py:346` 调用点（拿不到纯文本时回退）

**上游行为（细节）**
- 只吃 `class` 含 `markdown-body` 的 `<div>` 子树，用 `depth` 计数进出，`complete` 标记要求标签闭合完整才返回文本（防止截断的半截页面）。
- `p / li / h1-h4 / br / tr` 转成换行，`li` 额外加 `• `。
- 正文超 32000 字符截断。
- 解析失败或没有 `markdown-body` 就回退到 Markdown 版 `release_notes_text(body)`。

**我们现在**
`desktop_updater.release_notes_text(body, limit=40)`：剥掉 Markdown 表格、图片、链接、裸 URL，压成纯文字，最多 40 行。

**为什么不搬**
我们发 Release 的 body 一直是 Markdown。上游要解析官方公告页，是因为它跟库洛版本同步发布说明，说明来自官网 HTML。

**将来要搬的话**
那 57 行可以直接抄进 `desktop_updater.py`，在 `release_notes_text` 开头加一个分支：`body` 里出现 `<div class="markdown-body">` 就走 HTML 解析。

---

## 4. 更新前的「请主程序保存退出」握手

**上游位置**
- `src/local_host.py:233-239` `ready_for_restart(result)` / `stop_for_update()`
- `src/desktop_updater.py:363` `start_install(on_ready)`
- `src/desktop_updater.py:541` `report(phase,percent,message)`

**上游行为（细节）**
- `/update/install` → `start_install(callback)`：
  1. 先 `read_progress` 看有没有非终态任务，有就直接把旧任务返回（幂等，页面刷新不会起第二个）。
  2. 新建 `.updates/<uuid4>` 目录，写 `queued 0%`「准备下载更新…」。
  3. 写指针 `.updates/install-current.json` = `{job, version}`。
  4. 起 daemon 线程跑 `prepare(folder)`，失败写 `failed`「更新失败：…。原程序与数据保留。」
- `prepare` 完成后回调 `ready_for_restart(result)`：先 `ASSISTANT.handle('quit')` 让桌宠退出，再 `threading.Timer(8, stop_for_update)` 给 8 秒缓冲，然后 `ASSISTANT.save(True)` 落盘状态 + `server.shutdown()` 关掉服务，最后由 `--apply-update` 接管替换。
- 8 秒这个数写死，是留给「最后几笔统计写完」的窗口。

**我们现在**
`apply_update()` 由 `--apply-update` 子进程做（stop → replace → start → complete/failed 五阶段都写进度），主程序收到指令自己退出。`install.lock` 的 `msvcrt.locking` 已经保证同一时刻只有一个任务。

**为什么不搬**
架构决定。上游有独立的 `Upgrade.exe` 去替换文件，所以必须协商「你什么时候保存完、什么时候能动手」。我们同一个 exe 兼任宿主和替换器，没有第三方进程需要握手。

**将来要搬的话**
只有当更新链改成「独立升级器进程」时才需要，届时应做成 `/update/prepare-restart` 路由，语义是「宿主确认可安全退出」，而不是现在的「命令宿主退出」。

---

## 5. 绿色版启动引导

**上游位置**
- `src/portable_bootstrap.py`（180 行）

**上游行为（细节）**
- 单 exe 自解压/就地展开：`ROOT_FILES`（10 个文件）+ `ROOT_DIRS`（`_internal`、`runtime`、`cache`、`docs`、`licenses`）白名单。
- `CRITICAL` 六项（两个 exe、`launch.ps1`、`_internal/python312.dll`、`runtime/msedgewebview2.exe`、`cache/atlas.json`）缺一个就判「安装损坏」。
- `version()` 校验版本串必须是 2-4 段纯数字。
- `popen_independent` 起真正主程序，`native_is_running` 防重复启动。
- 带 zip 解包、sha256 校验、临时目录清理。

**我们现在**
Inno Setup 安装到固定目录，`launch.ps1` + `WuwaTappoLauncher.exe` 负责启动。

**为什么不搬**
绿色版会跟现有三件事直接冲突：计划任务自启路径、本机服务固定端口、`offline_state.json`/`cache` 的落盘位置，全都按安装目录写死。要支持便携版等于再开一条发行线。

---

## 6. 独立进程启动（DLL 搜索路径隔离）

**上游位置**
- `src/process_launch.py`（29 行）

**上游行为（细节）**
- `clean_dll_search_path()`：`GetDllDirectoryW` 存原值 → `SetDllDirectoryW(None)` 清空 → `yield` → 结束时还原。
- `popen_independent()`：额外给子进程设 `PYINSTALLER_RESET_ENVIRONMENT=1`，避免继承 `_MEIPASS` 等变量，导致子进程按父进程的临时目录去找 DLL。

**我们现在**
`apply_update` 起子进程时直接用 `subprocess`，没做这两步。

**为什么不搬**
只有 onefile 打包才会踩到——`_MEIPASS` 指向临时解压目录，父进程一退出目录就没了。我们是 onedir。

**风险提示**
如果哪天改成 onefile 发行，这 29 行必须补。否则「替换后的新程序」可能加载到旧 dll，表现为更新后功能异常但版本号是新的。

---

## 7. 房间历史 / 收藏房间 / 从历史记录进房

**上游位置**
- `src/desktop_assistant.js:163-171` `roomHistory` / `favoriteRoom` / `deleteRoom` / `joinHistoryRoom`
- `src/desktop_assistant.js:197` 注册进对外 API

**上游行为（细节）**
- `roomHistory()` → `/assistant/state` 的 `rooms`
- `favoriteRoom(code, favorite)` → `/assistant/favorite` 返回新列表
- `deleteRoom(code)` → `/assistant/delete-room`
- `joinHistoryRoom(code)` 三道守卫：本地全图鉴开着 → 抛「请先关闭本地全图鉴」；`updateRequired` → 抛「请先完成版本更新」；`!/^[A-Za-z0-9-]{1,32}$/` → 抛「房间号无效」，最后 `emit('local-assistant-join', code)`

**我们现在**
功能齐：列表、收藏、删除、加入都在面板联机页里；只是没拆成这四个具名 API，加入房间的逻辑写在面板内联 handler 里。

**为什么不算缺口**
纯代码组织差异，拆出来反而要维护两套入口。功能对得上。

---

## 8. 更新进度读写独立模块

**上游位置**
- `src/update_progress.py`（209 行）：`UpdateLease`、`write_progress`、`read_progress`、`TERMINAL`

**上游行为（细节）**
- `UpdateLease`：`.updates/install.lock` 上做跨进程独占（Windows `msvcrt.locking`，其余 `fcntl.flock`），抢不到抛「已有更新任务正在运行，请等待完成。」
- `write_progress`：先写唯一临时文件再 `os.replace`，失败重试（Windows 上读者占用会导致改名失败）。
- `read_progress`：`install-current.json` 指针 → `progress.json` → 合并 `result.json`，超时视为过期。

**我们现在**
同一套语义已经写在 `desktop_updater.py` 里：`_install_stream`（等价 `UpdateLease`）、`write_progress`（临时文件 + 2 秒重试）、`read_progress`（900 秒过期）。

**为什么不搬**
上游拆成单独文件是因为独立升级器进程也要 import 它。我们只有宿主一个进程读，塞在一起依赖更少。

---

## 附录 A：本轮已落地的四项（防止以后重复搬）

| 功能 | 落点 |
| --- | --- |
| 控制台升级为官方面板页签（联机下方、设置上方） | `windows/src/tools/build_integration.py`（`__consoleTabs` / `__ConsolePage` / 三段滑杆图标）、`integrate_assistant.py`（放行更新锁与页签白名单）、`desktop_assistant.js`（`mountConsole`） |
| 桌宠右键原生菜单 | `windows/src/src/desktop_context_menu.js`、`native_context_menu.py`、`local_host.py` 的 `/assistant/context-menu` |
| 更新页版本三行 + 进度 + 更新说明 | `desktop_updater.py`（`release_notes_text` / `write_progress` / `read_progress` / `_install_stream`）、`assistant_dashboard.js`（进度条 + notes + 2 秒轮询） |
| 开机自启托管 | `windows/src/src/startup_manager.py`、`local_host.py` 的 `/startup` |

打包注意：`native_context_menu` 与 `startup_manager` 是在 `try/except` 里导入的，`windows/src/build.py` 必须保留对应的 `--hidden-import`，否则发行包里会静默退化成「没菜单 / 退回官方自启开关」。

## 附录 B：怎么重跑这个扫描

上游快照在 `.work/upstream-138/tree/Averionx-WuwaTappo-FullAtlas-98f5ded/`（不入库）。重扫：

```bash
# 1. 列出上游所有顶层函数/类，排除本地已有的
grep -ho '^\(function\|class\|async function\) [A-Za-z_$]*' \
  .work/upstream-138/tree/*/src/*.js | sort -u > /tmp/up.txt
grep -ho '^\(function\|class\|async function\) [A-Za-z_$]*' \
  windows/src/src/*.js | sort -u > /tmp/lo.txt
comm -23 /tmp/up.txt /tmp/lo.txt

# 2. 单个符号确认是否真的没有（含等价实现）
grep -rn "符号名" windows/src macos/src-tauri macos/fullatlas-source
```

判据：**只比功能，不比函数名。** 名字没有但功能有的（如第 7 条），不算缺口。
