# THMR 终端客户端

CLI、TUI 与 Flutter 桌面界面共享 `thmrd` 播放宿主，支持免登录直播、来源、码率、音量和状态查询。

## 安装

解压后保留完整目录。独立终端包将 `bin` 加入 PATH；完整桌面包的命令位置如下：

| 平台 | CLI 位置 | 运行要求 |
| --- | --- | --- |
| Windows | `thmr.exe` | 保留同目录的 `thmrd.exe` 和 `libmpv-2.dll` |
| Linux | `bundle/thmr` | Ubuntu 24.04/glibc 2.39 基线，系统提供 `libmpv2`；MPRIS 需要用户 D-Bus 会话 |
| macOS | `THMR.app/Contents/Helpers/thmr` | 独立终端包需保留 `THMRHost.app`；公开分发需 Developer ID 与公证，预览包可采用本地签名 |

Linux 完整包可直接运行 `./thmr`，或将 `bundle` 的绝对路径加入 PATH。`thmr gui` 通过 `thmr-gui` 检查运行库并打开图形界面；应用菜单使用相同命令。

## 命令

```text
thmr                   交互终端进入 TUI，或使用 defaultUi 设置
thmr tui               打开终端界面
thmr gui               打开已安装的兼容 Flutter 界面
thmr play              后台收听，必要时启动宿主
thmr pause             暂停并停止拉流
thmr stop              停止收听，保留宿主
thmr toggle            切换播放/暂停
thmr volume 35         修改 THMR 音量，范围 0–100
thmr source auto       自动选择来源，也可用 source1/source2
thmr quality 128k      选择状态接口公开的启用码率
thmr status --json     查询状态，不自动启动宿主
thmr refresh           刷新曲目信息
thmr doctor            检查安装与音频依赖
thmr paths --json      查看运行与配置目录
thmr daemon start      启动宿主
thmr daemon stop       停止播放并关闭宿主
```

无参数启动需要交互终端；脚本中应显式指定子命令。完整参数见 `thmr --help`。

TUI：空格播放/暂停，`+/-` 调音量，`1/2/a` 选择来源，`b` 切码率，`r` 刷新，`s` 停止，`q` 离开界面并保留后台播放。直播不支持点播、seek 或本地跳歌，恢复播放时连接当前广播位置。

GUI 明确退出会先请求停止播放并关闭宿主。后台服务不可达或 2 秒内未确认时，客户端仍会关闭并释放本机连接；后台故障不会阻止退出。最小化到托盘/任务栏继续播放。

## 设置与升级

`thmr paths` 显示用户目录，偏好保存在配置目录的 `settings.toml`。例如 `defaultUi = "gui"` 可调整交互终端无参数启动的默认界面。GUI 主题与窗口设置仍由图形客户端保存。

独立终端包可用 `THMR_GUI_PATH` 或设置中的 `guiPath` 指定兼容 GUI。开发用环境变量与构建步骤见源码仓库的 `docs/DEVELOPMENT.md`。

升级前执行 `thmr daemon stop` 并关闭 TUI。完整桌面包使用 GUI 自更新；独立终端包当前手动替换完整目录。首次从 0.1.x 桌面版切换时先退出旧 GUI，重新确认音量、来源和码率；旧播放设置尚未自动迁移。

终端账号授权、收藏操作和跨设备遥控尚未实现。Linux 共享宿主模式的频谱保持静态；Windows 保留系统输出频谱。其他平台限制和验收要求见源码仓库的 `docs/DEVELOPMENT.md`。
