//! 跨渠道单选卡（select/picker）：发送、路由与四渠道回调分发。

use super::*;

// ===== 通用「单选卡」子系统（spec docs/specs/im-select-card.md）=====

/// 登记一条单选卡台账（顺带按 TTL + 每渠道软上限清理旧卡）。
pub(super) fn register_picker(state: &Arc<ServerState>, entry: PickerEntry) {
    let now = now_secs();
    let mut pickers = state.select.pickers.lock().unwrap();
    // TTL 兜底清理（全渠道）。
    pickers.retain(|p| now.saturating_sub(p.created_at) < SELECT_PICKER_TTL_SECS);
    let channel = entry.channel.clone();
    pickers.push(entry);
    // 每渠道软上限：超出丢最旧（本渠道最靠前的条目）。
    while pickers.iter().filter(|p| p.channel == channel).count() > SELECT_MAX_PICKERS_PER_CHANNEL {
        if let Some(pos) = pickers.iter().position(|p| p.channel == channel) {
            pickers.remove(pos);
        } else {
            break;
        }
    }
}

/// 发一张单选卡到某渠道，返回消息 id（MVP 仅飞书；其它渠道 None → 调用方回文本兜底）。
pub(super) async fn send_select_card(
    channel_id: &str,
    config: &AppConfig,
    view: &crate::select::SelectView,
) -> Option<String> {
    match channel_id {
        "feishu" => {
            let client = crate::feishu::client::FeishuClient::new(&config.channels.feishu).ok()?;
            let card = crate::feishu::card::build_select_card(view);
            client.send_card(&card).await.ok()
        }
        "dingding" => {
            // 钉钉：模板 + 变量。消息 id = 自铸 outTrackId（与 watch 卡同规，天然可编辑）。
            let client =
                crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding).ok()?;
            let otid = format!("select-{}", uuid::Uuid::new_v4());
            let map = crate::dingtalk::select::build_select_param_map(view, Lang::current());
            client
                .create_and_deliver_card(
                    &otid,
                    crate::dingtalk::select::DEFAULT_SELECT_CARD_TEMPLATE_ID,
                    map,
                    serde_json::json!({}),
                )
                .await
                .ok()?;
            Some(otid)
        }
        "telegram" => {
            let tg = &config.channels.telegram;
            let client = crate::telegram::TelegramClient::new(
                tg.bot_token.clone(),
                tg.chat_id.clone(),
                tg.api_base_url.clone(),
            )
            .ok()?;
            let html = crate::telegram::select::render_select_html(view);
            let markup = crate::telegram::select::inline_keyboard(view, Lang::current());
            client
                .send_message(&html, Some("HTML"), Some(markup))
                .await
                .ok()
                .map(|mid| mid.to_string())
        }
        "slack" => {
            let client = crate::slack::client::SlackClient::new(&config.channels.slack).ok()?;
            let dm = client.open_dm().await.ok()?;
            let (blocks, fallback) =
                crate::slack::select::build_select_blocks(view, Lang::current());
            client
                .post_message(&dm, Some(&blocks), &fallback)
                .await
                .ok()
        }
        _ => None,
    }
}

/// 组装并发一张 agent 单选卡：空选项 / 非支持渠道（send 失败）→ 返回 false（调用方回文本兜底）。
/// `payload` 仅 `PickerKind::Msg` 用（待发送内容随卡登记，点「发送」时投递）。
#[allow(clippy::too_many_arguments)] // args mirror the picker card fields
pub(super) async fn send_agent_picker(
    state: &Arc<ServerState>,
    channel_id: &str,
    config: &AppConfig,
    kind: PickerKind,
    title: String,
    options: Vec<crate::select::SelectOption>,
    payload: Option<String>,
    lang: Lang,
) -> bool {
    if options.is_empty() {
        return false;
    }
    let action = match kind {
        PickerKind::TaskWorkspace => crate::select::SelectAction::TaskWorkspace,
        PickerKind::TaskAgent => crate::select::SelectAction::TaskAgent,
        PickerKind::TaskPermission => crate::select::SelectAction::TaskPermission,
        PickerKind::Watch => crate::select::SelectAction::Watch,
        PickerKind::Status => crate::select::SelectAction::Status,
        PickerKind::Unwatch => crate::select::SelectAction::Unwatch,
        PickerKind::Msg => crate::select::SelectAction::Msg,
        PickerKind::Diff => crate::select::SelectAction::Diff,
        PickerKind::Stage => crate::select::SelectAction::Stage,
        PickerKind::Transcript => crate::select::SelectAction::Transcript,
        PickerKind::Todo => crate::select::SelectAction::Todo,
        PickerKind::TodoRm => crate::select::SelectAction::TodoRm,
        PickerKind::TodoRmEntry => crate::select::SelectAction::TodoRmEntry,
        PickerKind::TodoAuto => crate::select::SelectAction::TodoAuto,
        PickerKind::TodoAutoEntry => crate::select::SelectAction::TodoAutoEntry,
        // 管理卡不经单选卡通道发送（见 todo.rs::send_todo_manage / register_todo_manage）。
        PickerKind::TodoManage => return false,
    };
    let view = crate::select::build_view(title, options, action, lang);
    let session_ids: Vec<String> = view.options.iter().map(|o| o.id.clone()).collect();
    let Some(mid) = send_select_card(channel_id, config, &view).await else {
        return false;
    };
    register_picker(
        state,
        PickerEntry {
            channel: channel_id.to_string(),
            message_id: mid,
            kind,
            title: view.title.clone(),
            options: session_ids,
            payload,
            created_at: now_secs(),
            posted_ms: now_ms(),
        },
    );
    state.select.route_refresh.notify_one();
    true
}

#[allow(clippy::too_many_arguments)] // args mirror the select-card callback context
pub(super) async fn select_pick_task_flow(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    selected_id: &str,
    picker: &PickerEntry,
    config: &AppConfig,
    lang: Lang,
    ack: Option<crate::confirm::transport::FsAck>,
) {
    let title = match picker.kind {
        PickerKind::TaskWorkspace => crate::select::title_task_workspace(lang),
        PickerKind::TaskAgent => crate::select::title_task_agent(lang),
        PickerKind::TaskPermission => crate::select::title_task_permission(lang),
        _ => String::new(),
    };
    let label = match picker.kind {
        PickerKind::TaskWorkspace => {
            crate::autochannel::project_name(selected_id).unwrap_or_else(|| selected_id.to_string())
        }
        PickerKind::TaskAgent => AgentKind::parse(selected_id)
            .map(|kind| kind.label().to_string())
            .unwrap_or_else(|| selected_id.to_string()),
        PickerKind::TaskPermission if selected_id == "agent-default" => match lang {
            Lang::Zh => "Agent 默认",
            Lang::En => "Agent default",
        }
        .into(),
        PickerKind::TaskPermission => "YOLO".into(),
        _ => selected_id.to_string(),
    };
    if channel_id == "feishu" {
        if let Some(ack) = ack {
            let card = crate::feishu::card::build_select_final_card(&title, &label);
            let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
        }
    } else if channel_id == "dingding" {
        dd_finalize_select_card(config, mid, &label).await;
    } else {
        finalize_select_card_edit(channel_id, config, mid, &title, &label).await;
    }
    remove_picker(state, channel_id, mid);
    state.select.route_refresh.notify_one();
    continue_task_picker(state, channel_id, picker, selected_id, config, lang).await;
}

