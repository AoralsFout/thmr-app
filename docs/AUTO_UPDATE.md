# 更新与发布

本文维护客户端签名、安装和发行流程。源码构建与测试见[开发说明](DEVELOPMENT.md)，客户端版本说明集中在 [CHANGELOG](../CHANGELOG.md)。

## 更新行为

客户端从 `https://api.zyghit.cn/updater/thmr` 获取签名清单，下载后校验 Ed25519 签名、平台架构、大小和 SHA-256。下载不会自动退出应用，用户确认后安装。

| 平台 | 安装行为与条件 |
| --- | --- |
| Windows | 替换完整便携目录并重启；必要时请求 UAC，取消则保留原版 |
| macOS | 校验应用签名与 Bundle ID 后替换 `.app`；安装目录需可写 |
| Linux | 替换完整 `bundle`；目录与父目录需可写，系统提供运行库 |
| Android | 校验包名、版本码与签名后打开系统安装器，必须由用户确认 |
| iOS | 不使用此更新器，通过 App Store/TestFlight 渠道更新 |

桌面安装前关闭共享宿主并通知 TUI 退出。新版需返回健康首帧回执，超时或启动失败则回滚；用户设置位于应用数据目录，不随程序目录替换。此机制用于本仓库的 ZIP/tar.gz/APK，不承担系统包管理器或应用商店更新。

macOS DMG 用于首次拖拽安装，更新清单必须引用同一 Release 的 ZIP。独立终端包目前手动替换，升级前执行 `thmr daemon stop`。

## 密钥与 CI 配置

已有发行密钥时继续使用，不能为一次发版重新生成。首次配置可运行：

```sh
node tool/update_release.cjs keygen --output .update-keys/thmr-ed25519.pem
```

命令输出 Base64 公钥，私钥写入指定文件且不覆盖现有文件。将同一公钥配置到客户端和更新 API，私钥仅用于发布签名并保留离线备份。换钥需要迁移版本；仅更换服务端公钥会导致旧客户端验签失败。

GitHub Actions 配置如下：

| Repository Variable | 用途 |
| --- | --- |
| `THMR_UPDATE_PUBLIC_KEY` | Base64 更新公钥，正式标签构建必需 |
| `THMR_UPDATE_API_URL` | 可选，默认上述 API 地址 |
| `THMR_MACOS_RUNNER` | 可选，覆盖 macOS 构建运行器，默认值见工作流 |

| Repository Secret | 用途 |
| --- | --- |
| `THMR_UPDATE_PRIVATE_KEY` | Ed25519 PEM 私钥完整内容 |
| `ANDROID_KEYSTORE_BASE64` | 正式 APK 的固定 keystore |
| `ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`、`ANDROID_KEY_PASSWORD` | Android 签名材料 |
| `THMR_UPDATE_PUBLISH_TOKEN` | 仅可选 POST 发布工具需要 |

Android 首次发行后应保持原 keystore。签名不同的旧调试版无法直接覆盖安装。本地 Gradle 读取 `THMR_ANDROID_KEYSTORE_PATH`、`THMR_ANDROID_KEYSTORE_PASSWORD`、`THMR_ANDROID_KEY_ALIAS`、`THMR_ANDROID_KEY_PASSWORD`，也支持未提交的 `android/key.properties`；未配置时开发构建回退 debug 签名。

macOS 公开发行配置：

| Secret | 用途 |
| --- | --- |
| `MACOS_CERTIFICATE_BASE64`、`MACOS_CERTIFICATE_PASSWORD` | Developer ID Application 的 P12 与密码 |
| `MACOS_CODESIGN_IDENTITY` | Developer ID 签名身份 |
| `APPLE_API_KEY_P8`、`APPLE_API_KEY_ID`、`APPLE_API_ISSUER_ID` | Apple 公证凭据 |

`tool/macos_distribution.sh` 在原生程序加入后签名、可选公证并生成包。未配置 Developer ID 的预览使用 ad-hoc 签名，不代表已通过公证/Gatekeeper。CI 检查 universal 架构、签名和构建机原生架构的 GUI 启动；另一架构仍需实机验收。直接分发版关闭 App Sandbox，应用商店渠道需单独配置。

## 发布流程

