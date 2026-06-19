#!/usr/bin/env node
// Local mock IM server for the deterministic popup-launch perf harness
// (plan docs/plans/perf-harness-deterministic-mock-im.md).
//
// One process, one 127.0.0.1 port, serving BOTH the HTTP OpenAPIs and the WebSocket endpoints of
// all four supported channels (DingTalk / Feishu / Telegram / Slack). It does the MINIMUM each
// channel's `Router::connect()` needs to succeed and stay alive, and accepts the outgoing
// card/message sends so each channel's own send code (build card -> serialize -> HTTP) runs for real.
//
// Two deliberate ~150ms delays expose "IM blocks the popup" regressions:
//   - connect/open responses  (dd gateway/connections/open, sl apps.connections.open,
//     fs callback/ws/endpoint) -> these are awaited inside `connect()`, on today's popup path.
//   - send responses          (dd createAndDeliver, sl chat.postMessage, fs im/v1/messages,
//     tg sendMessage)          -> today these run on a detached task (off the path); the delay is a
//     future probe: if someone makes sending block the popup, it surfaces in the e2e number.
//
// The WS endpoints only complete the handshake and keep the socket open (draining input); no frames
// are decoded/encoded — connect succeeds after the upgrade and `is_alive()` stays true. That is all
// the connect path observes (the popup auto-dismisses, so no user reply needs to be simulated).
//
// Usage (standalone, for debugging):
//   node scripts/perf-mock-im.mjs [--port N] [--delay MS]
// Programmatic (used by perf-popup.mjs):
//   import { startMockIm } from "./perf-mock-im.mjs";
//   const mock = await startMockIm({ delayMs: 150 });
//   // mock.port, mock.urls.{telegram,dingtalk,slack,feishu}, mock.close()

import http from "node:http";
import crypto from "node:crypto";

const WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const DEFAULT_DELAY_MS = 150;

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

/** Read and discard a request body (frees the socket); resolves when the body is fully consumed. */
function drainBody(req) {
  return new Promise((resolve) => {
    req.on("data", () => {});
    req.on("end", resolve);
    req.on("error", resolve);
  });
}

/**
 * Start the mock IM server.
 * @param {{ delayMs?: number, port?: number }} [opts]
 * @returns {Promise<{ port: number, urls: object, close: () => Promise<void> }>}
 */