/// `/status` 详情（按 session_id 定位，避免 seq 漂移）。找不到 → notFound 提示。
pub(super) fn status_detail_by_session(
    snapshot: &serde_json::Value,
    session_id: &str,
    channel_id: &str,
    lang: Lang,
) -> String {
    let prefix = crate::autochannel::cmd_prefix(channel_id);
    let seq = snapshot.as_array().and_then(|l| {
        l.iter()
            .find(|r| r.get("sessionId").and_then(|v| v.as_str()) == Some(session_id))
            .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
    });
    match seq {
        Some(id) => crate::autochannel::status_detail_text(snapshot, id, prefix, lang),
        None => crate::i18n::tr(lang, "autoChannel.statusDetailNotFound")
            .replace("{id}", "?")
            .replace("{p}", prefix),
    }
}

/// 从单选卡台账移除某条卡。移除即视为「单选完结」：清零本渠道全部 watch 订阅的跟底节流
/// （`last_move_ms=0`）并唤醒引擎——单选期间被抑制的跟底在下一次内容变化时立即重发到会话底部
/// （用户定案，与「提问完结」一致；此处覆盖到钉钉，补上提问路径遗漏 dingding 的口径差）。
pub(super) fn remove_picker(state: &Arc<ServerState>, channel_id: &str, message_id: &str) {
    let removed = {
        let mut pickers = state.select.pickers.lock().unwrap();
        let before = pickers.len();
        pickers.retain(|p| !(p.channel == channel_id && p.message_id == message_id));
        pickers.len() != before
    };
    if !removed {
        return;
    }
    // 本渠道若已无其它在途单选卡，放开跟底：清零节流 + 唤醒引擎。
    if !has_active_select_on(state, channel_id) {
        let mut cleared = false;
        for s in state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter_mut()
            .filter(|s| s.channel == channel_id)
        {
            s.last_move_ms = 0;
            cleared = true;
        }
        if cleared {
            state.watch.notify.notify_one();
        }
    }
}

/// graceful 关停前把所有活动单选/确认卡就地定格为「已失效」终态（第 15 轮定案）：
/// 台账不持久化（spec im-select-card D7），重启后旧卡点击本会静默无响应——退出前主动
/// 去掉按钮/表单并留「请重新发送命令」提示。best-effort（渠道不可用/更新失败仅记日志），
/// 调用方需自行限时以免拖住关停。
pub(super) async fn finalize_all_select_cards(state: &Arc<ServerState>) {
    let pickers: Vec<PickerEntry> = std::mem::take(&mut *state.select.pickers.lock().unwrap());
    let confirms: Vec<ConfirmEntry> = std::mem::take(&mut *state.select.confirms.lock().unwrap());
    if pickers.is_empty() && confirms.is_empty() {
        return;
    }
    let lang = Lang::current();
    let config = state.config_snapshot();
    let label = crate::i18n::tr(lang, "select.expiredCard");
    for p in &pickers {
        match p.channel.as_str() {
            "feishu" => {
                let card = crate::feishu::card::build_select_final_card(&p.title, label);
                if let Ok(client) =
                    crate::feishu::client::FeishuClient::new(&config.channels.feishu)
                {
                    if let Err(err) = client.patch_card(&p.message_id, &card).await {
                        log(&format!("select: expire feishu card failed: {}", err));
                    }
                }
            }
            "dingding" => {
                if p.kind == PickerKind::TodoManage {
                    // 管理卡走提问卡模板：置私有 `submitted=true` 关表单 + 公有终态文案。
                    if let Ok(client) =
                        crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding)
                    {
                        if let Err(err) = client
                            .update_card_private(
                                &p.message_id,
                                serde_json::json!({ "submit_status": label }),
                                serde_json::json!({ "submitted": "true" }),
                            )
                            .await
                        {
                            log(&format!(
                                "select: expire dingtalk manage card failed: {}",
                                err
                            ));
                        }
                    }
                } else {
                    dd_finalize_select_card(&config, &p.message_id, label).await;
                }
            }
            _ => {
                finalize_select_card_edit(&p.channel, &config, &p.message_id, &p.title, label)
                    .await
            }
        }
    }
    for c in &confirms {
        let fv = crate::confirm::ConfirmFinalView {
            title: c.view.title.clone(),
            body: label.to_string(),
            label: crate::confirm::transport::truncate_for_label(label),
        };
        crate::confirm::transport::finalize(&c.channel, &config, &c.message_id, &fv, None).await;
    }
}

/// 幂等确保各渠道的单选卡回调路由任务在位（撤掉已无 picker 的渠道路由）。飞书 / 钉钉 / TG / Slack。
/// Confirm 卡 message_id 一并纳入（与 pickers 共享路由任务）。
pub(super) async fn ensure_select_routes(state: &Arc<ServerState>) {
    let mut desired: HashMap<String, Vec<String>> = HashMap::new();
    for p in state.select.pickers.lock().unwrap().iter() {
        desired
            .entry(p.channel.clone())
            .or_default()
            .push(p.message_id.clone());
    }
    for c in state.select.confirms.lock().unwrap().iter() {
        desired
            .entry(c.channel.clone())
            .or_default()
            .push(c.message_id.clone());
    }
    for mids in desired.values_mut() {
        mids.sort();
        mids.dedup();
    }
    {
        let mut routes = state.select.routes.lock().unwrap();
        routes.retain(|ch, h| {
            if desired.contains_key(ch) {
                true
            } else {
                h.stop.notify_waiters();
                false
            }
        });
    }
    let config = state.config_snapshot();
    for (ch, mids) in desired {
        ensure_select_route_for(state, &config, &ch, mids).await;
    }
}

