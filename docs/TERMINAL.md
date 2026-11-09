# THMR 终端客户端（开发预览）

THMR CLI/TUI 与 Flutter 桌面界面共享 `thmrd` 播放宿主。移动端仍由 App 管理音频。

## 安装与启动

解压后保留完整目录，将 `bin` 加入 PATH。Windows 终端包内含与 GUI 相同版本的 `libmpv-2.dll`。Linux 首期采用 Ubuntu 24.04/glibc 2.39 构建基线，需要系统 `libmpv2`，MPRIS 需要用户 D-Bus 会话。macOS 需保留 `THMRHost.app`；首次公开发行需要 Developer ID 签名与公证，未配置时预览包使用本地签名。

首次从 0.1.x 桌面版切换时，先退出旧 GUI，再启动完整新包，并重新确认音量、来源和码率。旧 GUI 的播放参数尚未自动迁移到宿主配置。

```text
thmr                   交互终端进入 TUI
thmr play              后台收听
thmr pause             暂停并停止拉流
thmr stop              停止收听
thmr volume 35         修改 THMR 音量
thmr source auto       自动选择来源，也可用 source1/source2
thmr quality 128k      选择状态接口公开的码率
thmr status --json     查询确认状态，不自动启动宿主
thmr gui               打开已安装的兼容 Flutter 界面
thmr doctor            检查安装与音频依赖
thmr daemon stop       关闭宿主
```

TUI：空格播放/暂停，`+/-` 调音量，`1/2/a` 选择来源，`b` 切码率，`r` 刷新，`s` 停止，`q` 离开界面并保留后台播放。直播不能点播、seek 或本地跳歌，恢复时连接当前广播位置。

完整 Linux 包保留原 `thmr` GUI 启动脚本，终端命令位于 `bin/thmr`；将完整包的 `bin` 加入 PATH。macOS 完整包中的命令位于 `THMR.app/Contents/Helpers`，避免与 GUI 的 `THMR` 文件名冲突。独立终端包可用 `THMR_GUI_PATH` 或设置文件中的 `guiPath` 指定已安装 GUI。

## 开发构建

安装仓库 `rust-toolchain.toml` 固定的 Rust 和 Flutter SDK。运行 `python tool/build_native.py` 构建本机桌面程序；macOS 同时构建 Intel 和 Apple Silicon，Windows 使用 MSVC 目标。

`flutter build windows --release`（或 linux/macos）之后，运行 `python tool/package_desktop.py --platform windows --bundle build/windows/x64/runner/Release`；对应平台路径由 CI 展示。macOS 的 helpers 必须在应用签名之前加入，终端应用另外签名、公证后归档。

Actions 的 `Build Clients` 手动构建提供 `platform` 选项：`all` 构建全部，或选择 `windows`、`linux`、`macos`、`android`。单平台构建仅上传预览附件，不创建或发布 Release；分析和通用测试仍会运行。

源码直接运行 Flutter 时，`THMR_CLI_PATH` 可指向已构建 `thmr`。需要验证旧的应用内播放器时，用 `--dart-define=THMR_LOCAL_PLAYER=true`；正式统一桌面包默认连接宿主。

`THMR_RUNTIME_DIR` 和 `THMR_CONFIG_DIR` 可指定隔离的用户目录；`THMR_MPV_LIBRARY` 可指定开发用音频库。会话凭据不会写入命令行输出。自动测速只读取有界的公开音频样本，不读取或发送凭据；受限来源会报告测速失败。

## 更新与支持范围

GUI 自更新负责完整桌面包；终端包当前手动替换完整目录，包管理器安装将由包管理器更新。升级前执行 `thmr daemon stop`，关闭该安装中的 TUI。偏好保存在用户配置目录，不放在程序目录。

当前不包含账号登录桥接、收藏操作或跨设备遥控。daemon 模式下 Linux 的旧 MP3 流频谱尚未迁移，界面保持静态，避免再拉第二路远端音频；Windows 保留现有系统输出频谱。平台原生媒体键和 macOS 打包后的实际行为以 CI/人工验收记录为准。