export function startMockIm(opts = {}) {
  const delayMs = opts.delayMs ?? DEFAULT_DELAY_MS;
  const sockets = new Set();
  const server = http.createServer();

  // ---- HTTP OpenAPIs --------------------------------------------------------
  server.on("request", async (req, res) => {
    await drainBody(req);
    const port = server.address().port;
    const { pathname } = new URL(req.url, "http://127.0.0.1");
    const send = async (obj, { delay = 0, status = 200 } = {}) => {
      if (delay > 0) await sleep(delay);
      const body = JSON.stringify(obj);
      res.writeHead(status, { "content-type": "application/json" });
      res.end(body);
    };
    const wsUrl = (path) => `ws://127.0.0.1:${port}${path}`;

    try {
      // -- Telegram: {base}/bot<token>/<method> --
      if (pathname.includes("/bot")) {
        const method = pathname.split("/").pop();
        switch (method) {
          case "getMe":
            return send({ ok: true, result: { id: 1, is_bot: true, first_name: "mock", username: "mockbot" } });
          case "getUpdates":
            // Throttle the poll loop without a real long hang; never the connect path.
            return send({ ok: true, result: [] }, { delay: 1000 });
          case "sendMessage": // send probe
            return send({ ok: true, result: { message_id: 1, chat: { id: 1 } } }, { delay: delayMs });
          case "answerCallbackQuery":
          case "editMessageReplyMarkup":
            return send({ ok: true, result: true });
          case "editMessageText":
            return send({ ok: true, result: { message_id: 1 } });
          default:
            return send({ ok: true, result: {} });
        }
      }

      // -- DingTalk (success judged by HTTP 2xx) --
      if (pathname.startsWith("/dd-api/")) {
        if (pathname.endsWith("/oauth2/accessToken"))
          return send({ accessToken: "mock-dd-token", expireIn: 7200 });
        if (pathname.endsWith("/gateway/connections/open")) // connect probe
          return send({ endpoint: wsUrl("/dd-ws"), ticket: "mock-ticket" }, { delay: delayMs });
        if (pathname.endsWith("/card/instances/createAndDeliver")) // send probe
          return send({ result: true }, { delay: delayMs });
        return send({ result: true });
      }

      // -- Feishu (success judged by code==0) --
      if (pathname.startsWith("/fs-api/")) {
        if (pathname.endsWith("/tenant_access_token/internal"))
          return send({ code: 0, tenant_access_token: "mock-fs-token", expire: 7200 });
        if (pathname.endsWith("/callback/ws/endpoint")) // connect probe
          return send(
            { code: 0, data: { URL: wsUrl("/fs-ws?service_id=1"), ClientConfig: { PingInterval: 120 } } },
            { delay: delayMs },
          );
        if (pathname.endsWith("/im/v1/messages")) // send probe
          return send({ code: 0, data: { message_id: "mock-msg" } }, { delay: delayMs });
        return send({ code: 0, data: {} });
      }

      // -- Slack (success judged by ok==true) --
      if (pathname.startsWith("/sl-api/")) {
        const method = pathname.split("/").pop();
        switch (method) {
          case "apps.connections.open": // connect probe
            return send({ ok: true, url: wsUrl("/sl-ws") }, { delay: delayMs });
          case "auth.test":
            return send({ ok: true, user_id: "U1", team: "mock", url: "https://mock/" });
          case "conversations.open":
            return send({ ok: true, channel: { id: "D1" } });
          case "chat.postMessage": // send probe
            return send({ ok: true, ts: "1.1", channel: "D1" }, { delay: delayMs });
          case "chat.update":
            return send({ ok: true, ts: "1.1" });
          default:
            return send({ ok: true });
        }
      }

      return send({ ok: true }, { status: 404 });
    } catch {
      try { res.end(); } catch { /* ignore */ }
    }
  });

  // ---- WebSocket endpoints (handshake + keep open; drain input, send nothing) ----
  server.on("upgrade", (req, socket) => {
    const key = req.headers["sec-websocket-key"];
    if (!key) {
      socket.destroy();
      return;
    }
    const accept = crypto.createHash("sha1").update(key + WS_GUID).digest("base64");
    socket.write(
      "HTTP/1.1 101 Switching Protocols\r\n" +
        "Upgrade: websocket\r\n" +
        "Connection: Upgrade\r\n" +
        `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
    );
    sockets.add(socket);
    socket.on("data", () => {}); // drain client frames (e.g. Feishu app-ping); never parsed
    socket.on("error", () => {});
    socket.on("close", () => sockets.delete(socket));
  });

  return new Promise((resolve, reject) => {
    server.on("error", reject);
    server.listen(opts.port ?? 0, "127.0.0.1", () => {
      const port = server.address().port;
      const base = `http://127.0.0.1:${port}`;
      resolve({
        port,
        urls: {
          telegram: `${base}/tg`,
          dingtalk: `${base}/dd-api`,
          slack: `${base}/sl-api`,
          feishu: `${base}/fs-api`,
        },
        close: () =>
          new Promise((done) => {
            for (const s of sockets) {
              try { s.destroy(); } catch { /* ignore */ }
            }
            sockets.clear();
            server.close(() => done());
          }),
      });
    });
  });
}

// Standalone mode: start and stay up until killed (for manual inspection).
if (import.meta.url === `file://${process.argv[1]}`) {
  const args = process.argv.slice(2);
  const get = (flag, def) => {
    const i = args.indexOf(flag);
    return i >= 0 ? args[i + 1] : def;
  };
  const delayMs = parseInt(get("--delay", String(DEFAULT_DELAY_MS)), 10);
  const port = parseInt(get("--port", "0"), 10);
  startMockIm({ delayMs, port }).then((mock) => {
    console.log(`mock IM listening on 127.0.0.1:${mock.port} (delay ${delayMs}ms)`);
    for (const [k, v] of Object.entries(mock.urls)) console.log(`  ${k}: ${v}`);
    process.on("SIGINT", () => mock.close().then(() => process.exit(0)));
    process.on("SIGTERM", () => mock.close().then(() => process.exit(0)));
  });
}