/// 幂等确保单一渠道的单选卡回调路由任务在位（飞书 / 钉钉 / TG / Slack；复用 watch 的路由句柄类型）。
/// 飞书走「回调同步回卡」(oneshot Option)；钉钉先空 ACK、卡片变化经 OpenAPI；TG/Slack 就地编辑
/// （见 `handle_select_dd_action` / `handle_select_tg_action` / `handle_select_slack_action`）。
pub(super) async fn ensure_select_route_for(
    state: &Arc<ServerState>,
    config: &AppConfig,
    channel_id: &str,
    mids: Vec<String>,
) {
    // 取该渠道的共享 Router（渠道不可用则跳过；picker 仍在，渠道恢复后下一拍补挂）。
    let router: WatchChannelRouter = match channel_id {
        "feishu" => {
            if !crate::app::is_feishu_active(config) {
                return;
            }
            match ensure_fs_router(state, &config.channels.feishu).await {
                Some(r) => WatchChannelRouter::Feishu(r),
                None => return,
            }
        }
        "dingding" => {
            if !crate::app::is_dingding_active(config) {
                return;
            }
            let dd = &config.channels.dingding;
            match ensure_dd_router(state, dd.client_id.trim(), dd.client_secret.trim()).await {
                Some(r) => WatchChannelRouter::DingTalk(r),
                None => return,
            }
        }
        "telegram" => {
            if !crate::app::is_telegram_active(config) {
                return;
            }
            match ensure_tg_router(state, &config.channels.telegram).await {
                Some(r) => WatchChannelRouter::Telegram(r),
                None => return,
            }
        }
        "slack" => {
            if !crate::app::is_slack_active(config) {
                return;
            }
            match ensure_sl_router(state, &config.channels.slack).await {
                Some(r) => WatchChannelRouter::Slack(r),
                None => return,
            }
        }
        _ => return,
    };
    // 现任务仍绑定同一存活 Router 且卡集合未变 → 无事可做。
    {
        let routes = state.select.routes.lock().unwrap();
        if let Some(h) = routes.get(channel_id) {
            if h.router.is_same_alive(&router) && h.mids == mids {
                return;
            }
        }
    }
    let stop = Arc::new(tokio::sync::Notify::new());
    let stop2 = stop.clone();
    let st = state.clone();
    let router_ref: WatchRouterRef = match &router {
        WatchChannelRouter::Feishu(r) => {
            let mut routed = r.register();
            for mid in &mids {
                routed.set_active(Some(mid), "");
            }
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = stop2.notified() => break,
                        ev = routed.recv() => match ev {
                            Some(crate::feishu::router::FsInbound::Card { data, ack }) => {
                                handle_select_card_action(&st, "feishu", &data, ack).await;
                            }
                            Some(_) => {} // 未认领聊天消息，不会到达；防御性忽略。
                            None => break, // Router 断开：下一拍 ensure 重建。
                        },
                    }
                }
            });
            WatchRouterRef::Feishu(Arc::downgrade(r))
        }
        WatchChannelRouter::DingTalk(r) => {
            let mut routed = r.register();
            for mid in &mids {
                routed.set_active(Some(mid), "");
            }
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = stop2.notified() => break,
                        ev = routed.recv() => match ev {
                            Some(crate::dingtalk::router::DdInbound::Card { data, ack }) => {
                                // 先 ACK 满足 3 秒回包（钉钉无「回调同步回卡」，卡片变化走 OpenAPI）。
                                // 提问模板的「提交」（待办管理卡）须回成功裁决（空包会显示「请求失败」）；
                                // 其余（单选/确认按钮）空包即可。
                                let _ = ack.send(if crate::dingtalk::card::is_submit(&data) {
                                    crate::dingtalk::card::submit_ack_success()
                                } else {
                                    serde_json::json!({})
                                });
                                handle_select_dd_action(&st, &data).await;
                            }
                            Some(_) => {} // 未认领聊天消息，不会到达；防御性忽略。
                            None => break, // Router 断开：下一拍 ensure 重建。
                        },
                    }
                }
            });
            WatchRouterRef::DingTalk(Arc::downgrade(r))
        }
        WatchChannelRouter::Telegram(r) => {
            let routed = r.register();
            for mid in &mids {
                if let Ok(m) = mid.parse::<i64>() {
                    // 仅认领卡片回调（`set_card_route`），不认领自由文字（不抢提问卡答案）。
                    routed.set_card_route(m);
                }
            }
            let mut routed = routed;
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = stop2.notified() => break,
                        ev = routed.recv() => match ev {
                            Some(crate::telegram::router::TgInbound::Callback(cb)) => {
                                handle_select_tg_action(&st, &cb).await;
                            }
                            Some(_) => {} // 未认领自由文字，不会到达；防御性忽略。
                            None => break,
                        },
                    }
                }
            });
            WatchRouterRef::Telegram(Arc::downgrade(r))
        }
        WatchChannelRouter::Slack(r) => {
            let mut routed = r.register();
            for mid in &mids {
                // user_id 传空 → 只认领卡片交互（message_ts），不认领聊天消息。
                routed.set_active(Some(mid), "");
            }
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = stop2.notified() => break,
                        ev = routed.recv() => match ev {
                            Some(crate::slack::router::SlInbound::Interactive(payload)) => {
                                handle_select_slack_action(&st, &payload).await;
                            }
                            Some(_) => {}
                            None => break,
                        },
                    }
                }
            });
            WatchRouterRef::Slack(Arc::downgrade(r))
        }
    };
    if let Some(old) = state.select.routes.lock().unwrap().insert(
        channel_id.to_string(),
        WatchRouteHandle {
            stop,
            router: router_ref,
            mids,
        },
    ) {
        old.stop.notify_waiters();
    }
}

/// 处理飞书单选卡 / 确认卡点击。
/// 过期 / 越界 / 无台账 → 空 ACK（静默，D7）。
pub(super) async fn handle_select_card_action(
    state: &Arc<ServerState>,
    channel_id: &str,
    data: &serde_json::Value,
    ack: crate::feishu::router::CardAck,
) {
    // Stage 双按钮确认卡。
    if let Some((mid, slot)) = crate::feishu::card::parse_confirm_action(data) {
        handle_confirm_action(state, channel_id, &mid, slot, Some(ack)).await;
        return;
    }
    let Some((mid, idx)) = crate::feishu::card::parse_select_action(data) else {
        // 非单选点击：可能是待办管理卡的表单提交（本路由上唯一带表单的卡）；否则空 ACK。
        fs_todo_manage_submit(state, data, ack).await;
        return;
    };
    let picker = {
        let pickers = state.select.pickers.lock().unwrap();
        pickers
            .iter()
            .find(|p| p.channel == channel_id && p.message_id == mid)
            .cloned()
    };
    let Some(picker) = picker else {
        let _ = ack.send(None); // 已过期 / 被清理：静默（D7）。
        return;
    };
    let Some(session_id) = picker.options.get(idx).cloned() else {
        let _ = ack.send(None);
        return;
    };
    let lang = Lang::current();
    let config = state.config_snapshot();
    match picker.kind {
        PickerKind::TaskWorkspace | PickerKind::TaskAgent | PickerKind::TaskPermission => {
            select_pick_task_flow(
                state,
                channel_id,
                &mid,
                &session_id,
                &picker,
                &config,
                lang,
                Some(ack),
            )
            .await;
        }
        PickerKind::Watch => {
            // 先完成就地变身（含卡片 ACK），再激活——避免激活的补推/回执拖慢同步 ACK。
            select_pick_watch(state, channel_id, &mid, &session_id, &config, lang, ack).await;
            // 卡片点『关注』＝在该渠道操作 → 设为活跃槽（与 /watch 一致，用户决策）。
            activate_channel_on_action(state, channel_id, &config, lang).await;
        }
        PickerKind::Status => {
            // 单选卡不动：先空 ACK，再回纯文本详情（可继续点其它 agent）。
            let _ = ack.send(None);
            let snapshot = state.agents.snapshot();
            let text = status_detail_by_session(&snapshot, &session_id, channel_id, lang);
            let _ = reply_channel_text(channel_id, &config, &text).await;
            // 卡片点『查看』＝在该渠道操作 → 设为活跃槽（补齐与 /status 文本命令的一致性）。
            activate_channel_on_action(state, channel_id, &config, lang).await;
        }
        PickerKind::Unwatch => {
            select_pick_unwatch(state, channel_id, &mid, &session_id, &config, lang, ack).await;
        }
        PickerKind::Msg => {
            let content = picker.payload.clone().unwrap_or_default();
            select_pick_msg(state, channel_id, &mid, &session_id, &content, lang, ack).await;
            // 卡片点『发送』＝在该渠道操作 → 设为活跃槽（与 /msg 一致，用户决策）。
            activate_channel_on_action(state, channel_id, &config, lang).await;
        }
        PickerKind::Diff | PickerKind::Stage | PickerKind::Transcript => {
            select_pick_export(
                state,
                channel_id,
                &mid,
                &session_id,
                picker.kind,
                &config,
                lang,
                Some(ack),
            )
            .await;
            activate_channel_on_action(state, channel_id, &config, lang).await;
        }
        // /todo · /todo-rm（spec todo-whats-next D8）。
        PickerKind::Todo => {
            fs_select_pick_todo(state, &mid, &session_id, lang, ack).await;
            activate_channel_on_action(state, channel_id, &config, lang).await;
        }
        PickerKind::TodoRm => {
            fs_select_pick_todo_rm(state, &mid, &session_id, lang, ack).await;
            activate_channel_on_action(state, channel_id, &config, lang).await;
        }
        PickerKind::TodoRmEntry => {
            fs_select_pick_todo_rm_entry(state, &mid, &session_id, &picker, lang, ack).await;
        }
        PickerKind::TodoAuto => {
            fs_select_pick_todo_auto(state, &mid, &session_id, lang, ack).await;
            activate_channel_on_action(state, channel_id, &config, lang).await;
        }
        PickerKind::TodoAutoEntry => {
            fs_select_pick_todo_auto_entry(state, &mid, &session_id, &picker, lang, ack).await;
        }
        // 管理卡无行按钮（options 恒空，上方取选项即已短路）；防御性空 ACK。
        PickerKind::TodoManage => {
            let _ = ack.send(None);
        }
    }
}

