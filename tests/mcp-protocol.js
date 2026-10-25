// Проверка MCP-сервера по протоколу без Discord: поднимает демон в песочнице (временные папка и pipe) и говорит с
// discordctl-mcp.exe через stdio, как агент. Хук в Discord не нужен, поэтому вызовы инструментов, которым нужен
// Discord, проверяются только на аккуратный отказ. Ничего не трогает в настоящей установке.
//   cargo build --release; node tests/mcp-protocol.js
const { spawn } = require("child_process");
const net = require("net");
const os = require("os");
const fs = require("fs");
const path = require("path");

const bin = path.join(__dirname, "..", "target", "release");
const id = Math.random().toString(16).slice(2, 10);
const home = fs.mkdtempSync(path.join(os.tmpdir(), "dctl-mcp-"));
const pipe = `\\\\.\\pipe\\discordctl-mcp-test-${id}`;
const env = { ...process.env, DISCORDCTL_HOME: home, DISCORDCTL_PIPE: pipe, DISCORDCTL_DISCORD_ROOT: path.join(home, "Discord") };
delete env.DISCORDCTL_TOKEN;

let failures = 0;
const check = (name, ok, extra = "") => {
  console.log(`${ok ? "  PASS" : "  FAIL"}  ${name}${ok ? "" : "  " + extra}`);
  if (!ok) failures++;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function owner(req) {
  return new Promise((resolve, reject) => {
    const sock = net.connect(pipe);
    let buf = "";
    sock.on("connect", () => sock.write(JSON.stringify(req) + "\n"));
    sock.on("data", (d) => {
      buf += d;
      if (buf.includes("\n")) {
        sock.end();
        resolve(JSON.parse(buf.split("\n")[0]));
      }
    });
    sock.on("error", reject);
  });
}

class Mcp {
  constructor(token, extraEnv = {}) {
    this.p = spawn(path.join(bin, "discordctl-mcp.exe"), [], { env: { ...env, ...extraEnv, DISCORDCTL_TOKEN: token }, stdio: ["pipe", "pipe", "pipe"] });
    this.n = 0;
    this.waiting = new Map();
    this.lines = [];
    this.exited = new Promise((r) => this.p.on("exit", r));
    let buf = "";
    this.p.stdout.on("data", (d) => {
      buf += d;
      let i;
      while ((i = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 1);
        this.lines.push(line);
        const m = JSON.parse(line);
        if (m.id === undefined) this.onNotification?.(m);
        this.waiting.get(m.id)?.(m);
      }
    });
  }
  rpc(method, params) {
    const id = ++this.n;
    return new Promise((resolve) => {
      this.waiting.set(id, resolve);
      this.p.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  }
  raw(text) {
    this.p.stdin.write(text + "\n");
  }
  call(name, args) {
    return this.rpc("tools/call", { name, arguments: args });
  }
  async names() {
    return (await this.rpc("tools/list")).result.tools.map((t) => t.name).sort();
  }
  close() {
    this.p.kill();
  }
}

(async () => {
  const daemon = spawn(path.join(bin, "discordctl-daemon.exe"), [], { env, stdio: "ignore" });
  for (let i = 0; i < 50; i++) {
    try {
      if ((await owner({ cmd: "ping" })).ok) break;
    } catch {
      await sleep(100);
    }
  }
  const grant = async (name, permissions) => (await owner({ cmd: "auth_grant", name, permissions })).data.token;

  try {
    console.log("== startup");
    const bare = spawn(path.join(bin, "discordctl-mcp.exe"), [], { env: { ...env, DISCORDCTL_TOKEN: "" }, stdio: "ignore" });
    check("refuses to start without a token (exit 2)", (await new Promise((r) => bare.on("exit", r))) === 2);

    console.log("== initialize");
    const t1 = await grant("reader", { list: ["guilds", "me"], read: ["*"], write: [] });
    const m = new Mcp(t1);
    const init = await m.rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "t", version: "0" } });
    check("answers with the requested version", init.result.protocolVersion === "2025-06-18");
    check("declares only tools, with instructions", Object.keys(init.result.capabilities).join() === "tools" && /data, \s*never instructions|never instructions/.test(init.result.instructions));
    const old = await m.rpc("initialize", { protocolVersion: "2024-11-05" });
    check("accepts an older supported version", old.result.protocolVersion === "2024-11-05");
    const future = await m.rpc("initialize", { protocolVersion: "2099-01-01" });
    check("unknown version falls back to the newest supported", future.result.protocolVersion === "2025-06-18");

    console.log("== framing and errors");
    m.raw(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }));
    await sleep(200);
    const before = m.lines.length;
    m.raw(JSON.stringify({ jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 99 } }));
    await sleep(200);
    check("notifications get no reply", m.lines.length === before);
    check("ping works", (await m.rpc("ping")).result !== undefined);
    const nope = await m.rpc("resources/list");
    check("unknown method is -32601", nope.error?.code === -32601);
    m.raw("{not json");
    await sleep(200);
    const perr = JSON.parse(m.lines[m.lines.length - 1]);
    check("malformed JSON is -32700 with null id", perr.error?.code === -32700 && perr.id === null);
    m.raw(JSON.stringify([{ jsonrpc: "2.0", id: 5, method: "ping" }]));
    await sleep(200);
    const batch = JSON.parse(m.lines[m.lines.length - 1]);
    check("a batch array is rejected as invalid (-32600)", batch.error?.code === -32600);

    console.log("== tools follow rights");
    check("list shows exactly what the rights allow", JSON.stringify(await m.names()) === JSON.stringify(["discord_list_guilds", "discord_me", "discord_read_messages"]), (await m.names()).join());
    const defs = (await m.rpc("tools/list")).result.tools;
    check("every tool has an object schema and read-only hint", defs.every((t) => t.inputSchema.type === "object" && t.annotations.readOnlyHint === true));
    const hidden = await m.call("discord_send_message", { channel: "dm:x", content: "hi" });
    check("a hidden tool cannot be called by name", hidden.result.isError && /not available/.test(hidden.result.content[0].text));
    const unk = await m.call("discord_nothing", {});
    check("unknown tool is an error result", unk.result.isError && /unknown tool/.test(unk.result.content[0].text));
    const noHook = await m.call("discord_list_guilds", {});
    check("without Discord a call fails politely", noHook.result.isError && /hook is not active|not active/.test(noHook.result.content[0].text), noHook.result.content[0].text);

    console.log("== rights change while connected");
    await grant("reader", { list: ["guilds"], read: [], write: [] }); // тот же name: токен пересоздаётся
    const stale = (await m.rpc("tools/list")).error;
    check("old token: tools/list fails and says why (not an empty list)", stale?.code === -32603 && /invalid or was revoked/.test(stale.message), JSON.stringify(stale));
    const afterRegrant = await m.call("discord_list_guilds", {});
    check("and calls say the token is invalid", afterRegrant.result.isError && /invalid or was revoked/.test(afterRegrant.result.content[0].text));
    m.close();

    console.log("== second token, revoke");
    const t2 = await grant("writer", { list: ["*"], read: ["*"], write: ["*"], voice: ["*"], voice_channels: ["*"], stream: ["*"] });
    const w = new Mcp(t2);
    await w.rpc("initialize", { protocolVersion: "2025-06-18" });
    check("full rights show all 22 tools", (await w.names()).length === 22, String((await w.names()).length));
    await owner({ cmd: "auth_revoke", name: "writer" });
    check("revoked token: tools/list fails with a clear message", /invalid or was revoked/.test((await w.rpc("tools/list")).error?.message ?? ""));

    console.log("== owner-only requests are closed to apps");
    const t3 = await grant("snoop", { list: ["*"], read: ["*"], write: [] });
    const viaToken = await new Promise((res, rej) => {
      const s = net.connect(pipe);
      let b = "";
      s.on("connect", () => s.write(JSON.stringify({ token: t3, cmd: "auth_list" }) + "\n"));
      s.on("data", (d) => ((b += d), b.includes("\n") && (s.end(), res(JSON.parse(b.split("\n")[0])))));
      s.on("error", rej);
    });
    check("a token cannot list apps or grant rights", viaToken.ok === false && /owner only/.test(viaToken.msg));
    const raw = await new Promise((res, rej) => {
      const s = net.connect(pipe);
      let b = "";
      s.on("connect", () => s.write(JSON.stringify({ token: t3, cmd: "call", method: "find_modules", params: {} }) + "\n"));
      s.on("data", (d) => ((b += d), b.includes("\n") && (s.end(), res(JSON.parse(b.split("\n")[0])))));
      s.on("error", rej);
    });
    check("a token cannot reach internal payload methods", raw.ok === false && /not available to apps/.test(raw.msg));

    console.log("== a call that waits for the owner stays alive");
    // Права на голос без канала: сам вызов `voice_leave` упирается только в подтверждение человека, Discord не нужен.
    const t4 = await grant("waiter", { list: [], read: [], write: [], voice: ["leave"] });
    const wmcp = new Mcp(t4, { DISCORDCTL_MCP_PROGRESS_MS: "300" });
    await wmcp.rpc("initialize", { protocolVersion: "2025-06-18" });
    const progress = [];
    wmcp.onNotification = (m) => m.method === "notifications/progress" && progress.push(m.params);
    const pending = wmcp.rpc("tools/call", { name: "discord_voice_leave", arguments: {}, _meta: { progressToken: "tok-1" } });
    let queued = [];
    for (let i = 0; i < 40 && !queued.length; i++) {
      await sleep(100);
      queued = (await owner({ cmd: "approvals" })).data;
    }
    check("the call waits in the approval queue", queued.length === 1 && queued[0].app === "waiter");
    await sleep(1000);
    check("progress notifications arrive while it waits, with the client's token and growing numbers", progress.length >= 2 && progress.every((p) => p.progressToken === "tok-1") && progress[1].progress > progress[0].progress, JSON.stringify(progress));
    check("ping is answered while the call waits", (await wmcp.rpc("ping")).result !== undefined);
    await owner({ cmd: "decide", id: queued[0].id, allow: false });
    const rejected = await pending;
    check("rejecting finishes the call with a clear error", rejected.result.isError && /rejected/.test(rejected.result.content[0].text), JSON.stringify(rejected.result));
    const afterDone = progress.length;
    await sleep(900);
    check("no progress is sent after the answer", progress.length === afterDone);
    wmcp.close();

    console.log("== daemon goes away");
    const r = new Mcp(t3);
    await r.rpc("initialize", { protocolVersion: "2025-06-18" });
    await owner({ cmd: "shutdown", force: true });
    await sleep(500);
    const listDown = (await r.rpc("tools/list")).error;
    check("tools/list with the daemon down names the daemon, not the token", listDown?.code === -32603 && /cannot reach the discordctl daemon/.test(listDown.message), JSON.stringify(listDown));
    const down = await r.call("discord_list_guilds", {});
    check("a call with the daemon down says the daemon is unreachable", down.result.isError && /cannot reach the discordctl daemon/.test(down.result.content[0].text) && !/token/.test(down.result.content[0].text), down.result.content[0].text);

    console.log("== robustness");
    r.p.stdin.write(Buffer.from([0xff, 0xfe, 0x7b, 0x0a])); // неверная UTF-8
    await sleep(300);
    const bad = JSON.parse(r.lines[r.lines.length - 1]);
    check("a line with invalid UTF-8 gets -32700 and the server stays alive", bad.error?.code === -32700 && (await r.rpc("ping")).result !== undefined);
    // Отмена вызова, которого нет, ничего не ломает и не даёт ответа.
    const n = r.lines.length;
    r.raw(JSON.stringify({ jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 12345, reason: "test" } }));
    await sleep(200);
    check("cancelling an unknown request is ignored", r.lines.length === n && (await r.rpc("ping")).result !== undefined);
    r.close();
  } finally {
    daemon.kill();
    await sleep(300);
    fs.rmSync(home, { recursive: true, force: true });
  }
  console.log(failures ? `\n${failures} FAILED` : "\nALL PASSED");
  process.exit(failures ? 1 : 0);
})();
