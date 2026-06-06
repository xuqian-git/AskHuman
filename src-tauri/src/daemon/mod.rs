//! 常驻 Daemon：子命令分发（run/start/stop/restart/status/logs）+ Phase 0 的空 Daemon 服务。
//!
//! Phase 0：起一个不承载任何渠道的空 Daemon，提供握手（含二进制指纹换新）、status、stop、
//! 单实例（flock）、自启、空闲退出。渠道 / 弹窗 / 提交将在后续 Phase 接入。

pub mod lifecycle;
#[cfg(unix)]
pub mod spawn;

/// `AskHuman daemon <sub>` 入口。永不返回（自行退出进程）。
pub fn dispatch(args: &[String]) -> ! {
    #[cfg(unix)]
    {
        std::process::exit(unix_impl::dispatch(args));
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        eprintln!("AskHuman daemon is not supported on this platform yet.");
        std::process::exit(1);
    }
}

#[cfg(unix)]
mod unix_impl {
    use super::lifecycle::{self, DaemonMeta, LockGuard};
    use crate::client;
    use crate::ipc::{self, transport, ClientMsg, HelloAck, HelloStatus, ServerMsg, StatusInfo};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::io::BufReader;
    use tokio::net::UnixStream;

    /// 无活动且无连接持续此时长后自动退出。可用 `ASKHUMAN_DAEMON_IDLE_SECS` 覆盖（便于测试）。
    const DEFAULT_IDLE_SECS: u64 = 300;