/// 单选卡点选「发送」（飞书就地定格）：校验目标工作中·非 grok → 投递 + 定格「已发送给 [编号]」；
/// 目标已漂移（不在工作中 / 已结束 / 消失）→ 定格「已不在工作中，未发送」。定格文案随卡回（ack）。
#[allow(clippy::too_many_arguments)]
pub(super) async fn select_pick_msg(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    session_id: &str,
    content: &str,
    lang: Lang,
    ack: crate::feishu::router::CardAck,
) {
    let snapshot = state.agents.snapshot();
    let rec = find_agent_by_session(&snapshot, session_id);
    let label = msg_pick_deliver(state, channel_id, session_id, rec, content, lang);
    let card =
        crate::feishu::card::build_select_final_card(&crate::select::title_msg(lang), &label);
    let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
    remove_picker(state, channel_id, mid);
}

/// 单选卡「发送」的共享收尾：目标仍工作中·非 grok → 投递并返回「已发送给 [编号] · 回执」定格文案；
/// 否则返回「已不在工作中，未发送」。渲染层各渠道自行把该文案落进定格卡。
pub(super) fn msg_pick_deliver(
    state: &Arc<ServerState>,
    channel_id: &str,
    session_id: &str,
    rec: Option<&serde_json::Value>,
    content: &str,
    lang: Lang,
) -> String {
    let ok = rec
        .map(|r| {
            r.get("state").and_then(|v| v.as_str()) == Some("working")
                && r.get("kind").and_then(|v| v.as_str()) != Some("grok")
        })
        .unwrap_or(false);
    if !ok {
        return crate::i18n::tr(lang, "select.msgTargetGone").to_string();
    }
    let seq = rec
        .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let note = deliver_msg(state, channel_id, session_id, content, lang);
    crate::i18n::tr(lang, "select.msgSentCard")
        .replace("{id}", &seq.to_string())
        .replace("{note}", &note)
}

/// 单选卡点选「watch」：就地把这张卡编辑成实时 watch 卡（经 oneshot 同步回卡）。
#[allow(clippy::too_many_arguments)]
pub(super) async fn select_pick_watch(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    session_id: &str,
    config: &AppConfig,
    lang: Lang,
    ack: crate::feishu::router::CardAck,
) {
    let now = now_secs();
    let snapshot = state.agents.snapshot();
    let rec = find_agent_by_session(&snapshot, session_id);
    let waiting = state
        .registry
        .in_flight_agent_session_ids()
        .contains(&session_id.to_string());
    let seq = rec
        .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let frame = crate::watch::build_frame(seq, rec, waiting);
    let ended = frame.phase == crate::watch::WatchPhase::Ended;
    if ended {
        // 已结束/消失：就地定格终态卡、不订阅、消费掉 picker。
        let card = crate::feishu::card::build_watch_card(&crate::watch::card_view(
            &frame,
            crate::watch::CardMode::Final(crate::watch::FinalKind::Ended),
            now,
            lang,
            None,
        ));
        let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
        remove_picker(state, channel_id, mid);
        // 不在此重挂 select 路由（避免 recv-loop 递归 → 非 Send）：残留的 mid 认领无害（卡已定格无按钮），
        // 下次 send_agent_picker / 监听重建时统一收敛。
        return;
    }
    // 上限校验（本渠道；已在关注同一 session＝换新卡，不计新增）。
    let already = state
        .watch
        .subs
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.channel == channel_id && s.session_id == session_id);
    if !already {
        let count = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.channel == channel_id && !s.rewatchable)
            .count();
        if count >= crate::watch::MAX_WATCHES {
            let _ = ack.send(None);
            let text = crate::i18n::tr(lang, "watch.limit")
                .replace("{n}", &crate::watch::MAX_WATCHES.to_string())
                .replace("{p}", crate::autochannel::cmd_prefix(channel_id));
            let _ = reply_channel_text(channel_id, config, &text).await;
            return;
        }
    }
    // 就地回一张实时 watch 卡（这条单选卡消息随即变成 watch 卡）。
    let card = crate::feishu::card::build_watch_card(&crate::watch::card_view(
        &frame,
        crate::watch::CardMode::Active,
        now,
        lang,
        None,
    ));
    let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
    // 登记订阅（含换新卡收尾）+ 消费 picker + 让 watch 立即认领本消息（`ensure_watch_routes` 不会递归
    // 回 select）。select 侧不在此重挂（避免 recv-loop 递归 → 非 Send）：本 mid 已被 watch 认领覆盖，
    // 残留的 select 认领无害，下次 send_agent_picker / 监听重建时收敛。
    register_watch_at(
        state, channel_id, session_id, seq, mid, &frame, false, config, lang,
    )
    .await;
    remove_picker(state, channel_id, mid);
    ensure_watch_routes(state).await;
}

/// 单选卡点选「unwatch」：取消该关注（旧卡定格）+ 回文本确认 + 就地刷新单选卡（移除该项）。
#[allow(clippy::too_many_arguments)]
pub(super) async fn select_pick_unwatch(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    session_id: &str,
    config: &AppConfig,
    lang: Lang,
    ack: crate::feishu::router::CardAck,
) {
    let now = now_secs();
    // 找到该 session 在本渠道的订阅（可能已被别处取消/结束 → 视为已不在关注，只刷新卡）。
    let entry = state
        .watch
        .subs
        .lock()
        .unwrap()
        .iter()
        .find(|s| s.channel == channel_id && s.session_id == session_id)
        .cloned();
    if let Some(entry) = entry {
        // 旧 watch 卡定格「已取消关注」（可重新关注）。
        if let Some(client) = watch_client(state, channel_id, config).await {
            let snapshot = state.agents.snapshot();
            let waiting = state
                .registry
                .in_flight_agent_session_ids()
                .contains(&entry.session_id);
            let frame = crate::watch::build_frame(
                entry.seq,
                find_agent_by_session(&snapshot, &entry.session_id),
                waiting,
            );
            if let Err(err) = client
                .edit(
                    &entry.message_id,
                    &frame,
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Cancelled),
                    now,
                    lang,
                    Some(&entry.session_id),
                )
                .await
            {
                log(&format!("select: finalize unwatch card failed: {}", err));
            }
        }
        {
            let mut subs = state.watch.subs.lock().unwrap();
            if let Some(s) = subs.iter_mut().find(|s| s.message_id == entry.message_id) {
                s.rewatchable = true;
            }
        }
        persist_watch_subs(state);
        state.watch.notify.notify_one();
        ensure_watch_routes(state).await;
        let text =
            crate::i18n::tr(lang, "watch.unwatchDone").replace("{id}", &entry.seq.to_string());
        let _ = reply_channel_text(channel_id, config, &text).await;
    }
    // 就地刷新单选卡：剩余订阅 → 新卡；空 → 定格「已全部取消关注」并消费 picker。
    let snapshot = state.agents.snapshot();
    let options = unwatch_options(state, channel_id, &snapshot, lang);
    if options.is_empty() {
        let card = crate::feishu::card::build_select_final_card(
            &crate::select::title_unwatch(lang),
            crate::i18n::tr(lang, "select.unwatchAllDoneCard"),
        );
        let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
        remove_picker(state, channel_id, mid);
        // 不在此重挂 select 路由（同 select_pick_watch 理由）：卡已定格无按钮，残留认领无害。
    } else {
        let view = crate::select::build_view(
            crate::select::title_unwatch(lang),
            options,
            crate::select::SelectAction::Unwatch,
            lang,
        );
        let new_ids: Vec<String> = view.options.iter().map(|o| o.id.clone()).collect();
        let card = crate::feishu::card::build_select_card(&view);
        let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
        // 更新 picker 的选项快照（下标对齐新卡）。
        if let Some(p) = state
            .select
            .pickers
            .lock()
            .unwrap()
            .iter_mut()
            .find(|p| p.channel == channel_id && p.message_id == mid)
        {
            p.options = new_ids;
        }
    }
}

