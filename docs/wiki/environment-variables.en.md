# Environment Variables

[简体中文](./environment-variables.md) | English

| Variable | Purpose | Legacy alias |
| --- | --- | --- |
| `ASKHUMAN_ENV_SOURCE_NAME` | Custom "source name": the popup title and channel message headers change from the default `the Loop` to your value (e.g. `Question from Agent`) | — |
| `ASKHUMAN_BINARY` | Absolute path to a binary that program integrations (the npm package) should prefer, handy for custom / test builds | `HUMANINLOOP_BINARY` |
| `ASKHUMAN_FEISHU_DEBUG` | When set to a non-empty value other than `0`, writes Feishu long-connection diagnostics to `~/.askhuman/feishu-debug.log` | `HUMANINLOOP_FEISHU_DEBUG` |

> The legacy variable names in parentheses are still recognized for a smooth migration.
