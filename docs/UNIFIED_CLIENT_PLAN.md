# 统一 THMR 桌面客户端实施计划

开始日期：2026-11-09。工作分支：`codex/unified-cli-tui-gui`，基于上游 `f96e76b`。

## 目标与约束

- 同一仓库维护 Flutter GUI、Rust CLI/TUI 和独立桌面播放宿主 `thmrd`。
- 桌面三个入口共享一个音频引擎、播放状态和系统媒体会话；移动端保留原生应用内播放。
- 首版交付免登录直播、来源/码率/音量设置、状态查询、TUI、GUI 接入和两类桌面安装包。
- 登录桥接、收藏、纯终端授权属于后续里程碑，不能阻塞免登录收听版本。
- 保留当前 Flutter 目录结构和旧客户端更新协议；统一产品版本，独立协商本机协议版本。
- 不提供点播、seek 或管理员跳歌；恢复播放连接直播边缘。

## 分阶段执行

每阶段完成适当验证后提交；集成阶段推送 fork，执行 PR/手动预览构建。稳定标签和正式发布等待整体验收。

### P1：工程与协议基础

- [x] Cargo workspace、固定工具链、依赖锁和统一版本注入。
- [x] 版本化命令、快照、会话实例 ID、状态 revision 和能力协商。
- [x] 用户级运行目录、独占锁、认证实例发现与配置持久化。
- [x] 协议/设置错误及兼容性测试。

### P2：播放宿主与 CLI

- [x] libmpv 后端；加载、停止、音量、事件与退出资源释放。
- [x] 单所有者播放状态机：暂停意图、重试上限、迟到事件处理。
- [x] Radio 状态轮询、换曲刷新、元数据、来源和启用码率。
- [x] 认证 loopback HTTP/WebSocket；断线重订阅完整快照。
- [x] CLI 命令、后台启动、状态 JSON、诊断和 GUI 启动。
- [x] Windows SMTC、Linux MPRIS、macOS MediaPlayer 集成及原生宿主生命周期。
- [x] 本地假流与真实服务的有界播放验证（空音频输出）。

### P3：TUI 与 Flutter 接入

- [x] TUI：状态/曲目/排播/来源/码率/音量，键盘操作与终端恢复。
- [x] Flutter 桌面 daemon adapter：命令、快照、重连与元数据。
- [x] 桌面禁用本地 Player、重复状态轮询和重复媒体注册；移动端保留原播放路径。
- [x] 来源、码率、音量统一归宿主；主题/窗口设置继续归 GUI。
- [x] 自动验证单实例、界面关闭保留播放、宿主重启、暂停期间切来源/码率。

### P4：构建与发行

- [x] 保留现有 GUI 产物名称，增加独立终端包。
- [x] 原生平台构建 Rust，完整桌面包捆绑 CLI/宿主；固定音频依赖来源。
- [x] PR 验证 Rust/Flutter/协议/打包；非发布分支不发布稳定更新。
- [x] 同一产品版本/构建号、校验和、依赖说明与签名流程。
- [x] 更新前停止宿主并通知 TUI 退出；兼容旧 GUI 清单，终端包首期手动升级。
- [x] 三平台 CI 构建与可执行冒烟，记录真实媒体键验收的范围。

## 发布形态

完整桌面包：Flutter GUI + `thmr`（CLI/TUI）+ `thmrd` + 平台音频依赖。

终端包：`thmr` + `thmrd` + 必要音频依赖，无 Flutter。首期使用压缩包和手动升级；包管理器版本由包管理器更新。

Windows x64、Linux x64、macOS arm64/x64 首期优先。macOS 完整包与终端包均构建 universal 二进制。Android 沿用 APK，iOS 不通过桌面自更新器更新。

## 验收标准

1. TUI 和 GUI 并发启动只有一个播放宿主和音频连接。
2. 系统媒体键、CLI、TUI、GUI 的播放暂停状态一致。
3. 关闭界面保留播放；明确停止/退出后关闭音频并释放资源。
4. 暂停期间断网恢复、切来源或换码率不自动播放。
5. 状态请求失败不打断正常音频；重试到上限后可主动恢复。
6. 不兼容协议明确报错，不静默生成第二个播放器。
7. Windows 实测；Linux/macOS 在原生 CI 构建并运行可自动化检查，物理媒体键另行记录。