1. 在 `CHANGELOG.md` 添加目标稳定版本段落，如 `## 0.2.0`。发布工具从该段生成 Release 正文、`RELEASE_NOTES.md` 和签名清单，缺失时停止。
2. 通过 `Build Clients` 的单平台构建或 PR 检查预览附件。上游 main 的完整构建生成 `build-版本-构建号` 预发布；fork、PR、普通开发分支与单平台构建只上传附件。
3. 完成验收后再推送 `v0.2.0` 一类稳定标签。各平台共用同一产品版本、构建号、公钥与 API 地址，依次上传到同一 Release 草稿。
4. 全部平台完成后上传校验和及稳定版本的签名清单，再发布 Release。预发布包不覆盖正式更新清单。
5. 将同一 Release 的原始安装包上传镜像，配置更新 API 的下载地址，并部署该 Release 的签名清单。构建工作流不会自动将更新发布到自有 API。

构建号由 `tool/resolve_version.py` 统一解析。重跑整个工作流会产生新构建号；只重试平台任务沿用该轮校验阶段的构建号。不要混用不同 Release 的包或解压后重新打包。

## 更新 API 与镜像

更新服务作为独立插件交付，不属于客户端仓库。这里仅保留客户端所需的部署契约；插件自身的安装/升级以其交付说明为准。

| 请求 | 行为 |
| --- | --- |
| `GET /updater/thmr` | 返回签名清单；无清单为 204，清单有效为 200，配置/验签故障为 503 |
| `GET /updater/thmr/status` | 插件健康与已验签版本，不返回凭据 |
| `POST /updater/thmr/publish` | 可选发布接口，需要 Bearer 发布凭据 |
| `GET /updater/ofcpl` | 兼容已有 CPL 路由 |

既有目录插件通常将私有配置与清单放在 `apis/updater/data/`，旧单文件插件使用 `data/updates/`。可通过 `THMR_UPDATE_CONFIG`/`THMR_UPDATE_MANIFEST` 指定持久化路径。同名的 `apis/updater.js` 与 `apis/updater/index.js` 只启用一个；覆盖升级插件前备份私有数据。含凭据的配置不能放入 `public`。

配置中保存 `publicKey` 和可选 `publishToken`，版本信息在签名清单中。将 Release 的 `thmr-update.json` 原样放入插件读取目录，可命名为 `thmr-release.json`；两者同时存在时既有插件优先后者。配置文件存在不代表已发布版本，204 也可能意味着清单路径错误。

镜像地址与清单分开配置，例如既有插件的 `THMR_DOWNLOAD_URLS`：

```js
const THMR_DOWNLOAD_URLS = {
  windows: 'https://你的镜像/thmr-app-windows-x64.zip',
  macos: 'https://你的镜像/thmr-app-macos-universal.zip',
  linux: 'https://你的镜像/thmr-app-linux-x64.tar.gz',
  android: 'https://你的镜像/thmr-app-android.apk',
};
```

镜像必须保留原包字节，地址使用完整 HTTPS URL。清单里的版本、大小、哈希与说明受签名保护，不手工修改 `payload` 或 `signature`。既有插件每次读取清单，更换清单无需重启；修改插件 JS 中的地址需重载。

可选 POST 发布使用至少 32 字符的 `THMR_UPDATE_PUBLISH_TOKEN`，服务端需保留 Authorization 头、允许对应路由及足够的 JSON 请求体。默认文件部署不依赖该接口。

## 清单与排错

API 信封为 `{schema:1, algorithm:"ed25519", payload:"Base64", signature:"Base64"}`。签名覆盖 payload 解码后的原始 UTF-8 字节，不对 JSON 重排后验签。payload 包含 `program`、`version`、`build_number`、`notes`、`pub_date`、`release_page` 和 `platforms`。

平台键为 `windows-x86_64`、`darwin-x86_64`、`darwin-aarch64`、`linux-x86_64`、`android-universal`；每项包含 HTTPS `url`、小写 SHA-256 `sha256`、字节数 `size` 和 `format`。API 的 `download_urls` 只覆盖下载位置，不覆盖签名保护的元数据。客户端先比较语义版本，再比较同一版本的构建号。

使用公钥检查已有清单：

```sh
node tool/update_release.cjs inspect --file thmr-release.json --public-key <Base64公钥>
```

| 现象 | 检查 |
| --- | --- |
| 客户端未配置公钥/验签失败 | 客户端、公钥配置与 CI 私钥是否匹配，清单是否被修改 |
| API 204/503 | 实际清单路径、读取权限、公钥与服务日志 |
| POST 401/413 | 发布凭据、代理 Authorization 和请求体限制 |
| POST 409 | 版本/构建号是否递增，是否有并发发布 |
| 下载校验失败 | 镜像是否上传了同一 Release 的原始完整安装包 |
| APK 拒绝安装 | 包名、签名、递增版本码与安装来源授权 |
| 桌面安装回滚 | 查看安装错误、宿主退出、目录写权限与健康启动回执 |
