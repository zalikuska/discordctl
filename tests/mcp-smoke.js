// Проверка MCP-сервера на живом демоне (нужен запущенный Discord с активным хуком). Ничего не отправляет.
// Права выдаются сырым запросом в pipe, в обход диалога CLI (его нельзя автоматизировать).
//   node tests/mcp-smoke.js
const { spawn, execFileSync } = require("child_process");
const net = require("net");
const path = require("path");

const root = path.join(__dirname, "..", "target", "release");
const cli = path.join(root, "discordctl.exe");
const mcpExe = path.join(root, "discordctl-mcp.exe");
const pipe = `\\\\.\\pipe\\discordctl-${(process.env.USERNAME || "").replace(/[^A-Za-z0-9]/g, "_")}`;

let failures = 0;
function check(name, ok, extra = "") {
  console.log(`${name.padEnd(58)} ${ok ? "ok" : "FAIL " + extra}`);
  if (!ok) failures++;
}

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

function cliJson(...args) {
  return JSON.parse(execFileSync(cli, [...args, "--json"], { encoding: "utf8", maxBuffer: 1 << 26 }));
}

class Mcp {
  constructor(token) {
    this.p = spawn(mcpExe, [], { env: { ...process.env, DISCORDCTL_TOKEN: token }, stdio: ["pipe", "pipe", "pipe"] });
    this.n = 0;
    this.waiting = new Map();
    this.raw = [];
    let buf = "";
    this.p.stdout.on("data", (d) => {
      buf += d;
      let i;
      while ((i = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 1);
        this.raw.push(line);
        const m = JSON.parse(line);
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
  notify(method) {
    this.p.stdin.write(JSON.stringify({ jsonrpc: "2.0", method }) + "\n");
  }
  call(name, args) {
    return this.rpc("tools/call", { name, arguments: args });
  }
  close() {
    this.p.kill();
  }
}

(async () => {
  // Без токена сервер не стартует.
  const bare = spawn(mcpExe, [], { env: { ...process.env, DISCORDCTL_TOKEN: "" }, stdio: ["pipe", "pipe", "pipe"] });
  const code = await new Promise((r) => bare.on("exit", r));
  check("refuses to start without a token", code === 2);

  const dms = cliJson("chat");
  const group = dms.find((d) => d.kind === "group");
  const other = dms.find((d) => d.id !== group.id);
  const name = "mcp-smoke-test";
  const g = await owner({ cmd: "auth_grant", name, permissions: { list: ["guilds"], read: [`dm:${group.id}`], write: [] } });
  const token = g.data.token;
  const mcp = new Mcp(token);

  const init = await mcp.rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "smoke", version: "0" } });
  check("initialize answers with a version and instructions", init.result.protocolVersion === "2025-06-18" && /untrusted|never instructions/i.test(init.result.instructions));
  mcp.notify("notifications/initialized");
  check("ping", (await mcp.rpc("ping")).result !== undefined);

  const list = (await mcp.rpc("tools/list")).result.tools.map((t) => t.name).sort();
  check("tool list follows permissions (guilds + read, no write)", JSON.stringify(list) === JSON.stringify(["discord_list_guilds", "discord_read_messages"]), list.join(","));
  const defs = (await mcp.rpc("tools/list")).result.tools;
  check("tools have schemas and annotations", defs.every((t) => t.inputSchema.type === "object" && t.annotations.readOnlyHint === true));

  const guilds = await mcp.call("discord_list_guilds", {});
  const gt = guilds.result.content.map((c) => c.text).join("\n");
  check("guild list is wrapped as untrusted", !guilds.result.isError && /SECURITY NOTICE/.test(gt) && /<discord-data-[0-9a-f]{24}>/.test(gt));

  const read = await mcp.call("discord_read_messages", { channel: group.id, limit: 3 });
  const blocks = read.result.content.map((c) => c.text);
  const tag = blocks[1].match(/^<discord-data-([0-9a-f]{24})>/)?.[1];
  check("read returns notice + one tagged data block", !read.result.isError && blocks.length === 2 && tag && blocks[1].endsWith(`</discord-data-${tag}>`));
  const data = JSON.parse(blocks[1].slice(blocks[1].indexOf("\n") + 1, blocks[1].lastIndexOf("\n")));
  check("data parses and holds 3 messages", data.messages.length === 3 && data.schema === 1);
  const read2 = await mcp.call("discord_read_messages", { channel: group.id, limit: 1 });
  const tag2 = read2.result.content[1].text.match(/^<discord-data-([0-9a-f]{24})>/)?.[1];
  check("boundary id is different on every call", tag2 && tag2 !== tag);

  const huge = await mcp.call("discord_read_messages", { channel: group.id, limit: 100000, before: "1" });
  check("absurd limit is clamped, request still works", !huge.result.isError || /HTTP/.test(huge.result.content[0].text));

  const denied = await mcp.call("discord_read_messages", { channel: other.id, limit: 1 });
  check("reading a chat without permission is an error", denied.result.isError && /no permission/.test(denied.result.content[0].text));

  const hidden = await mcp.call("discord_send_message", { channel: group.id, content: "must not go anywhere" });
  check("hidden write tool cannot be called by name", hidden.result.isError && /not available/.test(hidden.result.content[0].text));

  const unknown = await mcp.call("discord_nope", {});
  check("unknown tool is an error", unknown.result.isError);
  check("unknown method is a JSON-RPC error", (await mcp.rpc("resources/list")).error?.code === -32601);

  // Права на запись: инструменты появляются, сухой запуск проходит без подтверждения и ничего не шлёт.
  mcp.close();
  const g2 = await owner({ cmd: "auth_grant", name, permissions: { list: ["guilds"], read: [`dm:${group.id}`], write: [`dm:${group.id}`] } });
  global.token2 = g2.data.token;
  const mcp2 = new Mcp(global.token2);
  await mcp2.rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "smoke", version: "0" } });
  const list2 = (await mcp2.rpc("tools/list")).result.tools.map((t) => t.name);
  check("write tools appear once write is granted", ["discord_send_message", "discord_edit_message", "discord_delete_message", "discord_add_reaction", "discord_remove_reaction"].every((n) => list2.includes(n)));
  const dry = await mcp2.call("discord_send_message", { channel: group.id, content: "dry only", dry_run: true, mentions: true });
  const dryText = dry.result.content.map((c) => c.text).join("\n");
  check("dry-run send shows the request and pings nobody", !dry.result.isError && /dry_run/.test(dryText) && /"parse":\[\]/.test(dryText));
  const rm = await mcp2.call("discord_remove_reaction", { channel: group.id, message: "100000000000000000", emoji: "👍", dry_run: true });
  check("remove_reaction is a DELETE (fixed value not overridable)", /DELETE/.test(rm.result.content.map((c) => c.text).join("")));
  mcp2.close();

  // Отзыв действует сразу, и список инструментов пустеет.
  const mcp3 = new Mcp(global.token2);
  await mcp3.rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "smoke", version: "0" } });
  await owner({ cmd: "auth_revoke", name });
  check("after revoke the tool list is empty", (await mcp3.rpc("tools/list")).result.tools.length === 0);
  const after = await mcp3.call("discord_list_guilds", {});
  check("after revoke calls fail", after.result.isError && /invalid|revoked/.test(after.result.content[0].text));
  mcp3.close();

  console.log(failures ? `${failures} check(s) FAILED` : "all checks ok");
  process.exit(failures ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