## 执行记录

- 2026-11-09：确认上游 v0.1.6 基线和本机 Rust 1.96.0、Flutter 3.47.5；建立计划。
- P1–P4 的开发预览已实现，版本为 `0.2.0-dev.1`，本机协议为 `1`。实施过程按阶段提交并推送到 fork；没有创建稳定标签、发布正式更新或修改上游主分支。
- 三平台 Rust 格式、Clippy、单元测试与可执行程序检查通过：[Native host #42550302635](https://github.com/AoralsFout/thmr-app/actions/runs/42550302635)，运行代码提交 `116a22b`。Linux 私有 D-Bus 会话、macOS 无窗口原生媒体宿主均运行真实 libmpv 本地音频流；Unix 伪终端验证 `q` 保留播放、更新通知退出 TUI。
- 本地 Windows 完整包和终端包已构建，使用 MSVC 静态 CRT。隔离测试验证 GUI 健康回执、CLI 启动 GUI、重复启动、GUI 退出保留音频；真实 THMR MP3 免登录流在空音频输出下通过有界验证。
- Windows CI 预览为 `0.2.0-dev.1+1101`；完整包、终端包、版本信息和 SHA-256 文件下载至 `build/unified-preview/ci/windows/`（不提交生成产物）。仓库仍保留手工构建号 `+1`，CI 按工作流规则生成统一构建号。
- macOS 原生程序放在 `Contents/Helpers`，避免默认大小写不敏感的文件系统把 CLI `thmr` 与 GUI `THMR` 混为一个文件；打包回归测试检查 GUI 保留及 helper 路径。手动预览构建可选择单个平台，且禁止发布 Release。
- 平台构建与对应自动检查通过：[Windows x64](https://github.com/AoralsFout/thmr-app/actions/runs/42547163840/job/128935228023)、[Linux x64](https://github.com/AoralsFout/thmr-app/actions/runs/42547163840/job/128935227980)、[Android APK](https://github.com/AoralsFout/thmr-app/actions/runs/42547163840/job/128935228075)。
- [macOS universal 构建](https://github.com/AoralsFout/thmr-app/actions/runs/42550307337)中，两类包的真实 libmpv 播放测试、22 个 Mach-O 文件的架构/签名检查、ZIP 与 DMG 的 GUI 首帧健康回执均通过。Apple Silicon 切片已编入并检查，执行测试使用 Intel runner；公开发行签名与实机验收另列在下方。
- [macOS 预览附件](https://github.com/AoralsFout/thmr-app/actions/runs/42550307337/artifacts/13647322358)已上传，包含 universal GUI ZIP、DMG 和独立终端包。运行代码为 `116a22b`；最后的收尾提交只更新文档。
- Flutter 分析、完整测试，以及协议、设置、打包、更新器测试通过。本地完整 Python 测试为 37 项、3 项平台相关跳过；更新器夹具等待回滚进程启动和退出后再清理。GUI 自更新先请求 `prepare_update`，等待宿主退出后再替换文件；终端包保留手动升级方式。

## 正式发行前的人工验收与后续工作

- [ ] Windows/macOS/Linux 实际音频设备与系统媒体卡片、物理播放暂停键、耳机键验收。
- [ ] Apple Silicon 实机启动与播放；当前 macOS CI 在 Intel 运行，检查两种架构均已编入产物。
- [ ] 睡眠唤醒、真实断网恢复、长时间收听和多种 Linux 桌面环境验收。
- [ ] 在维护者签名环境验证 macOS Developer ID、公证与 Gatekeeper；fork 预览采用本地签名，不代表公开发行签名已通过。
- [ ] 统一桌面安装包整体人工验收后再决定正式版本和稳定标签。

后续里程碑保留账号登录/收藏桥接、包管理器发行和 Linux daemon 音频频谱。当前 Linux GUI 在共享宿主模式停用旧远端 MP3 频谱，避免重复拉流；Windows 保留系统输出频谱。这里的多端指三个桌面系统入口，不包含跨设备网络遥控。
