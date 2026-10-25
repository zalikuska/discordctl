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
    check("list shows exactly what the rights allow", JSON.stringify(await m.names()) === JSON.stringify(["discord_list_guilds", "discord_me", "discord_read_messages", "discord_wait_events"]), (await m.names()).join());
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
    check("full rights show all 24 tools", (await w.names()).length === 24, String((await w.names()).length));
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

    console.log("== events: what happens in Discord reaches the agent, filtered by its rights");
    const A = "111111111111111111", B = "222222222222222222";
    const emit = (events) => owner({ cmd: "dev_emit", events });
    const dmInfo = { kind: "dm", name: "someone" };
    const msg = (channel, text) => ({ kind: "message", ts: Date.now(), channel_id: channel, guild_id: null, channel: dmInfo, message: { content: text }, user: { id: "9", display_name: "someone" } });
    const tl = await grant("listener", { list: ["voices"], read: [`dm:${A}`], write: [] });
    const lm = new Mcp(tl);
    await lm.rpc("initialize", { protocolVersion: "2025-06-18" });
    check("the events tool is offered to an app that can read or see voice", (await lm.names()).includes("discord_wait_events"));
    const first = await lm.call("discord_wait_events", { kinds: ["all"], timeout: 1 });
    check("the first call opens the mailbox and returns empty", !first.result.isError && /"events":\[\]/.test(first.result.content[1].text), JSON.stringify(first.result));
    await emit([
      msg(A, "hello from the allowed chat"),
      msg(B, "SECRET from a chat the app may not read"),
      { kind: "typing", ts: 1, channel_id: A, guild_id: null, channel: dmInfo, user: { id: "9", display_name: "someone" } },
      { kind: "typing", ts: 1, channel_id: B, guild_id: null, channel: dmInfo, user: { id: "9", display_name: "someone" } },
      // Канал сервера без guild_id в событии: чужой серверный чат не должен сойти за личный под правом на `dm:`.
      { kind: "typing", ts: 1, channel_id: A, guild_id: null, channel: { kind: "text", guild_id: "333333333333333333" }, user: { id: "10", display_name: "guild typist" } },
      { kind: "voice_join", ts: 1, channel_id: B, guild_id: "333333333333333333", user: { id: "9", display_name: "someone" } },
    ]);
    const got = await lm.call("discord_wait_events", { kinds: ["all"], timeout: 2 });
    const body = got.result.content.map((c) => c.text).join("\n");
    check("events come inside the untrusted-data fence", /<discord-data-[0-9a-f]+>/.test(body) && /UNTRUSTED/.test(body));
    check("it gets its chat, its typing and voice (the voices word)", /allowed chat/.test(body) && (body.match(/"kind":"typing"/g) || []).length === 1 && /voice_join/.test(body));
    check("it never sees the chat it may not read", !/SECRET/.test(body));
    check("a server channel with no guild id in the event is not taken for a DM", !/guild typist/.test(body));
    const again = await lm.call("discord_wait_events", { kinds: ["messages"], timeout: 1 });
    check("events are delivered once", /"events":\[\]/.test(again.result.content[1].text));
    const t0 = Date.now();
    const waiting = lm.call("discord_wait_events", { kinds: ["messages"], timeout: 20 });
    await sleep(400);
    await emit([msg(A, "second message")]);
    const prompt = await waiting;
    check("a waiting call returns as soon as something arrives", Date.now() - t0 < 5000 && /second message/.test(prompt.result.content[1].text), String(Date.now() - t0));
    const typo = await lm.call("discord_wait_events", { kinds: ["mesages"], timeout: 1 });
    check("a misspelled kind is a clear error", typo.result.isError && /unknown event kind/.test(typo.result.content[0].text));
    lm.close();
    const tb = await grant("blind", { list: ["guilds"], read: [], write: [] });
    const blind = new Mcp(tb);
    await blind.rpc("initialize", { protocolVersion: "2025-06-18" });
    check("an app with nothing to listen to does not see the tool", !(await blind.names()).includes("discord_wait_events"));
    const denied = await blind.call("discord_wait_events", { timeout: 1 });
    check("and cannot call it by name either", denied.result.isError);
    blind.close();
    const viaOwner = await owner({ cmd: "events", kinds: ["voice"], wait_secs: 1, sub: "t" });
    await emit([{ kind: "voice_leave", ts: 1, channel_id: B, guild_id: null, user: { id: "9" } }]);
    const viaOwner2 = await owner({ cmd: "events", kinds: ["voice"], wait_secs: 1, sub: "t" });
    check("the owner can listen too and sees everything", viaOwner.ok && viaOwner2.data.events.length === 1);
    check("a token cannot use the owner's events request", (await new Promise((res, rej) => {
      const s = net.connect(pipe);
      let b = "";
      s.on("connect", () => s.write(JSON.stringify({ token: tl, cmd: "events" }) + "\n"));
      s.on("data", (d) => ((b += d), b.includes("\n") && (s.end(), res(JSON.parse(b.split("\n")[0])))));
      s.on("error", rej);
    })).ok === false);

    console.log("== presence: who is online and what they play, as events with a filter");
    const pres = (user, status, prev, changes) => ({ kind: "presence", ts: Date.now(), channel_id: null, guild_id: null, user: { id: user, display_name: "u" + user }, status, prev_status: prev, changes: changes.map(([change, name]) => ({ change, type: "playing", name })) });
    const text = (r) => r.result.content.map((c) => c.text).join("\n");
    const count = (r) => (text(r).match(/"kind":"presence"/g) || []).length;
    const tp = await grant("watcher", { list: ["presence"], read: [], write: [] });
    const pm = new Mcp(tp);
    await pm.rpc("initialize", { protocolVersion: "2025-06-18" });
    const pnames = await pm.names();
    check("an app with the presence word sees the presence tool and the events tool", pnames.includes("discord_presence") && pnames.includes("discord_wait_events"));
    const game = { kinds: ["presence"], filter: { activity: ["minecraft"] } };
    const opened = await pm.call("discord_wait_events", { ...game, timeout: 1 });
    check("opening a presence listener with a filter works", !opened.result.isError, JSON.stringify(opened.result));
    await emit([
      pres("1", "online", "online", [["started", "Minecraft Launcher"]]),
      pres("2", "online", "online", [["started", "Chess"]]),
      pres("3", "online", "offline", []),
      msg(A, "a message the presence listener must not get"),
    ]);
    const pg = await pm.call("discord_wait_events", { ...game, timeout: 2 });
    check("only the person who started the game arrives", /Minecraft Launcher/.test(text(pg)) && !/Chess/.test(text(pg)) && count(pg) === 1, text(pg).slice(0, 300));
    check("no messages come to a presence-only listener", !/must not get/.test(text(pg)));
    const entering = { kinds: ["presence"], filter: { status: ["online"] } };
    await pm.call("discord_wait_events", { ...entering, timeout: 1 });
    await emit([pres("3", "online", "offline", []), pres("4", "online", "online", [["started", "Chess"]])]);
    const pg2 = await pm.call("discord_wait_events", { ...entering, timeout: 2 });
    check("a status filter means ENTERING the status", count(pg2) === 1 && /"id":"3"/.test(text(pg2)), text(pg2).slice(0, 300));
    const badFilter = await pm.call("discord_wait_events", { kinds: ["presence"], filter: { status: ["away"] }, timeout: 1 });
    check("a filter with a wrong status is a clear error", badFilter.result.isError && /not a status/.test(badFilter.result.content[0].text));
    pm.close();
    const rd = new Mcp(await grant("reader2", { list: ["guilds"], read: ["*"], write: [] }));
    await rd.rpc("initialize", { protocolVersion: "2025-06-18" });
    const noWord = await rd.call("discord_wait_events", { kinds: ["presence"], timeout: 1 });
    check("reading chats does not give presence", noWord.result.isError && /no permission/.test(noWord.result.content[0].text));
    const dflt = await rd.call("discord_wait_events", { timeout: 1 });
    check("and `all` never includes presence", !dflt.result.isError && !text(dflt).includes('"presence"'), text(dflt).slice(0, 300));
    rd.close();

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
