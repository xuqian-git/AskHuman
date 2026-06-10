# 环境变量

简体中文 | [English](./environment-variables.en.md)

| 变量 | 作用 | 兼容旧名 |
| --- | --- | --- |
| `ASKHUMAN_ENV_SOURCE_NAME` | 自定义「来源名」：弹窗标题与各渠道消息头由默认的 `the Loop` 改为指定名称（如 `Question from Agent`） | — |
| `ASKHUMAN_BINARY` | 程序集成（npm 包）时优先使用的二进制绝对路径，便于自定义 / 测试 | `HUMANINLOOP_BINARY` |
| `ASKHUMAN_FEISHU_DEBUG` | 设为非空且非 `0` 时，写飞书长连接诊断日志到 `~/.askhuman/feishu-debug.log` | `HUMANINLOOP_FEISHU_DEBUG` |

> 括号中的旧变量名仍被识别，方便平滑迁移。
