# PROGRESS

按具体任务 / 需求记录待办与当前进展。任务 / 需求完成后删除其 section（历史留在 git）。

## 【进行中】项目 review 落实（第四批）：真机验证收尾

分支 `chore/project-review-2026-07`。已完成并验收：R7 渠道故障可见化、R9 设置页搜索。
本批已实现、待真机验证：
1. 玻璃背景加深（vibrancy tint 0.3→0.4）；
2. daemon.log 轮转（>5MB 复制到 .1 并截断，启动 + 每小时检查）；
3. R6 首次运行引导（无 IM 渠道时弹窗页脚一次性提示条，`~/.askhuman/ui-state.json`
   存关闭标记；「去配置」直达设置渠道 tab）+ 渠道卡「配置指南」文档外链（按语言选 .md/.en.md）；
4. GUI 宿主换新同步 B1+B2（最后窗口关闭即换新；有窗口挡住时托盘出「重启菜单栏应用以完成更新」项）。
其余 R8/R10–R15 未排期。另：TCC 弹窗修复用户尚未真机验证（Agent 任务确认弹层已验收）。

## 待办：项目 review 的 P2 项（择机）

报告见 `docs/investigations/project-review-2026-07.md`。剩余择机项：daemon.log 轮转、
SettingsView/PopupView 组件拆分、types.ts 改为从 Rust 派生（ts-rs/specta）、
secrets.rs 注释修正、gitutil 测试噪音、TS 7 升级（等 vue-tsc 支持）、
前端主 bundle 瘦身（R4，需 perf 基线）、agents.snapshot() typed 化 + pnpm/Node 版本对齐（R5）。

## 待办：Cursor 全局 Rules 迁移为用户级 always-on Skill

调查与候选设计见 `docs/investigations/cursor-global-rule-user-skill.md`。无 workspace folder 的 Cursor IDE
不创建项目 Rules 加载器，因此不会读取 `~/.cursor/rules/askhuman.mdc`。未来改为用户级
`~/.cursor/skills/askhuman/SKILL.md`，旧安装显示“需更新”，迁移时先写新 Skill、再清理旧托管 MDC。
Grok 默认会扫描 Cursor Skills，候选 frontmatter 已设计为对 Cursor 常驻、对 Grok 不可调用。

## 待办：daemon 二进制变化检测 —— 轮询 vs filewatch（后续评估，优先级低）

二进制变化检测目前是 **15s 轮询** `current_exe()` 指纹（稳态≈1 次 `stat`，靠 `binhash.json` 内容哈希缓存避免重哈希）。
是否改 **filewatch** 待权衡——难点：二进制走原子替换（rename 换 inode，需盯父目录 + 按文件名过滤 + 每次替换后重挂，
参考 `config_watch.rs`）、装在任意目录（`~/.local/bin`/brew/npm 前缀/`.app` bundle…）、且 watcher 仍要 stat/hash 才能确认
内容**真**变（指纹是内容哈希而非 mtime）。延迟要求松（~15s 够）+ Hello 路径兜底，故暂保持轮询。
