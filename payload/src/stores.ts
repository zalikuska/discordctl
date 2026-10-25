// Поиск stores Flux в кэше основного runtime.

import { mainRuntime } from "./webpack";

export type Store = Record<string, (...args: unknown[]) => unknown>;

/**
 * Обязательные stores и методы, которые мы от них ждём. Без самих stores читать нечего (`ready = false`),
 * пропавшие методы попадают в selfcheck: так видно, что именно поменял Discord.
 */
export const REQUIRED: Record<string, string[]> = {
  UserStore: ["getCurrentUser", "getUser"],
  GuildStore: ["getGuild", "getGuilds"],
  ChannelStore: ["getChannel", "getSortedPrivateChannels"],
  MessageStore: ["getMessages", "getMessage"],
  SelectedChannelStore: ["getChannelId"],
  RelationshipStore: ["getFriendIDs", "getRelationshipType"],
};

let cache: { modules: number; stores: Map<string, Store> } = { modules: -1, stores: new Map() };

function isStore(c: unknown): c is Store {
  if (!c || typeof c !== "object") return false;
  const s = c as Record<string, unknown>;
  // По набору методов матчить нельзя: попадаем в заглушки. Настоящий store отдаёт своё имя.
  return typeof s.getName === "function" && typeof s.addChangeListener === "function";
}

export function allStores(): Map<string, Store> {
  const main = mainRuntime();
  if (!main) return cache.stores;
  if (cache.modules === main.modules) return cache.stores;
  const stores = new Map<string, Store>();
  for (const id in main.req.c) {
    let ex: unknown;
    try {
      ex = main.req.c[id]?.exports;
    } catch {
      continue;
    }
    if (!ex || (typeof ex !== "object" && typeof ex !== "function")) continue;
    const candidates: unknown[] = [ex];
    try {
      for (const k of Object.keys(ex)) candidates.push((ex as Record<string, unknown>)[k]);
    } catch {
      // геттер экспорта может бросить, если модуль ещё не доинициализирован
    }
    for (const c of candidates) {
      try {
        if (!isStore(c)) continue;
        const name = (c.getName as () => unknown)();
        if (typeof name === "string" && !stores.has(name)) stores.set(name, c);
      } catch {
        // пропускаем
      }
    }
  }
  cache = { modules: main.modules, stores };
  return stores;
}

export function getStore(name: string): Store {
  const s = allStores().get(name);
  if (!s) throw new Error(`store ${name} not found (Discord internals changed?)`);
  return s;
}

export function missingStores(): string[] {
  const found = allStores();
  return Object.keys(REQUIRED).filter((n) => !found.has(n));
}