// ===== 钉钉单选卡点选（无「回调同步回卡」：空 ACK 已在路由任务发出，卡片变化走 OpenAPI）=====

/// 处理钉钉单选卡点击：解析 `(outTrackId, sid)` → 找 picker → 按 kind 分派。
/// 过期 / sid 空 / 不属于本卡 → 静默（D7；空 ACK 已由路由任务发出）。
pub(super) async fn handle_select_dd_action(state: &Arc<ServerState>, data: &serde_json::Value) {
    // Stage 确认卡（提问模板）：提交后按选项 0=暂存 / 1=取消。
    if handle_stage_dd_submit(state, data).await {
        return;
    }
    // 待办管理卡「新增」提交（提问卡模板复用，spec todo-whats-next D8）。
    if handle_todo_dd_submit(state, data).await {
        return;
    }
    let Some((otid, session_id)) = crate::dingtalk::select::parse_select_action(data) else {
        return;
    };
    let picker = {
        let pickers = state.select.pickers.lock().unwrap();
        pickers
            .iter()
            .find(|p| p.channel == "dingding" && p.message_id == otid)
            .cloned()
    };
    let Some(picker) = picker else {
        return; // 已过期 / 被清理：静默（D7）。
    };
    // 路由靠 param 回传的 session_id（不用会漂移的编号）；空 / 不属于本卡 → 无效（模板未绑定或已变）。
    if session_id.is_empty() || !picker.options.contains(&session_id) {
        return;
    }
    let lang = Lang::current();
    let config = state.config_snapshot();
    match picker.kind {
        PickerKind::TaskWorkspace | PickerKind::TaskAgent | PickerKind::TaskPermission => {
            select_pick_task_flow(
                state,
                "dingding",
                &otid,
                &session_id,
                &picker,
                &config,
                lang,
                None,
            )
            .await;
        }
        PickerKind::Watch => {
            dd_select_pick_watch(state, &otid, &session_id, &config, lang).await;
            // 卡片点『关注』＝在该渠道操作 → 设为活跃槽（与 /watch 一致，用户决策）。
            activate_channel_on_action(state, "dingding", &config, lang).await;
        }
        PickerKind::Status => {
            // 单选卡不动：回纯文本详情（可继续点其它 agent）。
            let snapshot = state.agents.snapshot();
            let text = status_detail_by_session(&snapshot, &session_id, "dingding", lang);
            let _ = reply_channel_text("dingding", &config, &text).await;
            // 卡片点『查看』＝在该渠道操作 → 设为活跃槽（补齐与 /status 文本命令的一致性）。
            activate_channel_on_action(state, "dingding", &config, lang).await;
        }
        PickerKind::Unwatch => {
            dd_select_pick_unwatch(state, &otid, &session_id, &config, lang).await;
        }
        PickerKind::Msg => {
            let content = picker.payload.clone().unwrap_or_default();
            dd_select_pick_msg(state, &otid, &session_id, &content, &config, lang).await;
            // 卡片点『发送』＝在该渠道操作 → 设为活跃槽（与 /msg 一致，用户决策）。
            activate_channel_on_action(state, "dingding", &config, lang).await;
        }
        PickerKind::Diff | PickerKind::Stage | PickerKind::Transcript => {
            select_pick_export(
                state,
                "dingding",
                &otid,
                &session_id,
                picker.kind,
                &config,
                lang,
                None,
            )
            .await;
            activate_channel_on_action(state, "dingding", &config, lang).await;
        }
        // /todo · /todo-rm（spec todo-whats-next D8）。
        PickerKind::Todo => {
            dd_select_pick_todo(state, &otid, &session_id, &config, lang).await;
            activate_channel_on_action(state, "dingding", &config, lang).await;
        }
        PickerKind::TodoRm => {
            dd_select_pick_todo_rm(state, &otid, &session_id, &config, lang).await;
            activate_channel_on_action(state, "dingding", &config, lang).await;
        }
        PickerKind::TodoRmEntry => {
            dd_select_pick_todo_rm_entry(state, &otid, &session_id, &picker, &config, lang).await;
        }
        PickerKind::TodoAuto => {
            dd_select_pick_todo_auto(state, &otid, &session_id, &config, lang).await;
            activate_channel_on_action(state, "dingding", &config, lang).await;
        }
        PickerKind::TodoAutoEntry => {
            dd_select_pick_todo_auto_entry(state, &otid, &session_id, &picker, &config, lang).await;
        }
        // 管理卡提交已在上方 handle_todo_dd_submit 处理；行按钮不存在。
        PickerKind::TodoManage => {}
    }
}

/// 钉钉单选卡点选「发送」：投递（若目标仍工作中·非 grok）+ 单选卡定格（OpenAPI 更新）。
pub(super) async fn dd_select_pick_msg(
    state: &Arc<ServerState>,
    otid: &str,
    session_id: &str,
    content: &str,
    config: &AppConfig,
    lang: Lang,
) {
    let snapshot = state.agents.snapshot();
    let rec = find_agent_by_session(&snapshot, session_id);
    let label = msg_pick_deliver(state, "dingding", session_id, rec, content, lang);
    dd_finalize_select_card(config, otid, &label).await;
    remove_picker(state, "dingding", otid);
}

