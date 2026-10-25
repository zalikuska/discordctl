// Методы, доступные снаружи через `call(метод, параметры)`. Результат обязан сериализоваться в JSON.

import { findDispatcher, watchDrain, watchSet } from "./events";
import { presence, type PresenceFilter } from "./presence";
import { channels, describeChannel, dms, friends, guilds, members, resolveChannel } from "./directory";
import { describeCandidates, httpInfo, writeBudget } from "./http";
import { readMessages } from "./messages";
import { findByFns } from "./modules";
import { startStream, stopStream, streamSources, streamStatus } from "./stream";
import { joinVoice, leaveVoice, setDeafen, setMute, voiceStatus, voiceWho } from "./voice";
import { deleteMessage, editMessage, react, sendMessage } from "./write";
import { allStores, getStore, missingStores, REQUIRED } from "./stores";
import { mainRuntime, webpackInfo } from "./webpack";

type Params = Record<string, unknown>;
type Method = (params: Params) => unknown | Promise<unknown>;

export const PAYLOAD_VERSION = 2;

function buildInfo() {
  const env = (window as unknown as { GLOBAL_ENV?: Record<string, unknown> }).GLOBAL_ENV;
  const tags = env?.SENTRY_TAGS as Record<string, unknown> | undefined;
  return {
    release_channel: (env?.RELEASE_CHANNEL as string | undefined) ?? null,
    build_number: (tags?.buildId as string | undefined) ?? null,
  };
}

function loggedIn(): boolean {
  try {
    return !!getStore("UserStore").getCurrentUser?.();
  } catch {
    return false;
  }
}

export function status() {
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
    ...buildInfo(),
  };
}

/** Самопроверка на версию Discord: какие ожидаемые stores и методы пропали. */
function selfcheck() {
  const found = allStores();
  const report: Record<string, { found: boolean; missing_methods: string[] }> = {};
  let ok = true;
  for (const [name, methods] of Object.entries(REQUIRED)) {
    const store = found.get(name);
    const missing_methods = store ? methods.filter((m) => typeof store[m] !== "function") : methods;
    report[name] = { found: !!store, missing_methods };
    if (!store || missing_methods.length) ok = false;
  }
  // Создатели действий клиента, которыми пользуется голос: пропали имена функций — значит, Discord их переименовал.
  const actions: Record<string, boolean> = {
    "selectVoiceChannel+disconnect": !!findByFns(["selectVoiceChannel", "disconnect"]),
    "toggleSelfMute+toggleSelfDeaf": !!findByFns(["toggleSelfMute", "toggleSelfDeaf"]),
  };
  if (Object.values(actions).some((v) => !v)) ok = false;
  // События нужны только для подписок: их отсутствие не делает клиент «несовместимым».
  return { ok, ...buildInfo(), stores: report, actions, http: httpInfo(), events: { dispatcher: !!findDispatcher() } };
}

function me() {
  const u = getStore("UserStore").getCurrentUser?.() as { id: string; username: string } | undefined;
  if (!u) throw new Error("not logged in yet");
  return { id: u.id, username: u.username };
}

