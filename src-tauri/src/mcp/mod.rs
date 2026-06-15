//! `AskHuman mcp`：以 STDIO 运行 MCP server，暴露单工具 `ask`。
//!
//! 设计为「薄壳」：每次 `ask` 调用都 spawn 一个现有的 `AskHuman --output json …` 子进程，
//! 复用全部既有 ask 流程（弹窗 / IM / 抢答 / 历史 / 落盘 / 排空与自动重连），再把人类回复中的
//! 图片读回转成 MCP `ImageContent` 一并返回。全平台同一套；daemon 换新 / 重启后下一次调用自动重连
//! （每次调用都是新起子进程、重新连接 daemon，因此 MCP server 进程可长期存活、跨 daemon 重启）。

mod ask;

use rmcp::{transport::stdio, ServiceExt};

/// 进入 STDIO MCP server 事件循环（不返回）。
pub fn run() -> ! {
    let code = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt.block_on(serve()),
        Err(_) => 3,
    };
    std::process::exit(code);
}

/// 建 server、握手、等关闭。返回进程退出码。
async fn serve() -> i32 {
    match ask::AskServer::new().serve(stdio()).await {
        Ok(service) => {
            let _ = service.waiting().await;
            0
        }
        // 握手失败（如非 MCP 客户端误启）：直接退出，stdout 不能有杂音。
        Err(_) => 3,
    }
}
