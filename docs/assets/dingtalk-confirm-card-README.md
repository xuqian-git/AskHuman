# 钉钉通用确认卡模板（新建）

**独立模板**，不是提问卡，也不是原 Watch 卡。布局与飞书 `/stage` 双按钮一致：

```
标题 (title)
正文 markdown
[ 确认 btn_primary ]  [ 取消 btn_secondary ]   ← finalized=false
[ 终态 final_label 禁用 ]                        ← finalized=true
```

## 文件

`dingtalk-confirm-card-template.json` — 从 Watch 导出骨架**删掉** state/TODO/updated/rewatch 等节点后，只保留：

- BaseText → `title`
- Markdown → `markdown`
- ColumnLayout(双 SingleButton) → 仅当 `finalized` 为 false
- ButtonBlock 单按钮 → `final_label`（终态）

按钮 actionId：`confirm_ok` / `confirm_cancel`。

## 导入

1. 钉钉开放平台 → 卡片平台 → **导入**该 JSON  
2. 设计器中核对：活动态双按钮、终态只显示 `final_label`  
3. 发布，复制模板 ID（`xxxx.schema`）  
4. 配置：

```bash
AskHuman config set channels.dingding.confirmCardTemplateId 'xxxx.schema'
```

## 变量

| 名 | 类型 | 说明 |
|---|---|---|
| title | string | 标题 |
| markdown | markdown | 正文 |
| btn_primary | string | 主按钮文案 |
| btn_secondary | string | 次按钮文案 |
| finalized | boolean | 是否终态 |
| final_label | string | 终态按钮/标签文案 |

代码：`src-tauri/src/dingtalk/confirm.rs`。