/// 钉钉单选卡点选「watch」：钉钉不能就地变身（模板固定），故**另发一张新的实时 watch 卡** +
/// 把单选卡定格「已选择 [n]」。已在关注同一 session ＝换新卡（`register_watch_at` 定格旧卡）。
pub(super) async fn dd_select_pick_watch(
    state: &Arc<ServerState>,
    otid: &str,
    session_id: &str,
    config: &AppConfig,
    lang: Lang,
) {
    let now = now_secs();
    let snapshot = state.agents.snapshot();
    let rec = find_agent_by_session(&snapshot, session_id);
    let waiting = state
        .registry
        .in_flight_agent_session_ids()
        .contains(&session_id.to_string());
    let seq = rec
        .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let frame = crate::watch::build_frame(seq, rec, waiting);
    let ended = frame.phase == crate::watch::WatchPhase::Ended;
    // 上限校验（本渠道；已在关注同一 session＝换新卡，不计新增；已结束不订阅、不计数）。
    let already = state
        .watch
        .subs
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.channel == "dingding" && s.session_id == session_id);
    if !ended && !already {
        let count = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.channel == "dingding" && !s.rewatchable)
            .count();
        if count >= crate::watch::MAX_WATCHES {
            let text = crate::i18n::tr(lang, "watch.limit")
                .replace("{n}", &crate::watch::MAX_WATCHES.to_string())
                .replace("{p}", crate::autochannel::cmd_prefix("dingding"));
            let _ = reply_channel_text("dingding", config, &text).await;
            return; // 单选卡不动，可另选。
        }
    }
    // 另发一张实时 watch 卡（活动态活卡 / 已结束则终态卡）。
    let Some(client) = watch_client(state, "dingding", config).await else {
        return;
    };
    let mode = if ended {
        crate::watch::CardMode::Final(crate::watch::FinalKind::Ended)
    } else {
        crate::watch::CardMode::Active
    };
    let new_mid = match client.send(&frame, mode, now, lang).await {
        Ok(m) => m,
        Err(err) => {
            log(&format!("select: send dingtalk watch card failed: {}", err));
            return;
        }
    };
    // 登记订阅（含换新卡：本渠道同 session 旧卡定格 Replaced）+ 让 watch 引擎认领新卡按钮。
    register_watch_at(
        state, "dingding", session_id, seq, &new_mid, &frame, ended, config, lang,
    )
    .await;
    ensure_watch_routes(state).await;
    // 单选卡定格「已选择 [n]」并消费 picker。
    let label = crate::i18n::tr(lang, "select.pickedCard").replace("{id}", &seq.to_string());
    dd_finalize_select_card(config, otid, &label).await;
    remove_picker(state, "dingding", otid);
}

/// 钉钉单选卡点选「unwatch」：取消该关注（旧 watch 卡定格）+ 回文本确认 + 就地刷新单选卡
/// （经 OpenAPI 更新 loop；取到 0 则定格「已全部取消关注」）。
pub(super) async fn dd_select_pick_unwatch(
    state: &Arc<ServerState>,
    otid: &str,
    session_id: &str,
    config: &AppConfig,
    lang: Lang,
) {
    let now = now_secs();
    let entry = state
        .watch
        .subs
        .lock()
        .unwrap()
        .iter()
        .find(|s| s.channel == "dingding" && s.session_id == session_id)
        .cloned();
    if let Some(entry) = entry {
        if let Some(client) = watch_client(state, "dingding", config).await {
            let snapshot = state.agents.snapshot();
            let waiting = state
                .registry
                .in_flight_agent_session_ids()
                .contains(&entry.session_id);
            let frame = crate::watch::build_frame(
                entry.seq,
                find_agent_by_session(&snapshot, &entry.session_id),
                waiting,
            );
            if let Err(err) = client
                .edit(
                    &entry.message_id,
                    &frame,
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Cancelled),
                    now,
                    lang,
                    Some(&entry.session_id),
                )
                .await
            {
                log(&format!(
                    "select: finalize dingtalk unwatch card failed: {}",
                    err
                ));
            }
        }
        {
            let mut subs = state.watch.subs.lock().unwrap();
            if let Some(s) = subs.iter_mut().find(|s| s.message_id == entry.message_id) {
                s.rewatchable = true;
            }
        }
        persist_watch_subs(state);
        state.watch.notify.notify_one();
        ensure_watch_routes(state).await;
        let text =
            crate::i18n::tr(lang, "watch.unwatchDone").replace("{id}", &entry.seq.to_string());
        let _ = reply_channel_text("dingding", config, &text).await;
    }
    // 就地刷新单选卡：剩余订阅 → 更新 loop；空 → 定格「已全部取消关注」并消费 picker。
    let snapshot = state.agents.snapshot();
    let options = unwatch_options(state, "dingding", &snapshot, lang);
    if options.is_empty() {
        dd_finalize_select_card(
            config,
            otid,
            crate::i18n::tr(lang, "select.unwatchAllDoneCard"),
        )
        .await;
        remove_picker(state, "dingding", otid);
    } else {
        let view = crate::select::build_view(
            crate::select::title_unwatch(lang),
            options,
            crate::select::SelectAction::Unwatch,
            lang,
        );
        let new_ids: Vec<String> = view.options.iter().map(|o| o.id.clone()).collect();
        if let Ok(client) = crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding)
        {
            let map = crate::dingtalk::select::build_select_param_map(&view, lang);
            if let Err(err) = client
                .update_card_private(otid, map, serde_json::json!({}))
                .await
            {
                log(&format!(
                    "select: refresh dingtalk unwatch card failed: {}",
                    err
                ));
            }
        }
        if let Some(p) = state
            .select
            .pickers
            .lock()
            .unwrap()
            .iter_mut()
            .find(|p| p.channel == "dingding" && p.message_id == otid)
        {
            p.options = new_ids;
        }
    }
}

/// 定格一张钉钉单选卡（按 key 更新公有 `finalized=true` + `final_label`）：隐藏循环、显示定格文案。
pub(super) async fn dd_finalize_select_card(config: &AppConfig, otid: &str, final_label: &str) {
    if let Ok(client) = crate::dingtalk::client::DingTalkClient::new(&config.channels.dingding) {
        let map = crate::dingtalk::select::build_select_final_param_map(final_label);
        if let Err(err) = client
            .update_card_private(otid, map, serde_json::json!({}))
            .await
        {
            log(&format!(
                "select: finalize dingtalk select card failed: {}",
                err
            ));
        }
    }
}

// ===== Telegram / Slack 单选卡点选（可就地编辑：点 watch → 本消息变身为实时 watch 卡）=====

/// 处理 Telegram 单选卡 / 确认卡点击：应答消除转圈 → 解析 → 分派。
pub(super) async fn handle_select_tg_action(state: &Arc<ServerState>, cb: &serde_json::Value) {
    let data = cb.get("data").and_then(|v| v.as_str()).unwrap_or("");
    let Some(mid) = cb
        .get("message")
        .and_then(|m| m.get("message_id"))
        .and_then(|v| v.as_i64())
    else {
        return;
    };
    let config = state.config_snapshot();
    // 应答 callback（消除客户端转圈，best-effort）。
    if let Some(id) = cb.get("id").and_then(|i| i.as_str()) {
        let tg = &config.channels.telegram;
        if let Ok(c) = crate::telegram::TelegramClient::new(
            tg.bot_token.clone(),
            tg.chat_id.clone(),
            tg.api_base_url.clone(),
        ) {
            c.answer_callback_query(id).await;
        }
    }
    if let Some(slot) = crate::telegram::confirm::parse_confirm_action(data) {
        handle_confirm_action(state, "telegram", &mid.to_string(), slot, None).await;
        return;
    }
    let Some(idx) = crate::telegram::select::parse_select_action(data) else {
        return;
    };
    dispatch_select_pick(state, "telegram", &mid.to_string(), idx, &config).await;
}

/// 处理 Slack 单选卡 / 确认卡点击（ack 已在 ws 层完成）。
pub(super) async fn handle_select_slack_action(
    state: &Arc<ServerState>,
    payload: &serde_json::Value,
) {
    let config = state.config_snapshot();
    if let Some((ts, slot)) = crate::slack::confirm::parse_confirm_action(payload) {
        handle_confirm_action(state, "slack", &ts, slot, None).await;
        return;
    }
    let Some((ts, idx)) = crate::slack::select::parse_select_action(payload) else {
        return;
    };
    dispatch_select_pick(state, "slack", &ts, idx, &config).await;
}

