# Linux 客户端

主要兼容目标为 Ubuntu 24.04 x64/glibc 2.39，使用 GTK 原生标题栏。其他发行版需提供兼容运行库；更高版本 glibc 上构建的包不保证兼容该基线。

## 安装与启动

Ubuntu 24.04 运行库：

```sh
sudo apt install libgtk-3-0t64 libmpv2 libsecret-1-0 libjsoncpp25 \
  libayatana-appindicator3-1
tar -xzf thmr-app-linux-x64.tar.gz
cd bundle
./thmr doctor
./thmr gui
```

Arch Linux 对应运行库为 `gtk3 mpv libsecret jsoncpp libayatana-appindicator`。

保留整个 `bundle`。根目录 `thmr` 是统一 CLI，无参数时在交互终端进入 TUI；完整用法见 `thmr --help` 或源码仓库的 `docs/TERMINAL.md`。`thmr gui` 通过 `thmr-gui` 检查 GUI 动态库和 libmpv，缺失时输出错误，并在有 Zenity/KDialog 时显示图形提示。直接运行 `thmr_app` 会跳过检查；仅检查 GUI 依赖可用 `./thmr-gui --check-runtime`。

建议放在长期使用且当前用户可写的位置，如 `~/Applications/THMR/`。程序目录及父目录都必须可写，自更新才能暂存并替换完整目录。

## 应用菜单

在最终安装目录执行 `./install-desktop.sh`，添加当前用户的 XDG 应用菜单入口和图标，无需 sudo，不启用开机自启。入口调用安装目录的 `thmr gui`，支持中文、空格及常见特殊字符路径。

移动安装目录或从旧版 GUI 启动脚本升级后，需重新运行安装脚本。`./uninstall-desktop.sh` 只删除它管理的入口和图标，保留程序与用户设置。

## 桌面行为

| 功能 | 行为 |
| --- | --- |
| 托盘 | StatusNotifier/AppIndicator；菜单提供显示窗口与退出 |
| 关闭窗口 | 默认询问，可记住退出或最小化选择；明确退出会停止并关闭共享宿主 |
| 托盘不可用/消失 | 保留可恢复的窗口，回退任务栏最小化并继续播放 |
| 重复启动 GUI | 唤起同一桌面会话的已有窗口 |
| 系统媒体控制 | 宿主提供 MPRIS 曲目信息、状态、播放/暂停、停止与音量；不提供 seek、上一曲或下一曲 |
| 频谱 | 共享宿主模式保持静态，尚未迁移音频频谱；应用内诊断模式使用内置 minimp3 |
| 登录/迷你播放器 | 登录保留网页版入口；不提供迷你播放器 |
| 自动更新 | 验签、哈希校验、完整目录替换、重启确认与失败回滚 |

GNOME 托盘取决于实际可用的 AppIndicator 宿主/扩展。Wayland 下唤起、焦点和最小化由合成器决定，系统拒绝最小化时保留可见窗口。

## 验证

Ubuntu 24.04 构建与验证依赖：

```sh
sudo apt install clang cmake ninja-build pkg-config libgtk-3-dev \
  liblzma-dev g++ libsecret-1-dev libjsoncpp-dev \
  libayatana-appindicator3-dev libmpv-dev mpv \
  python3-gi dbus-x11 xvfb desktop-file-utils
```

通用构建和测试步骤见源码仓库的 `docs/DEVELOPMENT.md`。完成原生与 Flutter 构建并运行 `package_desktop.py` 后：

```sh
python3 -m unittest discover -s tool -p 'test_linux_distribution.py'
python3 tool/test_linux_cli.py build/linux/x64/release/bundle/thmr
python3 tool/verify_linux_bundle.py build/linux/x64/release/bundle --max-glibc 2.39
python3 tool/test_linux_app.py build/linux/x64/release/bundle
dbus-run-session -- python3 tool/test_unified_host.py --bin-dir target/release
```

CI 还运行旧应用内 MPRIS/MP3 解码器的独立测试。minimp3 来源、哈希与许可证在 `native/vendor/minimp3/`，许可证随包安装到 `data/licenses/minimp3/LICENSE`。