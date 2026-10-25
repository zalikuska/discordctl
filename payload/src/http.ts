// HTTP-модуль самого клиента: он сам подставляет токен и заголовки, поэтому свой fetch не нужен.
// Все запросы идут по одному, с паузой и разбором 429: чтение истории не должно выглядеть как бот.

import { mainRuntime } from "./webpack";

interface HttpResponse {
  status: number;
  body: unknown;
}
interface RestModule {
  get(opts: Record<string, unknown>): Promise<HttpResponse>;
  getAPIBaseURL?: () => string;
}

const MIN_GAP_MS = 350;
const JITTER_MS = 300;
const MAX_RETRIES_429 = 3;

function withTimeout<T>(p: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`timeout ${ms} ms`)), ms);
    p.then(
      (v) => (clearTimeout(t), resolve(v)),
      (e) => (clearTimeout(t), reject(e)),
    );
  });
}

let rest: RestModule | null = null;
let probeLog: string[] = [];

function candidates(): RestModule[] {
  const main = mainRuntime();
  const found = new Set<RestModule>();
  if (!main) return [];
  const isRest = (o: unknown): o is RestModule => {
    if (!o || (typeof o !== "object" && typeof o !== "function")) return false;
    try {
      const r = o as Record<string, unknown>;
      if (!["get", "post", "patch", "put"].every((k) => typeof r[k] === "function")) return false;
      if (typeof r.del !== "function" && typeof r.delete !== "function") return false;
      // Объекты локализации — Proxy, у которого «есть» любой метод, а у superagent get(url, query, cb) возвращает запрос.
      // Настоящий HTTP-модуль отсеивается не по виду, а по пробе в locate().
      const src = String(r.get);
      return src !== "r=>e.get(t,r)" && !src.includes(".end(");
    } catch {
      return false;
    }
  };
  for (const id in main.req.c) {
    let ex: unknown;
    try {
      ex = main.req.c[id]?.exports;
    } catch {
      continue;
    }
    if (!ex || (typeof ex !== "object" && typeof ex !== "function")) continue;
    if (isRest(ex)) found.add(ex);
    try {
      for (const k of Object.keys(ex)) {
        const v = (ex as Record<string, unknown>)[k];
        if (isRest(v)) found.add(v);
      }
    } catch {
      // пропускаем
    }
  }
  return [...found];
}

/** Подходит тот кандидат, чей GET /users/@me вернул пользователя. Заглушки и чужие обёртки отсеиваются сами. */
async function locate(expectedUserId: string): Promise<RestModule> {
  if (rest) return rest;
  probeLog = [];
  const list = candidates();
  for (const [i, c] of list.entries()) {
    try {
      const res = await withTimeout(c.get({ url: "/users/@me" }), 5000);
      const id = (res?.body as { id?: string } | undefined)?.id;
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

/** Диагностика: как выглядят кандидаты (исходник get и базовый URL). */
export function describeCandidates() {
  const main = mainRuntime();
  const loose: unknown[] = [];
  for (const id in main?.req.c ?? {}) {
    let ex: any; // eslint-disable-line @typescript-eslint/no-explicit-any
    try {
      ex = main!.req.c[id]?.exports;
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
    for (const [k, o] of objs) {
      try {
        if (!o || typeof o.get !== "function" || typeof o.post !== "function" || typeof o.patch !== "function") continue;
        const src = String(o.get);
        if (src === "r=>e.get(t,r)") continue;
        loose.push({ id, key: k, has_del: typeof o.del, has_delete: typeof o.delete, base_fn: typeof o.getAPIBaseURL, get_src: src.slice(0, 200) });
      } catch {
        // пропускаем
      }
    }
  }
  return loose;
}

export function describeCandidatesOld() {
  return candidates().map((c, i) => {
    let base: unknown = null;
    try {
      base = c.getAPIBaseURL?.();
    } catch (e) {
      base = `throws: ${e}`;
    }
    return { i, base, keys: Object.keys(c).slice(0, 12), get_len: String(c.get).length, get_src: String(c.get).slice(0, 160) };
  });
}

export function httpInfo() {
  return { located: rest !== null, distinct_candidates: candidates().length, probe_log: probeLog, writes: writeBudget() };
}

let queue: Promise<unknown> = Promise.resolve();
let last = 0;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export type Method = "get" | "post" | "patch" | "put" | "delete";

// Запись реже и с большим зазором, чем чтение, плюс потолок в минуту: поток правок не должен выглядеть как бот.
// Упёршись в потолок, отказываем сразу с понятной ошибкой, а не засыпаем на минуты.
const WRITE_GAP_MS = 1200;
const WRITE_JITTER_MS = 900;
const MAX_WRITES_PER_MIN = 15;
const writeTimes: number[] = [];

function reserveWrite(): void {
  const now = Date.now();
  while (writeTimes.length && now - writeTimes[0]! > 60_000) writeTimes.shift();
  if (writeTimes.length >= MAX_WRITES_PER_MIN) {
    const wait = Math.ceil((60_000 - (now - writeTimes[0]!)) / 1000);
    throw new Error(`write rate limit: ${MAX_WRITES_PER_MIN} actions per minute, retry in ${wait} s`);
  }
  writeTimes.push(now);
}

export function writeBudget() {
  const now = Date.now();
  return { limit_per_minute: MAX_WRITES_PER_MIN, used: writeTimes.filter((t) => now - t <= 60_000).length };
}

/**
 * Запрос через клиентский модуль. Всё выстраивается в одну очередь.
 * При 429 ждём `retry_after` и повторяем (отклонённый запрос сообщение не создаёт); при таймауте не повторяем:
 * запись могла дойти, дубль хуже ошибки.
 */
export function apiRequest(
  expectedUserId: string,
  method: Method,
  url: string,
  opts: { query?: Record<string, unknown>; body?: unknown } = {},
): Promise<unknown> {
  const isWrite = method !== "get";
  if (isWrite) {
    try {
      reserveWrite();
    } catch (e) {
      return Promise.reject(e);
    }
  }
  const run = async () => {
    const gap = isWrite ? WRITE_GAP_MS + Math.random() * WRITE_JITTER_MS : MIN_GAP_MS + Math.random() * JITTER_MS;
    const wait = last + gap - Date.now();
    if (wait > 0) await sleep(wait);
    const mod = await locate(expectedUserId);
    const m = mod as unknown as Record<string, ((o: Record<string, unknown>) => Promise<HttpResponse>) | undefined>;
    const fn = method === "delete" ? m.del ?? m.delete : m[method];
    if (typeof fn !== "function") throw new Error(`HTTP module has no ${method}`);
    for (let attempt = 0; ; attempt++) {
      try {
        const res = await withTimeout(fn.call(mod, { url, query: opts.query, body: opts.body }), 30000);
        last = Date.now();
        return res.body;
      } catch (e) {
        last = Date.now();
        const r = e as { status?: number; body?: { retry_after?: number; message?: string } };
        if (r?.status === 429 && attempt < MAX_RETRIES_429) {
          await sleep(Math.ceil((r.body?.retry_after ?? 1) * 1000) + 250);
          continue;
        }
        if (r && typeof r.status === "number") throw new Error(`HTTP ${r.status}: ${r.body?.message ?? "request failed"}`);
        throw e;
      }
    }
  };
  const p = queue.then(run, run);
  queue = p.catch(() => undefined);
  return p;
}

export const apiGet = (expectedUserId: string, url: string, query?: Record<string, unknown>) =>
  apiRequest(expectedUserId, "get", url, { query });