/// TG/Slack 共用的下标分派：找 picker → 按下标取 session_id → 按 kind 处理。
/// 过期 / 越界 / 无 picker → 静默（D7）。
pub(super) async fn dispatch_select_pick(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    idx: usize,
    config: &AppConfig,
) {
    let picker = {
        let pickers = state.select.pickers.lock().unwrap();
        pickers
            .iter()
            .find(|p| p.channel == channel_id && p.message_id == mid)
            .cloned()
    };
    let Some(picker) = picker else {
        return; // 已过期 / 被清理：静默（D7）。
    };
    let Some(session_id) = picker.options.get(idx).cloned() else {
        return;
    };
    let lang = Lang::current();
    match picker.kind {
        PickerKind::TaskWorkspace | PickerKind::TaskAgent | PickerKind::TaskPermission => {
            select_pick_task_flow(
                state,
                channel_id,
                mid,
                &session_id,
                &picker,
                config,
                lang,
                None,
            )
            .await;
        }
        PickerKind::Watch => {
            select_pick_watch_inplace(state, channel_id, mid, &session_id, config, lang).await;
            // 卡片点『关注』＝在该渠道操作 → 设为活跃槽（与 /watch 一致，用户决策）。
            activate_channel_on_action(state, channel_id, config, lang).await;
        }
        PickerKind::Status => {
            let snapshot = state.agents.snapshot();
            let text = status_detail_by_session(&snapshot, &session_id, channel_id, lang);
            let _ = reply_channel_text(channel_id, config, &text).await;
            // 卡片点『查看』＝在该渠道操作 → 设为活跃槽（补齐与 /status 文本命令的一致性）。
            activate_channel_on_action(state, channel_id, config, lang).await;
        }
        PickerKind::Unwatch => {
            select_pick_unwatch_inplace(state, channel_id, mid, &session_id, config, lang).await;
        }
        PickerKind::Msg => {
            let content = picker.payload.clone().unwrap_or_default();
            select_pick_msg_inplace(state, channel_id, mid, &session_id, &content, config, lang)
                .await;
            // 卡片点『发送』＝在该渠道操作 → 设为活跃槽（与 /msg 一致，用户决策）。
            activate_channel_on_action(state, channel_id, config, lang).await;
        }
        PickerKind::Diff | PickerKind::Stage | PickerKind::Transcript => {
            select_pick_export(
                state,
                channel_id,
                mid,
                &session_id,
                picker.kind,
                config,
                lang,
                None,
            )
            .await;
            activate_channel_on_action(state, channel_id, config, lang).await;
        }
        // /todo · /todo-rm（spec todo-whats-next D8）。
        PickerKind::Todo => {
            select_pick_todo_text(state, channel_id, mid, &session_id, config, lang).await;
            activate_channel_on_action(state, channel_id, config, lang).await;
        }
        PickerKind::TodoRm => {
            select_pick_todo_rm_inplace(state, channel_id, mid, &session_id, config, lang).await;
            activate_channel_on_action(state, channel_id, config, lang).await;
        }
        PickerKind::TodoRmEntry => {
            select_pick_todo_rm_entry_inplace(
                state,
                channel_id,
                mid,
                &session_id,
                &picker,
                config,
                lang,
            )
            .await;
        }
        PickerKind::TodoAuto => {
            select_pick_todo_auto_inplace(state, channel_id, mid, &session_id, config, lang).await;
            activate_channel_on_action(state, channel_id, config, lang).await;
        }
        PickerKind::TodoAutoEntry => {
            select_pick_todo_auto_entry_inplace(
                state,
                channel_id,
                mid,
                &session_id,
                &picker,
                config,
                lang,
            )
            .await;
        }
        // 管理卡在 TG/Slack 上是纯文本形态，不会有卡片回调。
        PickerKind::TodoManage => {}
    }
}

/// 单选卡点选「发送」（TG/Slack 就地定格）：投递（若目标仍工作中·非 grok）+ 定格本单选卡。
pub(super) async fn select_pick_msg_inplace(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    session_id: &str,
    content: &str,
    config: &AppConfig,
    lang: Lang,
) {
    let snapshot = state.agents.snapshot();
    let rec = find_agent_by_session(&snapshot, session_id);
    let label = msg_pick_deliver(state, channel_id, session_id, rec, content, lang);
    finalize_select_card_edit(
        channel_id,
        config,
        mid,
        &crate::select::title_msg(lang),
        &label,
    )
    .await;
    remove_picker(state, channel_id, mid);
}

/// 单选卡点选「watch」（TG/Slack 可就地编辑）：把本消息编辑成实时 watch 卡（`WatchClient::edit`），
/// 登记订阅（含换新卡收尾）+ 消费 picker + 让 watch 引擎认领本消息。已结束则定格终态卡、不订阅。
pub(super) async fn select_pick_watch_inplace(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    session_id: &str,
    config: &AppConfig,
    lang: Lang,
) {
    let now = now_secs();
    let snapshot = state.agents.snapshot();
    let rec = find_agent_by_session(&snapshot, session_id);
    let waiting = state
        .registry
        .in_flight_agent_session_ids()
        .contains(&session_id.to_string());
    let seq = rec
        .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let frame = crate::watch::build_frame(seq, rec, waiting);
    let ended = frame.phase == crate::watch::WatchPhase::Ended;
    if ended {
        // 已结束/消失：就地把本消息编辑成终态卡、不订阅、消费掉 picker。
        if let Some(client) = watch_client(state, channel_id, config).await {
            if let Err(err) = client
                .edit(
                    mid,
                    &frame,
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Ended),
                    now,
                    lang,
                    None,
                )
                .await
            {
                log(&format!(
                    "select: transform to ended watch card failed ({}): {}",
                    channel_id, err
                ));
                return;
            }
        }
        remove_picker(state, channel_id, mid);
        return;
    }
    // 上限校验（本渠道；已在关注同一 session＝换新卡，不计新增）。
    let already = state
        .watch
        .subs
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.channel == channel_id && s.session_id == session_id);
    if !already {
        let count = state
            .watch
            .subs
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.channel == channel_id && !s.rewatchable)
            .count();
        if count >= crate::watch::MAX_WATCHES {
            let text = crate::i18n::tr(lang, "watch.limit")
                .replace("{n}", &crate::watch::MAX_WATCHES.to_string())
                .replace("{p}", crate::autochannel::cmd_prefix(channel_id));
            let _ = reply_channel_text(channel_id, config, &text).await;
            return; // 单选卡不动，可另选。
        }
    }
    // 就地把这条单选卡消息编辑成实时 watch 卡。
    let Some(client) = watch_client(state, channel_id, config).await else {
        return;
    };
    if let Err(err) = client
        .edit(mid, &frame, crate::watch::CardMode::Active, now, lang, None)
        .await
    {
        log(&format!(
            "select: transform select card to watch card failed ({}): {}",
            channel_id, err
        ));
        return;
    }
    register_watch_at(
        state, channel_id, session_id, seq, mid, &frame, false, config, lang,
    )
    .await;
    remove_picker(state, channel_id, mid);
    ensure_watch_routes(state).await;
}

