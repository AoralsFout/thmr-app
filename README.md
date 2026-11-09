# THMR 东方音乐电台

东方音乐电台 **THMR(TouHouMusicRadio)** 跨平台客户端 — Windows / macOS / Linux / Android / iOS。支持 Flutter 图形界面或无需图形界面的 CLI / TUI 终端客户端。

## 安装与使用

从项目 Release 或 Actions 构建附件获取安装包。桌面压缩包需保留完整目录；macOS 将 DMG 中的 `THMR.app` 拖入 Applications。iOS 暂无本仓库提供的发行包。

| 平台 | 图形界面+终端安装包 | 仅终端包 |
| --- | --- | --- |
| Windows x64 | `thmr-app-windows-x64.zip` | `thmr-terminal-windows-x64.zip` |
| macOS Intel / Apple Silicon | `thmr-app-macos-universal.dmg`，另有自更新用 ZIP | `thmr-terminal-macos-universal.tar.gz` |
| Linux x64 | `thmr-app-linux-x64.tar.gz` | `thmr-terminal-linux-x64.tar.gz` |
| Android | `thmr-app-android.apk` | — |

Linux 完整包解压后：

```sh
cd bundle
./thmr gui             # 图形界面
./thmr                 # 交互终端进入 TUI
./install-desktop.sh   # 当前用户的应用菜单入口
```

终端包将 `bin` 加入 PATH，即可使用 `thmr play`、`thmr pause`、`thmr status` 等命令。TUI 按 `q` 离开后继续后台播放；GUI 最小化后继续播放，明确退出则停止播放并关闭宿主。

首次从 0.1.x 升级时先退出旧客户端，重新确认音量、来源和码率；这些旧设置尚未自动迁移到共享宿主。当前开发预览的功能边界和验收要求见[开发说明](docs/DEVELOPMENT.md#支持范围)。

## 设计语言:「幻想乡放送局」

与 Web 端(`thmr/src/styles/tokens.css`)同一套令牌,源出博丽神社的红白配色:

| 令牌 | 浅色 | 深色 |
| --- | --- | --- |
| 页面底 `pageBg` | `#fbf2f4`(粉纸渐变) | `#160d10`(酒红黑渐变) |
| 主墨 `ink` | `#24161a` | `#f8edf0` |
| 弱化 `mutedInk` | `#6f5960` | `#d7c1c8` |
| 面板 `panelBg` | `rgba(255,251,252,.92)` 毛玻璃 | `rgba(31,19,23,.90)` |
| 描边 `panelBorder` | `rgba(84,34,46,.16)` | `rgba(255,228,236,.13)` |
| 强调 `accent` | `#c65a69` 绯红(可被 `station.public.accentColor` 运行时覆盖) | 同左 |

排版:**思源宋体(Noto Serif SC)通栏**,正文行高 1.62,与 Web 端一致。
背景:所选角色的插画固定在左下方，页面使用透明底色与半透明渐变遮罩；切换主题时插画平滑过渡。
品牌元素:logo 的鸟居「T」、阴阳玉「R」、电波弧线被复用为封面 fallback、空状态插画。

## 文档

| 文档 | 内容 |
| --- | --- |
| [终端客户端](docs/TERMINAL.md) | CLI/TUI 命令、安装目录、设置和升级 |
| [Linux 客户端](docs/LINUX.md) | 系统依赖、应用菜单和桌面兼容性 |
| [开发说明](docs/DEVELOPMENT.md) | 架构、源码运行、构建、测试和支持范围 |
| [后端接口](docs/API.md) | Radio、账号、收藏、反馈和登录约定 |
| [本机控制协议](protocol/README.md) | GUI/CLI/TUI 与宿主的 HTTP/WebSocket 协议 |
| [更新与发布](docs/AUTO_UPDATE.md) | 签名、CI、安装器、更新 API 与镜像部署 |
| [更新记录](CHANGELOG.md) | 各版本面向用户的变更 |

## 开发

安装 Flutter、仓库固定的 Rust 工具链和对应平台的编译依赖后：

```sh
flutter pub get
python tool/build_native.py --debug
```

将 `THMR_CLI_PATH` 设为已构建 `thmr` 的绝对路径，再运行 `flutter run -d windows`（或 `macos` / `linux`）。移动端直接使用 `flutter run -d <设备 ID>`。各平台路径和打包命令见[开发说明](docs/DEVELOPMENT.md)。

安装包由 CI 发布到 Release/Artifacts，不存入源码仓库。