const methods: Record<string, Method> = {
  ping: () => "pong",
  status: () => status(),
  selfcheck: () => selfcheck(),
  stores: () => [...allStores().keys()].sort(),
  me: () => me(),
  send: (p) =>
    sendMessage({
      channel: String(p.channel ?? ""),
      content: p.content as string,
      reply_to: p.reply_to == null ? undefined : String(p.reply_to),
      mentions: p.mentions === true,
      dry_run: p.dry_run === true,
    }),
  edit: (p) =>
    editMessage({
      channel: String(p.channel ?? ""),
      message: String(p.message ?? ""),
      content: p.content as string,
      mentions: p.mentions === true,
      dry_run: p.dry_run === true,
    }),
  delete: (p) => deleteMessage({ channel: String(p.channel ?? ""), message: String(p.message ?? ""), dry_run: p.dry_run === true }),
  react: (p) =>
    react({
      channel: String(p.channel ?? ""),
      message: String(p.message ?? ""),
      emoji: String(p.emoji ?? ""),
      remove: p.remove === true,
      dry_run: p.dry_run === true,
    }),
  stream_sources: () => streamSources(),
  stream_status: () => streamStatus(),
  stream_start: (p) => startStream({ source: String(p.source ?? ""), audio: p.audio === true, dry_run: p.dry_run === true }),
  stream_stop: (p) => stopStream({ dry_run: p.dry_run === true }),
  voice_status: () => voiceStatus(),
  watch_set: (p) =>
    watchSet({
      kinds: Array.isArray(p.kinds) ? p.kinds.map(String) : [],
      presence: Array.isArray(p.presence) ? (p.presence as PresenceFilter[]) : null,
    }),
  presence: (p) =>
    presence({
      guild: p.guild == null ? undefined : String(p.guild),
      user: p.user == null ? undefined : String(p.user),
      status: p.status,
      activity: p.activity,
      limit: p.limit == null ? undefined : Number(p.limit),
    }),
  watch_drain: () => watchDrain(),
  voice_who: (p) =>
    voiceWho({
      channel: p.channel == null ? undefined : String(p.channel),
      guild: p.guild == null ? undefined : String(p.guild),
    }),
  members: (p) =>
    members({
      guild: String(p.guild ?? ""),
      query: p.query == null ? undefined : String(p.query),
      limit: p.limit == null ? undefined : Number(p.limit),
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
  messages: (p) =>
    readMessages({
      channel: String(p.channel ?? ""),
      limit: p.limit == null ? undefined : Number(p.limit),
      before: p.before == null ? undefined : String(p.before),
      after: p.after == null ? undefined : String(p.after),
    }),
  /** Диагностика: модули, в чьих экспортах (на один уровень вглубь) есть все перечисленные функции. */
  find_modules: (p) => {
    const props = (p.props as string[] | undefined) ?? [];
    const main = mainRuntime();
    if (!main || !props.length) return [];
    const hits: { id: string; key: string | null; props: string[] }[] = [];
    for (const id in main.req.c) {
      let ex: unknown;
      try {
        ex = main.req.c[id]?.exports;
      } catch {
        continue;
      }
      if (!ex || (typeof ex !== "object" && typeof ex !== "function")) continue;
      const isProxy = (o: unknown) => { try { return typeof (o as Record<string, unknown>)["__discordctl_probe__"] === "function"; } catch { return false; } };
      const has = (o: unknown) => !!o && (typeof o === "object" || typeof o === "function") && !isProxy(o) && props.every((k) => { try { return typeof (o as Record<string, unknown>)[k] === "function"; } catch { return false; } });
      if (has(ex)) hits.push({ id, key: null, props });
      try {
        for (const k of Object.keys(ex)) if (has((ex as Record<string, unknown>)[k])) hits.push({ id, key: k, props });
      } catch {
        // пропускаем
      }
      if (hits.length >= 20) break;
    }
    return hits;
  },
  /** Диагностика: функции модуля по id и ключу экспорта, с началом исходника. */
  module_fns: (p) => {
    const main = mainRuntime();
    const ex = main?.req.c[String(p.id)]?.exports as Record<string, any> | undefined; // eslint-disable-line @typescript-eslint/no-explicit-any
    const obj = ex && p.key ? ex[String(p.key)] : ex;
    if (!obj) throw new Error("module not found");
    const only = p.fn ? String(p.fn) : null;
    const n = Number(p.chars ?? 160);
    return Object.keys(obj)
      .filter((k) => typeof obj[k] === "function" && (!only || k === only))
      .map((k) => ({ name: k, length: obj[k].length, src: String(obj[k]).slice(0, n) }));
  },
  /** Диагностика: ключи DiscordNative (мост к Electron) без вызова чего-либо. */
  native_keys: (p) => {
    const n = (window as unknown as { DiscordNative?: Record<string, any> }).DiscordNative; // eslint-disable-line @typescript-eslint/no-explicit-any
    if (!n) return null;
    const part = p.part ? n[String(p.part)] : n;
    return part ? Object.keys(part) : null;
  },
  /** Диагностика: модули, у которых есть функции с именем по регулярному выражению (без Proxy). */
  find_fn_names: (p) => {
    const re = new RegExp(String(p.pattern), "i");
    const main = mainRuntime();
    const out: { id: string; key: string; names: string[] }[] = [];
    if (!main) return out;
    for (const id in main.req.c) {
      let ex: any; // eslint-disable-line @typescript-eslint/no-explicit-any
      try {
        ex = main.req.c[id]?.exports;
      } catch {
        continue;
      }
      if (!ex || (typeof ex !== "object" && typeof ex !== "function")) continue;
      const objs: [string, any][] = [["", ex]]; // eslint-disable-line @typescript-eslint/no-explicit-any
      try {
        for (const k of Object.keys(ex)) objs.push([k, ex[k]]);
      } catch {
        // пропускаем
      }
      for (const [key, o] of objs) {
        try {
          if (!o || (typeof o !== "object" && typeof o !== "function")) continue;
          if (typeof o["__discordctl_probe__"] === "function") continue;
          const names = Object.keys(o).filter((k) => typeof o[k] === "function" && re.test(k));
          if (names.length) out.push({ id, key, names: names.slice(0, 12) });
        } catch {
          // пропускаем
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
    const out: { id: string; key: string; fn: string; src: string }[] = [];
    if (!main) return out;
    for (const id in main.req.c) {
      let ex: any; // eslint-disable-line @typescript-eslint/no-explicit-any
      try {
        ex = main.req.c[id]?.exports;
      } catch {
        continue;
      }
      if (!ex || (typeof ex !== "object" && typeof ex !== "function")) continue;
      const objs: [string, any][] = [["", ex]]; // eslint-disable-line @typescript-eslint/no-explicit-any
      try {
        for (const k of Object.keys(ex)) objs.push([k, ex[k]]);
      } catch {
        // пропускаем
      }
      for (const [key, o] of objs) {
        try {
          if (!o || (typeof o !== "object" && typeof o !== "function")) continue;
          if (typeof o["__discordctl_probe__"] === "function") continue;
          for (const k of Object.keys(o)) {
            if (typeof o[k] !== "function") continue;
            const src = String(o[k]);
            const at = src.indexOf(needle);
            if (at >= 0) out.push({ id, key, fn: k, src: src.slice(Math.max(0, at - Number(p.before ?? 60)), at + n) });
          }
        } catch {
          // пропускаем
        }
      }
      if (out.length >= 12) break;
    }
    return out;
  },
  describe: (p) => {
    const store = getStore(String(p.store));
    const names = new Set<string>();
    for (let o: object | null = store; o && o !== Object.prototype; o = Object.getPrototypeOf(o)) {
      for (const k of Object.getOwnPropertyNames(o)) if (typeof (store as Record<string, unknown>)[k] === "function") names.add(k);
    }
    return [...names].sort();
  },
  methods: () => Object.keys(methods).sort(),
};

/** Единая точка входа. Всегда возвращает JSON-строку `{ok, result}` или `{ok:false, error}`: так CDP не упирается в сериализацию объектов. */
export async function call(method: string, params?: Params): Promise<string> {
  try {
    const fn = Object.prototype.hasOwnProperty.call(methods, method) ? methods[method] : undefined;
    if (!fn) return JSON.stringify({ ok: false, error: `unknown method ${method}` });
    return JSON.stringify({ ok: true, result: await fn(params ?? {}) });
  } catch (e) {
    return JSON.stringify({ ok: false, error: e instanceof Error ? e.message : String(e) });
  }
}