/// 单选卡点选「unwatch」（TG/Slack）：取消该关注（旧 watch 卡定格）+ 文本确认 + 就地刷新本单选卡
/// （移除该项；取到 0 则定格「已全部取消关注」）。
pub(super) async fn select_pick_unwatch_inplace(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    session_id: &str,
    config: &AppConfig,
    lang: Lang,
) {
    let now = now_secs();
    let entry = state
        .watch
        .subs
        .lock()
        .unwrap()
        .iter()
        .find(|s| s.channel == channel_id && s.session_id == session_id)
        .cloned();
    if let Some(entry) = entry {
        if let Some(client) = watch_client(state, channel_id, config).await {
            let snapshot = state.agents.snapshot();
            let waiting = state
                .registry
                .in_flight_agent_session_ids()
                .contains(&entry.session_id);
            let frame = crate::watch::build_frame(
                entry.seq,
                find_agent_by_session(&snapshot, &entry.session_id),
                waiting,
            );
            if let Err(err) = client
                .edit(
                    &entry.message_id,
                    &frame,
                    crate::watch::CardMode::Final(crate::watch::FinalKind::Cancelled),
                    now,
                    lang,
                    Some(&entry.session_id),
                )
                .await
            {
                log(&format!(
                    "select: finalize unwatch card failed ({}): {}",
                    channel_id, err
                ));
            }
        }
        {
            let mut subs = state.watch.subs.lock().unwrap();
            if let Some(s) = subs.iter_mut().find(|s| s.message_id == entry.message_id) {
                s.rewatchable = true;
            }
        }
        persist_watch_subs(state);
        state.watch.notify.notify_one();
        ensure_watch_routes(state).await;
        let text =
            crate::i18n::tr(lang, "watch.unwatchDone").replace("{id}", &entry.seq.to_string());
        let _ = reply_channel_text(channel_id, config, &text).await;
    }
    // 就地刷新单选卡：剩余订阅 → 新卡；空 → 定格「已全部取消关注」并消费 picker。
    let snapshot = state.agents.snapshot();
    let options = unwatch_options(state, channel_id, &snapshot, lang);
    if options.is_empty() {
        finalize_select_card_edit(
            channel_id,
            config,
            mid,
            &crate::select::title_unwatch(lang),
            crate::i18n::tr(lang, "select.unwatchAllDoneCard"),
        )
        .await;
        remove_picker(state, channel_id, mid);
    } else {
        let view = crate::select::build_view(
            crate::select::title_unwatch(lang),
            options,
            crate::select::SelectAction::Unwatch,
            lang,
        );
        let new_ids: Vec<String> = view.options.iter().map(|o| o.id.clone()).collect();
        refresh_select_card_edit(channel_id, config, mid, &view, lang).await;
        if let Some(p) = state
            .select
            .pickers
            .lock()
            .unwrap()
            .iter_mut()
            .find(|p| p.channel == channel_id && p.message_id == mid)
        {
            p.options = new_ids;
        }
    }
}

/// 就地把 TG/Slack 单选卡编辑为新的一版单选卡（`/unwatch` 移除该项后刷新）。
pub(super) async fn refresh_select_card_edit(
    channel_id: &str,
    config: &AppConfig,
    mid: &str,
    view: &crate::select::SelectView,
    lang: Lang,
) {
    match channel_id {
        "telegram" => {
            let Ok(m) = mid.parse::<i64>() else { return };
            let tg = &config.channels.telegram;
            if let Ok(c) = crate::telegram::TelegramClient::new(
                tg.bot_token.clone(),
                tg.chat_id.clone(),
                tg.api_base_url.clone(),
            ) {
                let html = crate::telegram::select::render_select_html(view);
                let markup = crate::telegram::select::inline_keyboard(view, lang);
                if let Err(err) = c
                    .edit_message_text(m, &html, Some("HTML"), Some(markup))
                    .await
                {
                    log(&format!(
                        "select: refresh telegram select card failed: {}",
                        err
                    ));
                }
            }
        }
        "slack" => {
            if let Ok(c) = crate::slack::client::SlackClient::new(&config.channels.slack) {
                if let Ok(dm) = c.open_dm().await {
                    let (blocks, fallback) = crate::slack::select::build_select_blocks(view, lang);
                    if let Err(err) = c.update_message(&dm, mid, Some(&blocks), &fallback).await {
                        log(&format!(
                            "select: refresh slack select card failed: {}",
                            err
                        ));
                    }
                }
            }
        }
        _ => {}
    }
}

/// 就地把 TG/Slack 单选卡定格为无按钮终态（标题 + 定格文案）。
pub(super) async fn finalize_select_card_edit(
    channel_id: &str,
    config: &AppConfig,
    mid: &str,
    title: &str,
    final_label: &str,
) {
    match channel_id {
        "telegram" => {
            let Ok(m) = mid.parse::<i64>() else { return };
            let tg = &config.channels.telegram;
            if let Ok(c) = crate::telegram::TelegramClient::new(
                tg.bot_token.clone(),
                tg.chat_id.clone(),
                tg.api_base_url.clone(),
            ) {
                let html = crate::telegram::select::render_select_final_html(title, final_label);
                if let Err(err) = c.edit_message_text(m, &html, Some("HTML"), None).await {
                    log(&format!(
                        "select: finalize telegram select card failed: {}",
                        err
                    ));
                }
            }
        }
        "slack" => {
            if let Ok(c) = crate::slack::client::SlackClient::new(&config.channels.slack) {
                if let Ok(dm) = c.open_dm().await {
                    let (blocks, fallback) =
                        crate::slack::select::build_select_final_blocks(title, final_label);
                    if let Err(err) = c.update_message(&dm, mid, Some(&blocks), &fallback).await {
                        log(&format!(
                            "select: finalize slack select card failed: {}",
                            err
                        ));
                    }
                }
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn select_pick_export(
    state: &Arc<ServerState>,
    channel_id: &str,
    mid: &str,
    session_id: &str,
    kind: PickerKind,
    config: &AppConfig,
    lang: Lang,
    ack: Option<crate::feishu::router::CardAck>,
) {
    let snapshot = state.agents.snapshot();
    let seq = find_agent_by_session(&snapshot, session_id)
        .and_then(|r| r.get("seq").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let label_key = match kind {
        PickerKind::Diff => "select.diffDoneCard",
        PickerKind::Stage => "select.stageOpenedCard",
        PickerKind::Transcript => "select.transcriptDoneCard",
        _ => "select.diffDoneCard",
    };
    let title = match kind {
        PickerKind::Diff => crate::select::title_diff(lang),
        PickerKind::Stage => crate::select::title_stage(lang),
        PickerKind::Transcript => crate::select::title_transcript(lang),
        _ => String::new(),
    };
    let label = crate::i18n::tr(lang, label_key).replace("{id}", &seq.to_string());
    if channel_id == "feishu" {
        if let Some(ack) = ack {
            let card = crate::feishu::card::build_select_final_card(&title, &label);
            let _ = ack.send(Some(crate::feishu::card::callback_update_card(card)));
        }
    } else if channel_id == "dingding" {
        dd_finalize_select_card(config, mid, &label).await;
    } else {
        finalize_select_card_edit(channel_id, config, mid, &title, &label).await;
    }
    remove_picker(state, channel_id, mid);
    match kind {
        PickerKind::Diff => run_diff(state, channel_id, session_id, config, lang).await,
        PickerKind::Transcript => run_transcript(state, channel_id, session_id, config, lang).await,
        PickerKind::Stage => run_stage_confirm(state, channel_id, session_id, config, lang).await,
        _ => {}
    }
}
