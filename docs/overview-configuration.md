# 配置结构概览

> 本文是 `docs/overview.md` 的专题补充，记录配置模型与读取边界。用户向说明见 `docs/wiki/`，字段真值以 `src-tauri/src/config.rs` 为准。

## 文件与兼容性

- 主配置文件为 `~/.askhuman/config.json`；新位置缺失时回退旧 `~/.humaninloop/config.json`。
- 配置缺字段时取默认值，未知字段忽略，文件缺失或损坏时回退完整默认配置。
- Unix 下配置目录收紧为 `0700`，配置文件收紧为 `0600`。

## 字段地图

- `general`
  - `theme`、`language`、`alwaysOnTop`、`appearAnimation`、`windowEffect`
  - `speechLanguage`、`speechShortcut`
  - `historyLimit`（默认 200）与 `popupSound`
  - `menuBarIcon`（`off|active|always`，默认 `off`，仅 macOS/Linux 桌面）
  - `popupPrewarm`（默认 `true`，Unix）
  - `daemonLifecycle`（`activity|keepalive`，默认 `activity`，Unix）
- `channels.popup`：`enabled`、`width`、`height`、`rememberSize`
- `channels.telegram`：`enabled`、`botToken`、`chatId`、`apiBaseUrl`
- `channels.dingding`：`enabled`、`clientId`、`clientSecret`、`userId`、三种卡片模板 ID，以及文本附件内联/转 docx 开关
- `channels.feishu`：`enabled`、`appId`、`appSecret`、`openId`、`baseUrl`
- `channels.slack`：`enabled`、`botToken`、`appToken`、`userId`
- `channels.autoActivation`：IM 渠道按需发送，默认 `false`
- `channels.autoEndWatch`：切离活跃 IM 后自动结束该渠道 watch，默认 `true`，仅在 `autoActivation` 开启时生效
- `experimental.enabled`：显示实验区，默认 `false`
- `experimental.verticalQuestions`：多问题纵向显示，默认 `false`

## 读取入口

- `AppConfig::load()` 解析完整配置和系统钥匙串中的密钥；Daemon、IM 渠道、设置页密钥状态等确需凭据的路径使用它。
- `AppConfig::load_without_secrets()` 只读非密钥配置；语言、主题、历史上限、纯信息命令及窗口创建等路径使用它，避免无关操作触发钥匙串访问。
- `AppConfig::save()` 负责原子写入，并将受管密钥迁移或写入系统钥匙串；只持有非密钥配置的调用不会清空既有密钥。

密钥存储和 macOS 会话边界见 `docs/specs/secret-storage-keychain.md`。
