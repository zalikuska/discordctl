// Поиск модулей с действиями клиента (создателей действий) по именам функций. Экспорты называются по-разному
// от сборки к сборке, а имена самих функций (selectVoiceChannel, toggleSelfMute…) пока стабильны.

import { mainRuntime } from "./webpack";

type Fns = Record<string, (...args: unknown[]) => unknown>;

const cache = new Map<string, { modules: number; found: Fns | null }>();

// Объекты локализации — Proxy, у которого «есть» любая функция: их отсеивает проба несуществующего ключа.
const isProxy = (o: unknown) => {
  try {
    return typeof (o as Record<string, unknown>)["__discordctl_probe__"] === "function";
  } catch {
    return false;
  }
};

/** Первый непрокси-объект экспорта (на уровень вглубь), у которого есть все перечисленные функции. */
export function findByFns(names: string[]): Fns | null {
  const main = mainRuntime();
  if (!main) return null;
  const key = names.join("+");
  const hit = cache.get(key);
  if (hit && (hit.found || hit.modules === main.modules)) return hit.found;
  const has = (o: unknown): o is Fns => {
    if (!o || (typeof o !== "object" && typeof o !== "function") || isProxy(o)) return false;
    try {
      return names.every((n) => typeof (o as Record<string, unknown>)[n] === "function");
    } catch {
      return false;
    }
  };
  let found: Fns | null = null;
  for (const id in main.req.c) {
    let ex: unknown;
    try {
      ex = main.req.c[id]?.exports;
    } catch {
      continue;
    }
    if (!ex || (typeof ex !== "object" && typeof ex !== "function")) continue;
    if (has(ex)) {
      found = ex;
      break;
    }
    try {
      for (const k of Object.keys(ex)) {
        const v = (ex as Record<string, unknown>)[k];
        if (has(v)) {
          found = v;
          break;
        }
      }
    } catch {
      // геттер экспорта может бросить
    }
    if (found) break;
  }
  cache.set(key, { modules: main.modules, found });
  return found;
}

export function requireFns(names: string[]): Fns {
  const f = findByFns(names);
  if (!f) throw new Error(`client action module with ${names.join(", ")} not found (Discord internals changed?)`);
  return f;
}

const srcCache = new Map<string, { modules: number; fn: ((...args: unknown[]) => unknown) | null }>();

/**
 * Экспортированная функция, в чьём исходнике есть все метки. Для действий с минифицированными именами:
 * ищем по неизменным строкам (тип Flux-действия, имя места в аналитике), а не по имени.
 */
export function findFnBySource(markers: string[]): ((...args: unknown[]) => unknown) | null {
  const main = mainRuntime();
  if (!main) return null;
  const key = markers.join("\u0000");
  const hit = srcCache.get(key);
  if (hit && (hit.fn || hit.modules === main.modules)) return hit.fn;
  let found: ((...args: unknown[]) => unknown) | null = null;
  const scan = (o: unknown) => {
    if (found || !o || (typeof o !== "object" && typeof o !== "function") || isProxy(o)) return;
    let keys: string[];
    try {
      keys = Object.keys(o);
    } catch {
      return;
    }
    for (const k of keys) {
      try {
        const v = (o as Record<string, unknown>)[k];
        if (typeof v !== "function") continue;
        const src = String(v);
        if (markers.every((m) => src.includes(m))) {
          found = v as (...args: unknown[]) => unknown;
          return;
        }
      } catch {
        // геттер может бросить
      }
    }
  };
  for (const id in main.req.c) {
    let ex: unknown;
    try {
      ex = main.req.c[id]?.exports;
    } catch {
      continue;
    }
    scan(ex);
    if (found) break;
    try {
      for (const k of Object.keys((ex ?? {}) as object)) {
        scan((ex as Record<string, unknown>)[k]);
        if (found) break;
      }
    } catch {
      // пропускаем
    }
    if (found) break;
  }
  srcCache.set(key, { modules: main.modules, fn: found });
  return found;
}
