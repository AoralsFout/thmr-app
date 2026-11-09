# 后端接口

本文整理当前 Flutter 客户端使用的接口与解析约定。调用实现位于 `lib/core/api/`，数据模型位于 `lib/core/models/models.dart`；桌面播放宿主的 Radio 请求位于 `crates/thmr-daemon/src/sources.rs`。本机 GUI/CLI 控制接口另见[控制协议](../protocol/README.md)。

## 配置

| Flutter 构建参数 | 默认值 | 用途 |
| --- | --- | --- |
| `THMR_RADIO_BASE_URL` | `https://radio.yunmoan.cn` | 应用内播放器与 Radio 请求 |
| `THMR_BASE_URL` | `https://thmr.yunmoan.cn` | 账号、收藏、投票和曲库代理 |
| `THMR_STATION_ID` | `touhou` | 电台 ID |
| `THMR_STREAM_TOKEN` | 空 | 受限流与反馈请求的 token |
| `THMR_FEATURE_VOTE` | `false` | 显示投票功能 |

参数由 `lib/core/config.dart` 的 `--dart-define` 读取。共享宿主的播放来源、频道与码率由用户配置目录的 `settings.toml` 管理；Flutter 的 Radio 构建参数不会覆盖它。

## 通用约定

- 除导出内容外使用 JSON，错误为 `{"error":"消息"}`。401 表示账号会话无效，清除登录态后重新登录。
- ID 使用字符串；`*Ms` 为毫秒 epoch，`createdAt`/`updatedAt`/`addedAt`/`expiresAt` 为 ISO 字符串或空值。
- Radio 的 `coverUrl` 可能为相对路径，使用当前 Radio 来源补全；账号接口通常返回绝对地址。空封面使用本地图片。
- 状态/业务请求不使用 HTTP 缓存；封面可缓存。收藏离线快照由客户端状态层单独管理。
- 直播没有 seek、上一曲或下一曲。切换来源/码率需要重连，必须保留用户暂停意图。

## Radio

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/api/stations/:sid/status` | 电台、启用码率、听众数、当前曲与队列 |
| GET | `/api/stations/:sid/playlist` | 排播列表 |
| GET | `/radio/:sid/:profileId` | MP3 直播流 |
| GET | `/api/stations/:sid/tracks/:trackId/cover` | 曲目封面 |
| POST | `/api/stations/:sid/feedback` | 曲目反馈 |

码率取 `station.bitrates` 中的启用条目。应用内播放器配置了 `streamToken` 时，将其作为 `token` 查询参数传给流和反馈请求。当前共享宿主首期面向免登录公开流，没有账号授权或流凭据桥接。

状态中的主要字段：

| 字段 | 客户端用途 |
| --- | --- |
| `station`、`station.bitrates` | 电台信息、启用码率与音频参数 |
| `runtime`、`totals`、`profiles` | 服务状态与听众统计 |
| `current` | 当前曲，可为空；包含 `trackId`、`title`、`artist`、`coverUrl`、`startedAtMs`、`expectedEndMs` 等 |
| `previous`、`queue` | 上一曲信息和计划排播；不提供本地点播能力 |

轮询与预计换曲时刻刷新由各播放路径的状态模块管理。网络元数据错误与音频错误分别处理，查询失败不应立即中断正常播放。

### 反馈

请求体为 `{type, contact, message, track, pageUrl}`，其中 `track` 提交当前曲目的 ID、标题和作者。响应由 `FeedbackResult` 解析，`autoSkipped` 为真时界面提示已自动切换曲目。

支持的类型为 `non-touhou`、`non-music`、`copyright`、`copyright-takedown`、`copyright-attribution`、`copyright-permission`、`violation`、`metadata`、`playback`、`other`。联系方式必填，消息可选；具体限制由反馈界面与服务端校验。

## 账号与登录

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/api/meta` | 公开配置、投票阈值与收藏上限 |
| GET | `/api/auth/login?redirect=...` | Natayark 登录入口 |
| GET | `/api/auth/me` | 用候选会话验证并读取用户 |
| PATCH | `/api/auth/profile` | 设置昵称，请求体 `{nickname}` |
| POST | `/api/auth/logout` | 注销会话 |

Flutter 在应用内 WebView 完成授权，优先使用临时 loopback 回调；后端拒绝该回调时回退网页登录落地页。客户端读取同一 WebView 环境的 Cookie，将候选值作为 Bearer 调用 `/api/auth/me` 验证，不依赖固定 Cookie 名称。通过后保存会话，后续业务请求使用 `Authorization: Bearer <token>`。`needsNickname` 为真时先完成昵称设置。

Linux 未实现 WebView 登录，只提供浏览器入口；系统浏览器 Cookie 不会自动交给客户端。终端授权尚未实现。

## 收藏、分享与曲库

除公开分享外，收藏操作需要账号会话。

| 方法 | 路径 | 请求或结果 |
| --- | --- | --- |
| GET | `/api/me/collections` | 收藏夹列表；首项为默认收藏夹。客户端也兼容 `collections`/`data` 包装 |
| POST | `/api/me/collections` | `{name, description}` → 收藏夹 |
| GET | `/api/me/collections/:cid` | `sort`、`order` 查询参数 → `{collection, items}` |
| DELETE | `/api/me/collections/:cid` | 删除非默认收藏夹 |
| GET | `/api/me/favorites/status` | `stationId`、`trackId` → 默认收藏状态和各夹状态 |
| POST | `/api/me/collections/:cid/items` | `{stationId, trackId}` → 条目 |
| DELETE | `/api/me/collections/:cid/items/:itemId` | 移除条目 |
| GET | `/api/me/collections/:cid/export` | `format=json/m3u/csv` → 文本 |
| POST | `/api/me/collections/:cid/share` | `{enabled}` → 分享开关 |
| GET | `/api/collections/:slug` | 公开分享 → `{collection, items}`；公开字段可能不含 `share` |
| GET | `/api/tracks/search` | `stationId`、`q`、`page`、`pageSize`、`sort` → `tracks` |

默认收藏夹不能删除。重复添加或达到上限可能返回 409，条目不存在可能返回 404。分享链接为 `https://thmr.yunmoan.cn/share/{slug}`，客户端通过公开 API 读取；410 表示分享过期。

收藏按钮根据状态接口返回的 `collectionId`/`itemId` 添加或移除，不能用曲目 ID 代替收藏条目 ID。结构定义与容错逻辑集中在模型和 API 适配器，不在文档中维护第二套 Dart 模型。

## 投票

| 方法 | 路径 | 用途 |
| --- | --- | --- |
| GET | `/api/vote/status?stationId=...` | 公开查询，登录时包含个人状态 |
| POST | `/api/vote` | `{stationId}` 提交投票 |
| DELETE | `/api/vote?stationId=...` | 取消投票 |

轮次由当前曲目及开始时间确定，换曲后更新状态。阈值来自 `/api/meta`；投票是否触发切歌由服务端决定，不能映射成播放器“下一曲”。