    fn idle_timeout() -> Duration {
        let secs = std::env::var("ASKHUMAN_DAEMON_IDLE_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_IDLE_SECS);
        Duration::from_secs(secs)
    }

    fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn log(msg: &str) {
        // `daemon run` 经 spawn 时 stderr 已重定向到 daemon.log；前台运行则打到终端。
        eprintln!("[askhuman-daemon {}] {}", now_secs(), msg);
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime")
            .block_on(f)
    }

    pub fn dispatch(args: &[String]) -> i32 {
        match args.first().map(|s| s.as_str()).unwrap_or("") {
            "run" => run_cmd(),
            "start" => start_cmd(),
            "stop" => stop_cmd(),
            "restart" => restart_cmd(),
            "status" => status_cmd(),
            "logs" => logs_cmd(),
            "" => {
                eprintln!("usage: AskHuman daemon <run|start|stop|restart|status|logs>");
                1
            }
            other => {
                eprintln!("unknown daemon subcommand: {}", other);
                eprintln!("usage: AskHuman daemon <run|start|stop|restart|status|logs>");
                1
            }
        }
    }

    // —— run：前台运行 Daemon 主体 ——

    fn run_cmd() -> i32 {
        let lock = match lifecycle::acquire_lock() {
            Ok(Some(l)) => l,
            Ok(None) => {
                log("another daemon is already running; exiting");
                return 0;
            }
            Err(e) => {
                log(&format!("failed to acquire lock: {}", e));
                return 1;
            }
        };
        let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                log(&format!("failed to build runtime: {}", e));
                return 1;
            }
        };
        rt.block_on(serve(lock))
    }

    struct ServerState {
        startup_fp: lifecycle::Fingerprint,
        started_at: u64,
        active: AtomicUsize,
        last_active: Mutex<Instant>,
        shutdown: tokio::sync::Notify,
    }

    async fn serve(_lock: LockGuard) -> i32 {
        let listener = match transport::bind() {
            Ok(l) => l,
            Err(e) => {
                log(&format!("failed to bind socket: {}", e));
                return 1;
            }
        };
        let startup_fp = lifecycle::current_fingerprint();
        let started_at = now_secs();
        let meta = DaemonMeta {
            pid: std::process::id(),
            version: version(),
            protocol_version: ipc::PROTOCOL_VERSION,
            started_at,
            socket: transport::socket_path().display().to_string(),
            fingerprint: startup_fp,
        };
        if let Err(e) = lifecycle::write_meta(&meta) {
            log(&format!("failed to write daemon.json: {}", e));
        }
        log(&format!(
            "started pid={} version={} protocol={}",
            meta.pid, meta.version, meta.protocol_version
        ));

        let state = Arc::new(ServerState {
            startup_fp,
            started_at,
            active: AtomicUsize::new(0),
            last_active: Mutex::new(Instant::now()),
            shutdown: tokio::sync::Notify::new(),
        });

        // 空闲退出检查。
        {
            let state = state.clone();
            tokio::spawn(async move {
                let timeout = idle_timeout();
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    if state.active.load(Ordering::SeqCst) == 0 {
                        let idle = state.last_active.lock().map(|t| t.elapsed()).unwrap_or_default();
                        if idle >= timeout {
                            log("idle timeout reached; shutting down");
                            state.shutdown.notify_one();
                            break;
                        }
                    }
                }
            });
        }

        loop {
            tokio::select! {
                _ = state.shutdown.notified() => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _addr)) => {
                        let st = state.clone();
                        tokio::spawn(async move { handle_conn(stream, st).await; });
                    }
                    Err(e) => log(&format!("accept error: {}", e)),
                }
            }
        }

        cleanup();
        log("stopped");
        0
    }

    async fn handle_conn(stream: UnixStream, state: Arc<ServerState>) {
        state.active.fetch_add(1, Ordering::SeqCst);
        let (r, mut w) = stream.into_split();
        let mut reader = BufReader::new(r);

        loop {
            let msg: Option<ClientMsg> = match ipc::read_msg(&mut reader).await {
                Ok(m) => m,
                Err(e) => {
                    log(&format!("read error: {}", e));
                    break;
                }
            };
            let Some(msg) = msg else { break }; // EOF / 对端关闭

            match msg {
                ClientMsg::Hello(hello) => {
                    let now_fp = lifecycle::current_fingerprint();
                    // 自身二进制被换 / 客户端二进制不一致 / 协议不一致 → 过时，让位换新。
                    let stale = now_fp != state.startup_fp
                        || hello.fingerprint != state.startup_fp
                        || hello.protocol_version != ipc::PROTOCOL_VERSION;
                    let auto_restart = std::env::var("ASKHUMAN_DAEMON_AUTORESTART")
                        .map(|v| v != "0")
                        .unwrap_or(true);
                    let restarting = stale && auto_restart;
                    let ack = HelloAck {
                        protocol_version: ipc::PROTOCOL_VERSION,
                        daemon_version: version(),
                        status: if restarting {
                            HelloStatus::Restarting
                        } else {
                            HelloStatus::Ok
                        },
                        reason: if restarting {
                            Some("binary or protocol changed".to_string())
                        } else {
                            None
                        },
                    };
                    let _ = ipc::write_msg(&mut w, &ServerMsg::HelloAck(ack)).await;
                    if restarting {
                        log("stale binary/protocol detected; shutting down for restart");
                        state.shutdown.notify_one();
                        break;
                    }
                }
                ClientMsg::Status => {
                    let info = StatusInfo {
                        pid: std::process::id(),
                        version: version(),
                        protocol_version: ipc::PROTOCOL_VERSION,
                        uptime_secs: now_secs().saturating_sub(state.started_at),
                        socket: transport::socket_path().display().to_string(),
                        active_requests: 0, // Phase 0：尚无请求
                    };
                    let _ = ipc::write_msg(&mut w, &ServerMsg::Status(info)).await;
                }
                ClientMsg::Stop => {
                    let _ = ipc::write_msg(&mut w, &ServerMsg::Stopping).await;
                    log("stop requested");
                    state.shutdown.notify_one();
                    break;
                }
            }
        }

        if let Ok(mut t) = state.last_active.lock() {
            *t = Instant::now();
        }
        state.active.fetch_sub(1, Ordering::SeqCst);
    }

    fn cleanup() {
        let _ = std::fs::remove_file(transport::socket_path());
        let _ = std::fs::remove_file(lifecycle::meta_path());
    }

    // —— start / stop / restart / status / logs：作为客户端操作 Daemon ——

    fn start_cmd() -> i32 {
        block_on(async {
            match client::ensure_running().await {
                Ok(()) => {
                    match client::request_status().await {
                        Some(info) => print_status(&info),
                        None => println!("askhuman daemon: running"),
                    }
                    0
                }
                Err(e) => {
                    eprintln!("failed to start daemon: {}", e);
                    1
                }
            }
        })
    }

    fn stop_cmd() -> i32 {
        block_on(async {
            if client::request_stop().await {
                println!("askhuman daemon: stopping");
            } else {
                println!("askhuman daemon: not running");
            }
            0
        })
    }

    fn restart_cmd() -> i32 {
        block_on(async {
            let _ = client::request_stop().await;
            client::wait_until_down(Duration::from_secs(5)).await;
            match client::ensure_running().await {
                Ok(()) => {
                    println!("askhuman daemon: restarted");
                    0
                }
                Err(e) => {
                    eprintln!("failed to restart daemon: {}", e);
                    1
                }
            }
        })
    }

    fn status_cmd() -> i32 {
        block_on(async {
            match client::request_status().await {
                Some(info) => {
                    print_status(&info);
                    0
                }
                None => {
                    println!("askhuman daemon: not running");
                    1
                }
            }
        })
    }

    fn logs_cmd() -> i32 {
        let p = lifecycle::log_path();
        println!("daemon log: {}", p.display());
        println!("(tip: tail -f {})", p.display());
        0
    }

    fn print_status(info: &StatusInfo) {
        println!("askhuman daemon: running");
        println!("  pid        {}", info.pid);
        println!(
            "  version    {} (protocol {})",
            info.version, info.protocol_version
        );
        println!("  uptime     {}s", info.uptime_secs);
        println!("  socket     {}", info.socket);
        println!("  requests   {} active", info.active_requests);
    }
}
