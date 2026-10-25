"use strict";
(() => {
  // src/webpack.ts
  var GLOBAL = "webpackChunkdiscord_app";
  var runtimes = [];
  var pushReplacements = 0;
  function onRuntime(req) {
    if (req?.c && !runtimes.includes(req)) runtimes.push(req);
  }
  function hookArray(arr) {
    if (!Array.isArray(arr)) return;
    let current = arr.push;
    Object.defineProperty(arr, "push", {
      configurable: true,
      get: () => current,
      set(fn) {
        current = fn;
        pushReplacements++;
        try {
          fn.call(arr, [[Symbol("discordctl")], {}, onRuntime]);
        } catch {
        }
      }
    });
  }
  function installWebpackHook() {
    let chunk;
    Object.defineProperty(window, GLOBAL, {
      configurable: true,
      get: () => chunk,
      set(v) {
        chunk = v;
        hookArray(v);
      }
    });
  }
  function mainRuntime() {
    let best = null;
    for (const req of runtimes) {
      const modules = Object.keys(req.c).length;
      if (!best || modules > best.modules) best = { req, modules };
    }
    return best;
  }
  function webpackInfo() {
    return { runtimes: runtimes.length, push_replacements: pushReplacements };
  }

  // src/stores.ts
  var REQUIRED = {
    UserStore: ["getCurrentUser", "getUser"],
    GuildStore: ["getGuild", "getGuilds"],
    ChannelStore: ["getChannel", "getSortedPrivateChannels"],
    MessageStore: ["getMessages", "getMessage"],
    SelectedChannelStore: ["getChannelId"],
    RelationshipStore: ["getFriendIDs", "getRelationshipType"]
  };
  var cache = { modules: -1, stores: /* @__PURE__ */ new Map() };
  function isStore(c) {
    if (!c || typeof c !== "object") return false;
    const s = c;
    return typeof s.getName === "function" && typeof s.addChangeListener === "function";
  }
  function allStores() {
    const main = mainRuntime();
    if (!main) return cache.stores;
    if (cache.modules === main.modules) return cache.stores;
    const stores = /* @__PURE__ */ new Map();
    for (const id in main.req.c) {
      let ex;
      try {
        ex = main.req.c[id]?.exports;
      } catch {
        continue;
      }
      if (!ex || typeof ex !== "object" && typeof ex !== "function") continue;
      const candidates2 = [ex];
      try {
        for (const k of Object.keys(ex)) candidates2.push(ex[k]);
      } catch {
      }
      for (const c of candidates2) {
        try {
          if (!isStore(c)) continue;
          const name = c.getName();
          if (typeof name === "string" && !stores.has(name)) stores.set(name, c);
        } catch {
        }
      }
    }
    cache = { modules: main.modules, stores };
    return stores;
  }
  function getStore(name) {
    const s = allStores().get(name);
    if (!s) throw new Error(`store ${name} not found (Discord internals changed?)`);
    return s;
  }
  function missingStores() {
    const found = allStores();
    return Object.keys(REQUIRED).filter((n) => !found.has(n));
  }

  // src/directory.ts
  var CHANNEL_TYPES = {
    0: "text",
    1: "dm",
    2: "voice",
    3: "group",
    4: "category",
    5: "announcement",
    10: "thread",
    11: "thread",
    12: "thread",
    13: "stage",
    14: "directory",
    15: "forum",
    16: "media"
  };
  var call = (store3, method, ...args) => {
    const fn = getStore(store3)[method];
    if (typeof fn !== "function") throw new Error(`${store3}.${method} is missing (Discord internals changed?)`);
    return fn.apply(getStore(store3), args);
  };
  var isId = (s) => /^\d{15,25}$/.test(s);
  var norm = (s) => s.trim().toLowerCase();
  function userName(u) {
    return u ? String(u.globalName ?? u.global_name ?? u.username ?? u.id) : "?";
  }
  function userBrief(u) {
    return u ? { id: String(u.id), username: String(u.username), display_name: userName(u), bot: !!u.bot } : null;
  }
  function friendNick(userId) {
    try {
      const n = call("RelationshipStore", "getNickname", userId);
      return typeof n === "string" && n ? n : null;
    } catch {
      return null;
    }
  }
  function recipientsOf(ch) {
    const ids = ch.recipients ?? [];
    return ids.map((id) => call("UserStore", "getUser", id)).filter(Boolean);
  }
  function dmTitle(ch) {
    if (ch.name) return String(ch.name);
    return recipientsOf(ch).map((u) => friendNick(String(u.id)) ?? userName(u)).join(", ");
  }
  function dmBrief(ch) {
    return {
      id: String(ch.id),
      kind: CHANNEL_TYPES[ch.type] ?? String(ch.type),
      name: dmTitle(ch),
      recipients: recipientsOf(ch).map((u) => userBrief(u)),
      last_message_id: ch.lastMessageId ?? null
    };
  }
  function dms() {
    const list = call("ChannelStore", "getSortedPrivateChannels");
    return list.map(dmBrief);
  }
  function guilds() {
    const all = Object.values(call("GuildStore", "getGuilds"));
    const me3 = call("UserStore", "getCurrentUser");
    return all.map((g) => ({ id: String(g.id), name: String(g.name), owner: g.ownerId === me3?.id, member_count: g.memberCount ?? null })).sort((a, b) => a.name.localeCompare(b.name));
  }
  function friends() {
    const ids = call("RelationshipStore", "getFriendIDs");
    return ids.map((id) => {
      const u = call("UserStore", "getUser", id);
      return u ? { ...userBrief(u), nickname: friendNick(id) } : { id, username: null, display_name: null, bot: false, nickname: null };
    }).sort((a, b) => String(a.display_name).localeCompare(String(b.display_name)));
  }
  function pick(items, query, tiers, what) {
    const q = norm(query);
    const byId = items.find((t) => t.id === query);
    if (byId) return byId;
    for (const test of [(n) => n === q, (n) => n.includes(q)]) {
      for (const names of tiers) {
        const hit = items.filter((t) => names(t).some((n) => n && test(norm(n))));
        if (hit.length === 1) return hit[0];
        if (hit.length > 1) {
          throw new Error(`${what} "${query}" is ambiguous: ${hit.slice(0, 8).map((t) => `${tiers[0](t)[0]} (${t.id})`).join(", ")}`);
        }
      }
    }
    throw new Error(`${what} "${query}" not found`);
  }
  function resolveGuild(query) {
    const list = Object.values(call("GuildStore", "getGuilds"));
    return pick(list.map((g) => ({ ...g, id: String(g.id) })), query, [(g) => [g.name]], "guild");
  }
  function guildChannels(guildId) {
    const map = call("ChannelStore", "getMutableGuildChannelsForGuild", guildId);
    return Object.values(map).sort((a, b) => (a.position ?? 0) - (b.position ?? 0));
  }
  var MEMBERS_DEFAULT = 100;
  var MEMBERS_MAX = 500;
  function members(p) {
    const g = resolveGuild(p.guild);
    const gid = String(g.id);
    const raw = call("GuildMemberStore", "getMembers", gid) ?? [];
    let total = null;
    try {
      const n = call("GuildMemberCountStore", "getMemberCount", gid);
      total = typeof n === "number" ? n : null;
    } catch {
    }
    let roleNames = {};
    try {
      const roles = call("GuildStore", "getRoles", gid) ?? {};
      roleNames = Object.fromEntries(Object.entries(roles).map(([id, r]) => [id, String(r.name ?? id)]));
    } catch {
    }
    const q = norm(p.query ?? "");
    const all = raw.map((m) => {
      const u = call("UserStore", "getUser", m.userId);
      const brief = userBrief(u) ?? { id: String(m.userId), username: null, display_name: null, bot: false };
      const roles = (m.roles ?? []).map((id) => roleNames[id] ?? id);
      return { ...brief, nickname: m.nick ?? null, roles: roles.slice(0, 10), joined_at: m.joinedAt ?? null };
    }).filter((m) => !q || [m.display_name, m.username, m.nickname].some((n) => n && norm(String(n)).includes(q))).sort((a, b) => String(a.nickname ?? a.display_name).localeCompare(String(b.nickname ?? b.display_name)));
    const limit = Math.min(Math.max(1, Math.floor(p.limit ?? MEMBERS_DEFAULT) || MEMBERS_DEFAULT), MEMBERS_MAX);
    return {
      guild: { id: gid, name: String(g.name) },
      loaded: raw.length,
      total_members: total,
      matched: all.length,
      truncated: all.length > limit,
      members: all.slice(0, limit)
    };
  }
  function channels(guildQuery) {
    const g = resolveGuild(guildQuery);
    const all = guildChannels(String(g.id));
    const catName = (id) => id ? all.find((c) => c.id === id)?.name ?? null : null;
    return all.filter((c) => c.type !== 4).map((c) => ({
      id: String(c.id),
      name: String(c.name),
      kind: CHANNEL_TYPES[c.type] ?? String(c.type),
      category: catName(c.parent_id),
      topic: c.topic ?? null
    }));
  }
  function resolveChannel(ref, opts = {}) {
    const r = ref.trim();
    if (!r) throw new Error("empty channel reference");
    const dmMatch = /^(?:(?:dm|group):|@)(.+)$/i.exec(r);
    if (dmMatch) return resolveDm(dmMatch[1]);
    const gm = /^(?:guild:)?(.+?)\/(?:channel:)?(.+)$/i.exec(r);
    if (gm) {
      const g = resolveGuild(gm[1]);
      const list = guildChannels(String(g.id)).filter((c) => opts.voice ? c.type === 2 || c.type === 13 : c.type !== 4).map((c) => ({ ...c, id: String(c.id) }));
      return pick(list, gm[2], [(c) => [c.name]], "channel");
    }
    if (isId(r)) {
      const ch = call("ChannelStore", "getChannel", r);
      if (ch) return ch;
      const dmId = call("ChannelStore", "getDMFromUserId", r);
      if (dmId) return call("ChannelStore", "getChannel", dmId);
      throw new Error(`no channel or direct chat with id ${r} is known to the client`);
    }
    return resolveDm(r);
  }
  var memberNames = (c) => recipientsOf(c).flatMap((u) => [userName(u), String(u.username), friendNick(String(u.id))]);
  function resolveDm(query) {
    const list = call("ChannelStore", "getSortedPrivateChannels");
    const items = list.map((c) => ({ ...c, id: String(c.id) }));
    const q = query.trim();
    if (isId(q)) {
      const byUser = call("ChannelStore", "getDMFromUserId", q);
      if (byUser) return call("ChannelStore", "getChannel", byUser);
    }
    return pick(
      items,
      q,
      [
        (c) => [dmTitle(c)],
        // 1:1 чат важнее группы, где человек просто участник.
        (c) => c.type === 1 ? memberNames(c) : [],
        (c) => c.type === 3 ? memberNames(c) : []
      ],
      "direct chat"
    );
  }
  function describeChannel(ch) {
    const kind = CHANNEL_TYPES[ch.type] ?? String(ch.type);
    return {
      id: String(ch.id),
      kind,
      name: ch.type === 1 || ch.type === 3 ? dmTitle(ch) : String(ch.name ?? ""),
      guild_id: ch.guild_id ?? null
    };
  }

  // src/http.ts
  var MIN_GAP_MS = 350;
  var JITTER_MS = 300;
  var MAX_RETRIES_429 = 3;
  function withTimeout(p, ms) {
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`timeout ${ms} ms`)), ms);
      p.then(
        (v) => (clearTimeout(t), resolve(v)),
        (e) => (clearTimeout(t), reject(e))
      );
    });
  }
  var rest = null;
  var probeLog = [];
  function candidates() {
    const main = mainRuntime();
    const found = /* @__PURE__ */ new Set();
    if (!main) return [];
    const isRest = (o) => {
      if (!o || typeof o !== "object" && typeof o !== "function") return false;
      try {
        const r = o;
        if (!["get", "post", "patch", "put"].every((k) => typeof r[k] === "function")) return false;
        if (typeof r.del !== "function" && typeof r.delete !== "function") return false;
        const src = String(r.get);
        return src !== "r=>e.get(t,r)" && !src.includes(".end(");
      } catch {
        return false;
      }
    };
    for (const id in main.req.c) {
      let ex;
      try {
        ex = main.req.c[id]?.exports;
      } catch {
        continue;
      }
      if (!ex || typeof ex !== "object" && typeof ex !== "function") continue;
      if (isRest(ex)) found.add(ex);
      try {
        for (const k of Object.keys(ex)) {
          const v = ex[k];
          if (isRest(v)) found.add(v);
        }
      } catch {
      }
    }
    return [...found];
  }
  async function locate(expectedUserId) {
    if (rest) return rest;
    probeLog = [];
    const list = candidates();
    for (const [i, c] of list.entries()) {
      try {
        const res = await withTimeout(c.get({ url: "/users/@me" }), 5e3);
        const id = res?.body?.id;
        if (res?.status === 200 && id === expectedUserId) {
          rest = c;
          return c;
        }
        probeLog.push(`#${i}: status ${res?.status}, id ${id}`);
      } catch (e) {
        probeLog.push(`#${i}: ${e instanceof Error ? e.message : JSON.stringify(e)}`);
      }
    }
    throw new Error(`HTTP module not found among ${list.length} candidates (${probeLog.join("; ")})`);
  }
  function describeCandidates() {
    const main = mainRuntime();
    const loose = [];
    for (const id in main?.req.c ?? {}) {
      let ex;
      try {
        ex = main.req.c[id]?.exports;
      } catch {
        continue;
      }
      if (!ex || typeof ex !== "object" && typeof ex !== "function") continue;
      const objs = [["", ex]];
      try {
        for (const k of Object.keys(ex)) objs.push([k, ex[k]]);
      } catch {
      }
      for (const [k, o] of objs) {
        try {
          if (!o || typeof o.get !== "function" || typeof o.post !== "function" || typeof o.patch !== "function") continue;
          const src = String(o.get);
          if (src === "r=>e.get(t,r)") continue;
          loose.push({ id, key: k, has_del: typeof o.del, has_delete: typeof o.delete, base_fn: typeof o.getAPIBaseURL, get_src: src.slice(0, 200) });
        } catch {
        }
      }
    }
    return loose;
  }
  function httpInfo() {
    return { located: rest !== null, distinct_candidates: candidates().length, probe_log: probeLog, writes: writeBudget() };
  }
  var queue = Promise.resolve();
  var last = 0;
  var sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  var WRITE_GAP_MS = 1200;
  var WRITE_JITTER_MS = 900;
  var MAX_WRITES_PER_MIN = 15;
  var writeTimes = [];
  function reserveWrite() {
    const now = Date.now();
    while (writeTimes.length && now - writeTimes[0] > 6e4) writeTimes.shift();
    if (writeTimes.length >= MAX_WRITES_PER_MIN) {
      const wait = Math.ceil((6e4 - (now - writeTimes[0])) / 1e3);
      throw new Error(`write rate limit: ${MAX_WRITES_PER_MIN} actions per minute, retry in ${wait} s`);
    }
    writeTimes.push(now);
  }
  function writeBudget() {
    const now = Date.now();
    return { limit_per_minute: MAX_WRITES_PER_MIN, used: writeTimes.filter((t) => now - t <= 6e4).length };
  }
  function apiRequest(expectedUserId, method, url, opts = {}) {
    const isWrite = method !== "get";
    if (isWrite) {
      try {
        reserveWrite();
      } catch (e) {
        return Promise.reject(e);
      }
    }
    const run2 = async () => {
      const gap = isWrite ? WRITE_GAP_MS + Math.random() * WRITE_JITTER_MS : MIN_GAP_MS + Math.random() * JITTER_MS;
      const wait = last + gap - Date.now();
      if (wait > 0) await sleep(wait);
      const mod = await locate(expectedUserId);
      const m = mod;
      const fn = method === "delete" ? m.del ?? m.delete : m[method];
      if (typeof fn !== "function") throw new Error(`HTTP module has no ${method}`);
      for (let attempt = 0; ; attempt++) {
        try {
          const res = await withTimeout(fn.call(mod, { url, query: opts.query, body: opts.body }), 3e4);
          last = Date.now();
          return res.body;
        } catch (e) {
          last = Date.now();
          const r = e;
          if (r?.status === 429 && attempt < MAX_RETRIES_429) {
            await sleep(Math.ceil((r.body?.retry_after ?? 1) * 1e3) + 250);
            continue;
          }
          if (r && typeof r.status === "number") throw new Error(`HTTP ${r.status}: ${r.body?.message ?? "request failed"}`);
          throw e;
        }
      }
    };
    const p = queue.then(run2, run2);
    queue = p.catch(() => void 0);
    return p;
  }
  var apiGet = (expectedUserId, url, query) => apiRequest(expectedUserId, "get", url, { query });

  // src/messages.ts
  var PAGE = 100;
  var MAX_TOTAL = 2e3;
  function author(a) {
    if (!a) return null;
    return userBrief({ id: a.id, username: a.username, global_name: a.global_name, bot: a.bot });
  }
  function toMessage(m) {
    const ref = m.message_reference;
    const replied = m.referenced_message;
    return {
      id: String(m.id),
      channel_id: String(m.channel_id),
      author: author(m.author),
      content: String(m.content ?? ""),
      timestamp: m.timestamp ?? null,
      edited_timestamp: m.edited_timestamp ?? null,
      type: m.type ?? 0,
      pinned: !!m.pinned,
      reply_to: ref?.message_id ? {
        message_id: String(ref.message_id),
        author: replied ? author(replied.author) : null,
        content_preview: replied ? String(replied.content ?? "").slice(0, 200) : null
      } : null,
      attachments: (m.attachments ?? []).map((a) => ({
        id: String(a.id),
        filename: String(a.filename),
        size: a.size ?? null,
        content_type: a.content_type ?? null,
        url: a.url ?? null
      })),
      embeds: (m.embeds ?? []).map((e) => ({
        type: e.type ?? null,
        title: e.title ?? null,
        description: e.description ?? null,
        url: e.url ?? null
      })),
      reactions: (m.reactions ?? []).map((r) => ({
        emoji: r.emoji?.id ? `${r.emoji.name}:${r.emoji.id}` : String(r.emoji?.name ?? ""),
        count: r.count ?? 0,
        me: !!r.me
      })),
      mentions: (m.mentions ?? []).map((u) => String(u.id)),
      stickers: (m.sticker_items ?? []).map((s) => ({ id: String(s.id), name: String(s.name) }))
    };
  }
  async function readMessages(p) {
    const ch = resolveChannel(p.channel);
    const me3 = getStore("UserStore").getCurrentUser?.();
    if (!me3) throw new Error("not logged in yet");
    const want = Math.min(Math.max(1, Math.floor(p.limit ?? 50)), MAX_TOTAL);
    const forward = !!p.after;
    let cursor = forward ? p.after : p.before;
    const got = [];
    let hasMore = true;
    while (got.length < want && hasMore) {
      const limit = Math.min(PAGE, want - got.length);
      const query = { limit };
      if (cursor) query[forward ? "after" : "before"] = cursor;
      const page = await apiGet(String(me3.id), `/channels/${ch.id}/messages`, query);
      if (!Array.isArray(page)) throw new Error("unexpected response from the messages endpoint");
      if (page.length < limit) hasMore = false;
      if (!page.length) break;
      got.push(...page);
      const ids = page.map((m) => BigInt(m.id));
      const edge = forward ? ids.reduce((a, b) => a > b ? a : b) : ids.reduce((a, b) => a < b ? a : b);
      cursor = String(edge);
    }
    got.sort((a, b) => BigInt(a.id) < BigInt(b.id) ? -1 : 1);
    return {
      schema: 1,
      channel: describeChannel(ch),
      direction: forward ? "after" : "before",
      messages: got.map(toMessage),
      has_more: hasMore,
      next: got.length ? cursor : null
    };
  }

  // src/modules.ts
  var cache2 = /* @__PURE__ */ new Map();
  var isProxy = (o) => {
    try {
      return typeof o["__discordctl_probe__"] === "function";
    } catch {
      return false;
    }
  };
  function findByFns(names) {
    const main = mainRuntime();
    if (!main) return null;
    const key = names.join("+");
    const hit = cache2.get(key);
    if (hit && (hit.found || hit.modules === main.modules)) return hit.found;
    const has = (o) => {
      if (!o || typeof o !== "object" && typeof o !== "function" || isProxy(o)) return false;
      try {
        return names.every((n) => typeof o[n] === "function");
      } catch {
        return false;
      }
    };
    let found = null;
    for (const id in main.req.c) {
      let ex;
      try {
        ex = main.req.c[id]?.exports;
      } catch {
        continue;
      }
      if (!ex || typeof ex !== "object" && typeof ex !== "function") continue;
      if (has(ex)) {
        found = ex;
        break;
      }
      try {
        for (const k of Object.keys(ex)) {
          const v = ex[k];
          if (has(v)) {
            found = v;
            break;
          }
        }
      } catch {
      }
      if (found) break;
    }
    cache2.set(key, { modules: main.modules, found });
    return found;
  }
  function requireFns(names) {
    const f = findByFns(names);
    if (!f) throw new Error(`client action module with ${names.join(", ")} not found (Discord internals changed?)`);
    return f;
  }
  var srcCache = /* @__PURE__ */ new Map();
  function findFnBySource(markers) {
    const main = mainRuntime();
    if (!main) return null;
    const key = markers.join("\0");
    const hit = srcCache.get(key);
    if (hit && (hit.fn || hit.modules === main.modules)) return hit.fn;
    let found = null;
    const scan = (o) => {
      if (found || !o || typeof o !== "object" && typeof o !== "function" || isProxy(o)) return;
      let keys;
      try {
        keys = Object.keys(o);
      } catch {
        return;
      }
      for (const k of keys) {
        try {
          const v = o[k];
          if (typeof v !== "function") continue;
          const src = String(v);
          if (markers.every((m) => src.includes(m))) {
            found = v;
            return;
          }
        } catch {
        }
      }
    };
    for (const id in main.req.c) {
      let ex;
      try {
        ex = main.req.c[id]?.exports;
      } catch {
        continue;
      }
      scan(ex);
      if (found) break;
      try {
        for (const k of Object.keys(ex ?? {})) {
          scan(ex[k]);
          if (found) break;
        }
      } catch {
      }
      if (found) break;
    }
    srcCache.set(key, { modules: main.modules, fn: found });
    return found;
  }

  // src/voice.ts
  var sleep2 = (ms) => new Promise((r) => setTimeout(r, ms));
  var store = (name, method, ...args) => {
    const s = getStore(name);
    const fn = s[method];
    if (typeof fn !== "function") throw new Error(`${name}.${method} is missing (Discord internals changed?)`);
    return fn.apply(s, args);
  };
  async function until(cond, ms) {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      try {
        if (cond()) return true;
      } catch {
      }
      await sleep2(250);
    }
    try {
      return cond();
    } catch {
      return false;
    }
  }
  var VOICE_KINDS = /* @__PURE__ */ new Set([1, 2, 3, 13]);
  function participants(channelId) {
    const states = store("VoiceStateStore", "getVoiceStatesForChannel", channelId) ?? {};
    return Object.values(states).map((v) => {
      const u = store("UserStore", "getUser", v.userId);
      return {
        user: userBrief(u) ?? { id: String(v.userId), username: null, display_name: userName(u), bot: false },
        muted: !!(v.mute || v.selfMute),
        deafened: !!(v.deaf || v.selfDeaf),
        streaming: !!v.selfStream,
        video: !!v.selfVideo
      };
    });
  }
  function voiceWho(p) {
    if (p.channel) {
      const ch = resolveChannel(p.channel, { voice: true });
      const info = describeChannel(ch);
      if (!VOICE_KINDS.has(ch.type)) throw new Error(`${info.kind} "${info.name}" is not a voice channel`);
      return { channel: info, participants: participants(String(ch.id)) };
    }
    if (!p.guild) throw new Error("give a voice channel (Server/channel) or a server name");
    const g = resolveGuild(p.guild);
    const voice = guildChannels(String(g.id)).filter((c) => c.type === 2 || c.type === 13);
    const occupied = voice.map((c) => ({ channel: describeChannel(c), participants: participants(String(c.id)) })).filter((c) => c.participants.length > 0);
    return {
      guild: { id: String(g.id), name: String(g.name) },
      voice_channels: voice.length,
      people_in_voice: occupied.reduce((n, c) => n + c.participants.length, 0),
      channels: occupied
    };
  }
  function voiceStatus() {
    const channelId = store("SelectedChannelStore", "getVoiceChannelId") ?? store("RTCConnectionStore", "getChannelId") ?? null;
    const ch = channelId ? store("ChannelStore", "getChannel", channelId) : null;
    return {
      connected: !!channelId && !!store("RTCConnectionStore", "isConnected"),
      state: String(store("RTCConnectionStore", "getState") ?? "unknown"),
      channel: ch ? describeChannel(ch) : null,
      self_mute: !!store("MediaEngineStore", "isSelfMute"),
      self_deaf: !!store("MediaEngineStore", "isSelfDeaf"),
      participants: channelId ? participants(String(channelId)) : []
    };
  }
  async function joinVoice(p) {
    const ch = resolveChannel(p.channel, { voice: true });
    const info = describeChannel(ch);
    if (!VOICE_KINDS.has(ch.type)) throw new Error(`${info.kind} "${info.name}" is not a voice channel`);
    const id = String(ch.id);
    const isPrivate = ch.type === 1 || ch.type === 3;
    const others = participants(id);
    const plan = { channel: info, would_ring: isPrivate && others.length === 0, participants_now: others.length };
    if (plan.would_ring && !p.ring) {
      throw new Error(`nobody is in a call in "${info.name}"; joining would start one and ring the other people (pass ring to allow)`);
    }
    if (p.dry_run) return { dry_run: true, ...plan };
    const cur = voiceStatus();
    if (cur.connected && cur.channel?.id === id) return { joined: true, already: true, ...cur };
    const actions = requireFns(["selectVoiceChannel", "disconnect"]);
    await actions.selectVoiceChannel(id);
    const ok = await until(() => String(store("RTCConnectionStore", "getChannelId")) === id && !!store("RTCConnectionStore", "isConnected"), 2e4);
    if (!ok) {
      throw new Error("the client did not connect (the channel may need a confirmation in the Discord window, or you lack the permission to connect)");
    }
    return { joined: true, already: false, ...voiceStatus() };
  }
  async function leaveVoice(p) {
    const cur = voiceStatus();
    if (p.dry_run) return { dry_run: true, would_leave: cur.channel };
    if (!cur.channel) return { left: false, reason: "not in a voice channel", ...cur };
    const actions = requireFns(["selectVoiceChannel", "disconnect"]);
    await actions.disconnect();
    const ok = await until(() => !store("SelectedChannelStore", "getVoiceChannelId"), 1e4);
    if (!ok) throw new Error("the client is still connected after disconnect");
    await sleep2(1500);
    if (store("SelectedChannelStore", "getVoiceChannelId")) {
      await actions.disconnect();
      if (!await until(() => !store("SelectedChannelStore", "getVoiceChannelId"), 1e4)) throw new Error("the client keeps reconnecting after disconnect");
    }
    return { left: true, ...voiceStatus() };
  }
  async function setFlag(kind, want, dry) {
    const read = () => !!store("MediaEngineStore", kind === "mute" ? "isSelfMute" : "isSelfDeaf");
    if (dry) return { dry_run: true, [kind]: want, now: read() };
    if (read() === want) return { changed: false, ...voiceStatus() };
    const actions = requireFns(["toggleSelfMute", "toggleSelfDeaf"]);
    const toggle = () => kind === "mute" ? actions.toggleSelfMute() : actions.toggleSelfDeaf();
    await toggle();
    if (!await until(() => read() === want, 1500)) {
      await toggle();
      if (!await until(() => read() === want, 3e3)) throw new Error(`could not change self ${kind}`);
    }
    return { changed: true, ...voiceStatus() };
  }
  var setMute = (mute, dry = false) => setFlag("mute", mute, dry);
  var setDeafen = (deafen, dry = false) => setFlag("deaf", deafen, dry);

  // src/stream.ts
  var sleep3 = (ms) => new Promise((r) => setTimeout(r, ms));
  var store2 = (name, method, ...args) => {
    const s = getStore(name);
    const fn = s[method];
    if (typeof fn !== "function") throw new Error(`${name}.${method} is missing (Discord internals changed?)`);
    return fn.apply(s, args);
  };
  async function until2(cond, ms) {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      try {
        if (cond()) return true;
      } catch {
      }
      await sleep3(300);
    }
    return false;
  }
  var kindOf = (id) => id.startsWith("screen:") ? "screen" : "window";
  async function rawSources(thumbs = false) {
    const native = window.DiscordNative;
    const fn = native?.desktopCapture?.getDesktopCaptureSources;
    if (!native || typeof fn !== "function") throw new Error("DiscordNative.desktopCapture.getDesktopCaptureSources is missing (Discord internals changed?)");
    const size = thumbs ? { width: 150, height: 150 } : { width: 0, height: 0 };
    return await fn.call(native.desktopCapture, { types: ["screen", "window"], thumbnailSize: size });
  }
  async function streamSources() {
    return (await rawSources()).map((s) => ({ id: String(s.id), name: String(s.name ?? ""), kind: kindOf(String(s.id)) }));
  }
  function currentStream() {
    return store2("ApplicationStreamingStore", "getCurrentUserActiveStream") ?? null;
  }
  function streamStatus() {
    const s = currentStream();
    const ch = s ? store2("ChannelStore", "getChannel", s.channelId) : null;
    return { streaming: !!s, channel: ch ? describeChannel(ch) : null, voice: voiceStatus().channel };
  }
  function streamKey(s) {
    return s.streamType === "guild" ? `guild:${s.guildId}:${s.channelId}:${s.ownerId}` : `call:${s.channelId}:${s.ownerId}`;
  }
  async function startStream(p) {
    const list = await rawSources();
    const src = list.find((s) => String(s.id) === p.source);
    if (!src) throw new Error(`source "${p.source}" not found (use the list of sources; window ids change when windows close)`);
    const plan = { source: { id: String(src.id), name: String(src.name ?? ""), kind: kindOf(String(src.id)) }, audio: !!p.audio };
    if (p.dry_run) return { dry_run: true, ...plan, ...streamStatus() };
    if (currentStream()) throw new Error("a stream is already running; stop it first");
    if (!voiceStatus().connected) throw new Error("not in a voice channel: join one first (the stream goes to the channel you are in)");
    const start = findFnBySource(['location:"startStreamWithSource"', "Starting stream for source id"]);
    if (!start) throw new Error("client function startStreamWithSource not found (Discord internals changed?)");
    const result = await start(src, { soundshareEnabled: !!p.audio });
    if (!Array.isArray(result) || result[0] !== true) throw new Error(`the client refused to start the stream: ${result?.[1] ?? "unknown reason"}`);
    if (!await until2(() => !!currentStream(), 15e3)) throw new Error("the stream did not appear (the client accepted the request but nothing started)");
    return { started: true, ...plan, ...streamStatus() };
  }
  async function stopStream(p) {
    const s = currentStream();
    if (p.dry_run) return { dry_run: true, would_stop: !!s, ...streamStatus() };
    if (!s) return { stopped: false, reason: "no stream is running", ...streamStatus() };
    const stop = findFnBySource(['type:"STREAM_STOP"']);
    if (!stop) throw new Error("client function that stops streams not found (Discord internals changed?)");
    requireFns(["toggleSelfMute", "setGoLiveSource"]).setGoLiveSource(null);
    stop(streamKey(s), true, true);
    if (!await until2(() => !currentStream(), 1e4)) throw new Error("the stream is still running after stop");
    return { stopped: true, ...streamStatus() };
  }

  // src/write.ts
  var MAX_CONTENT = 4e3;
  var isId2 = (s) => /^\d{15,25}$/.test(s);
  var DISCORD_EPOCH = 1420070400000n;
  var nonce = () => String(BigInt(Date.now()) - DISCORD_EPOCH << 22n | BigInt(Math.floor(Math.random() * 4194304)));
  function me() {
    const u = getStore("UserStore").getCurrentUser?.();
    if (!u) throw new Error("not logged in yet");
    return String(u.id);
  }
  function checkContent(content) {
    if (typeof content !== "string" || !content.trim()) throw new Error("message content is empty");
    if (content.length > MAX_CONTENT) throw new Error(`message is ${content.length} characters, the limit is ${MAX_CONTENT}`);
    return content;
  }
  function checkId(value, what) {
    const s = String(value ?? "");
    if (!isId2(s)) throw new Error(`${what} must be a message id (digits), got "${s}"`);
    return s;
  }
  var allowedMentions = (mentions) => mentions ? { parse: ["users"], replied_user: true } : { parse: [], replied_user: false };
  async function run(c, method, url, body, wrap) {
    const uid = me();
    if (c.dry_run) return { dry_run: true, request: { method: method.toUpperCase(), url, body: body ?? null } };
    const res = await apiRequest(uid, method, url, { body });
    return wrap ? wrap(res) : { result: res };
  }
  async function sendMessage(p) {
    const content = checkContent(p.content);
    const ch = resolveChannel(p.channel);
    const body = { content, nonce: nonce(), tts: false, flags: 0, allowed_mentions: allowedMentions(!!p.mentions) };
    if (p.reply_to) {
      const ref = { message_id: checkId(p.reply_to, "reply_to"), channel_id: String(ch.id) };
      if (ch.guild_id) ref.guild_id = String(ch.guild_id);
      body.message_reference = ref;
    }
    const out = await run(p, "post", `/channels/${ch.id}/messages`, body, (r) => ({ sent: true, message: toMessage(r) }));
    return { channel: describeChannel(ch), ...out };
  }
  async function editMessage(p) {
    const content = checkContent(p.content);
    const ch = resolveChannel(p.channel);
    const id = checkId(p.message, "message");
    const body = { content, allowed_mentions: allowedMentions(!!p.mentions) };
    const out = await run(p, "patch", `/channels/${ch.id}/messages/${id}`, body, (r) => ({ edited: true, message: toMessage(r) }));
    return { channel: describeChannel(ch), ...out };
  }
  async function deleteMessage(p) {
    const ch = resolveChannel(p.channel);
    const id = checkId(p.message, "message");
    const out = await run(p, "delete", `/channels/${ch.id}/messages/${id}`, void 0, () => ({ deleted: true, message_id: id }));
    return { channel: describeChannel(ch), ...out };
  }
  async function react(p) {
    const emoji = String(p.emoji ?? "").trim();
    if (!emoji) throw new Error("emoji is empty");
    const ch = resolveChannel(p.channel);
    const id = checkId(p.message, "message");
    const url = `/channels/${ch.id}/messages/${id}/reactions/${encodeURIComponent(emoji)}/@me`;
    const out = await run(p, p.remove ? "delete" : "put", url, void 0, () => ({ reacted: !p.remove, emoji, message_id: id }));
    return { channel: describeChannel(ch), ...out };
  }

  // src/methods.ts
  var PAYLOAD_VERSION = 2;
  function buildInfo() {
    const env = window.GLOBAL_ENV;
    const tags = env?.SENTRY_TAGS;
    return {
      release_channel: env?.RELEASE_CHANNEL ?? null,
      build_number: tags?.buildId ?? null
    };
  }
  function loggedIn() {
    try {
      return !!getStore("UserStore").getCurrentUser?.();
    } catch {
      return false;
    }
  }
  function status() {
    const main = mainRuntime();
    const missing = missingStores();
    return {
      v: PAYLOAD_VERSION,
      ...webpackInfo(),
      main_modules: main?.modules ?? 0,
      stores: allStores().size,
      missing,
      // Пока Discord не принёс данные аккаунта (READY), списки пусты: считаем это «не готов», а не «пусто».
      ready: !!main && missing.length === 0 && loggedIn(),
      ...buildInfo()
    };
  }
  function selfcheck() {
    const found = allStores();
    const report = {};
    let ok = true;
    for (const [name, methods2] of Object.entries(REQUIRED)) {
      const store3 = found.get(name);
      const missing_methods = store3 ? methods2.filter((m) => typeof store3[m] !== "function") : methods2;
      report[name] = { found: !!store3, missing_methods };
      if (!store3 || missing_methods.length) ok = false;
    }
    const actions = {
      "selectVoiceChannel+disconnect": !!findByFns(["selectVoiceChannel", "disconnect"]),
      "toggleSelfMute+toggleSelfDeaf": !!findByFns(["toggleSelfMute", "toggleSelfDeaf"])
    };
    if (Object.values(actions).some((v) => !v)) ok = false;
    return { ok, ...buildInfo(), stores: report, actions, http: httpInfo() };
  }
  function me2() {
    const u = getStore("UserStore").getCurrentUser?.();
    if (!u) throw new Error("not logged in yet");
    return { id: u.id, username: u.username };
  }
  var methods = {
    ping: () => "pong",
    status: () => status(),
    selfcheck: () => selfcheck(),
    stores: () => [...allStores().keys()].sort(),
    me: () => me2(),
    send: (p) => sendMessage({
      channel: String(p.channel ?? ""),
      content: p.content,
      reply_to: p.reply_to == null ? void 0 : String(p.reply_to),
      mentions: p.mentions === true,
      dry_run: p.dry_run === true
    }),
    edit: (p) => editMessage({
      channel: String(p.channel ?? ""),
      message: String(p.message ?? ""),
      content: p.content,
      mentions: p.mentions === true,
      dry_run: p.dry_run === true
    }),
    delete: (p) => deleteMessage({ channel: String(p.channel ?? ""), message: String(p.message ?? ""), dry_run: p.dry_run === true }),
    react: (p) => react({
      channel: String(p.channel ?? ""),
      message: String(p.message ?? ""),
      emoji: String(p.emoji ?? ""),
      remove: p.remove === true,
      dry_run: p.dry_run === true
    }),
    stream_sources: () => streamSources(),
    stream_status: () => streamStatus(),
    stream_start: (p) => startStream({ source: String(p.source ?? ""), audio: p.audio === true, dry_run: p.dry_run === true }),
    stream_stop: (p) => stopStream({ dry_run: p.dry_run === true }),
    voice_status: () => voiceStatus(),
    voice_who: (p) => voiceWho({
      channel: p.channel == null ? void 0 : String(p.channel),
      guild: p.guild == null ? void 0 : String(p.guild)
    }),
    members: (p) => members({
      guild: String(p.guild ?? ""),
      query: p.query == null ? void 0 : String(p.query),
      limit: p.limit == null ? void 0 : Number(p.limit)
    }),
    voice_join: (p) => joinVoice({ channel: String(p.channel ?? ""), ring: p.ring === true, dry_run: p.dry_run === true }),
    voice_leave: (p) => leaveVoice({ dry_run: p.dry_run === true }),
    voice_mute: (p) => setMute(p.mute !== false, p.dry_run === true),
    voice_deafen: (p) => setDeafen(p.deafen !== false, p.dry_run === true),
    rest_candidates: () => describeCandidates(),
    guilds: () => guilds(),
    channels: (p) => channels(String(p.guild ?? "")),
    dms: () => dms(),
    friends: () => friends(),
    resolve: (p) => describeChannel(resolveChannel(String(p.channel ?? ""), { voice: p.voice === true })),
    messages: (p) => readMessages({
      channel: String(p.channel ?? ""),
      limit: p.limit == null ? void 0 : Number(p.limit),
      before: p.before == null ? void 0 : String(p.before),
      after: p.after == null ? void 0 : String(p.after)
    }),
    /** Диагностика: модули, в чьих экспортах (на один уровень вглубь) есть все перечисленные функции. */
    find_modules: (p) => {
      const props = p.props ?? [];
      const main = mainRuntime();
      if (!main || !props.length) return [];
      const hits = [];
      for (const id in main.req.c) {
        let ex;
        try {
          ex = main.req.c[id]?.exports;
        } catch {
          continue;
        }
        if (!ex || typeof ex !== "object" && typeof ex !== "function") continue;
        const isProxy2 = (o) => {
          try {
            return typeof o["__discordctl_probe__"] === "function";
          } catch {
            return false;
          }
        };
        const has = (o) => !!o && (typeof o === "object" || typeof o === "function") && !isProxy2(o) && props.every((k) => {
          try {
            return typeof o[k] === "function";
          } catch {
            return false;
          }
        });
        if (has(ex)) hits.push({ id, key: null, props });
        try {
          for (const k of Object.keys(ex)) if (has(ex[k])) hits.push({ id, key: k, props });
        } catch {
        }
        if (hits.length >= 20) break;
      }
      return hits;
    },
    /** Диагностика: функции модуля по id и ключу экспорта, с началом исходника. */
    module_fns: (p) => {
      const main = mainRuntime();
      const ex = main?.req.c[String(p.id)]?.exports;
      const obj = ex && p.key ? ex[String(p.key)] : ex;
      if (!obj) throw new Error("module not found");
      const only = p.fn ? String(p.fn) : null;
      const n = Number(p.chars ?? 160);
      return Object.keys(obj).filter((k) => typeof obj[k] === "function" && (!only || k === only)).map((k) => ({ name: k, length: obj[k].length, src: String(obj[k]).slice(0, n) }));
    },
    /** Диагностика: ключи DiscordNative (мост к Electron) без вызова чего-либо. */
    native_keys: (p) => {
      const n = window.DiscordNative;
      if (!n) return null;
      const part = p.part ? n[String(p.part)] : n;
      return part ? Object.keys(part) : null;
    },
    /** Диагностика: модули, у которых есть функции с именем по регулярному выражению (без Proxy). */
    find_fn_names: (p) => {
      const re = new RegExp(String(p.pattern), "i");
      const main = mainRuntime();
      const out = [];
      if (!main) return out;
      for (const id in main.req.c) {
        let ex;
        try {
          ex = main.req.c[id]?.exports;
        } catch {
          continue;
        }
        if (!ex || typeof ex !== "object" && typeof ex !== "function") continue;
        const objs = [["", ex]];
        try {
          for (const k of Object.keys(ex)) objs.push([k, ex[k]]);
        } catch {
        }
        for (const [key, o] of objs) {
          try {
            if (!o || typeof o !== "object" && typeof o !== "function") continue;
            if (typeof o["__discordctl_probe__"] === "function") continue;
            const names = Object.keys(o).filter((k) => typeof o[k] === "function" && re.test(k));
            if (names.length) out.push({ id, key, names: names.slice(0, 12) });
          } catch {
          }
        }
        if (out.length >= 30) break;
      }
      return out;
    },
    /** Диагностика: функции, в чьём исходнике есть подстрока (например, тип действия Flux). */
    find_fn_src: (p) => {
      const needle = String(p.contains);
      const n = Number(p.chars ?? 240);
      const main = mainRuntime();
      const out = [];
      if (!main) return out;
      for (const id in main.req.c) {
        let ex;
        try {
          ex = main.req.c[id]?.exports;
        } catch {
          continue;
        }
        if (!ex || typeof ex !== "object" && typeof ex !== "function") continue;
        const objs = [["", ex]];
        try {
          for (const k of Object.keys(ex)) objs.push([k, ex[k]]);
        } catch {
        }
        for (const [key, o] of objs) {
          try {
            if (!o || typeof o !== "object" && typeof o !== "function") continue;
            if (typeof o["__discordctl_probe__"] === "function") continue;
            for (const k of Object.keys(o)) {
              if (typeof o[k] !== "function") continue;
              const src = String(o[k]);
              const at = src.indexOf(needle);
              if (at >= 0) out.push({ id, key, fn: k, src: src.slice(Math.max(0, at - Number(p.before ?? 60)), at + n) });
            }
          } catch {
          }
        }
        if (out.length >= 12) break;
      }
      return out;
    },
    describe: (p) => {
      const store3 = getStore(String(p.store));
      const names = /* @__PURE__ */ new Set();
      for (let o = store3; o && o !== Object.prototype; o = Object.getPrototypeOf(o)) {
        for (const k of Object.getOwnPropertyNames(o)) if (typeof store3[k] === "function") names.add(k);
      }
      return [...names].sort();
    },
    methods: () => Object.keys(methods).sort()
  };
  async function call2(method, params) {
    try {
      const fn = Object.prototype.hasOwnProperty.call(methods, method) ? methods[method] : void 0;
      if (!fn) return JSON.stringify({ ok: false, error: `unknown method ${method}` });
      return JSON.stringify({ ok: true, result: await fn(params ?? {}) });
    } catch (e) {
      return JSON.stringify({ ok: false, error: e instanceof Error ? e.message : String(e) });
    }
  }

  // src/index.ts
  if (!("__discordctl" in window)) {
    installWebpackHook();
    Object.defineProperty(window, "__discordctl", {
      value: Object.freeze({ status: () => status(), call: call2 }),
      enumerable: false
    });
  }
})();
