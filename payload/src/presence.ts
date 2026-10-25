// Присутствие: в сети ли человек и во что играет. Берётся из состояния клиента (PresenceStore), ничего не запрашивается.
// Клиент знает присутствие друзей и тех участников серверов, о которых Discord прислал данные: на больших серверах
// это не все.

import { findUser, guildMembersOf, resolveGuild, userBrief } from "./directory";
import { getStore } from "./stores";

type Raw = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

const TYPES = ["playing", "streaming", "listening", "watching", "custom", "competing"];
export const STATUSES = ["online", "idle", "dnd", "offline"];

export function activityBrief(a: Raw) {
  return {
    type: TYPES[a.type] ?? String(a.type),
    name: String(a.name ?? ""),
    state: a.state ?? null,
    details: a.details ?? null,
    application_id: a.application_id ?? a.applicationId ?? null,
    since: a.timestamps?.start ?? a.created_at ?? a.createdAt ?? null,
  };
}

/** Ключ занятия: смена деталей (песня, уровень) не считается новым занятием. */
export const activityKey = (a: { type: string; name: string }) => `${a.type}:${a.name}`;

function call(store: string, method: string, ...args: unknown[]): any { // eslint-disable-line @typescript-eslint/no-explicit-any
  const s = getStore(store);
  const fn = s[method];
  if (typeof fn !== "function") throw new Error(`${store}.${method} is missing (Discord internals changed?)`);
  return fn.apply(s, args);
}

/** Сейчас, по данным клиента. */
export function presenceOf(userId: string) {
  let status = "offline";
  let client: Raw = {};
  let activities: Raw[] = [];
  try {
    status = String(call("PresenceStore", "getStatus", userId) ?? "offline");
  } catch (e) {
    if (String(e).includes("is missing")) throw e;
  }
  try {
    client = (getStore("PresenceStore").getClientStatus?.(userId) as Raw | undefined) ?? {};
  } catch {
    // нет разбивки по устройствам
  }
  try {
    activities = (getStore("PresenceStore").getActivities?.(userId) as Raw[] | undefined) ?? [];
  } catch {
    // нет занятий
  }
  return { status, client_status: client, activities: activities.map(activityBrief) };
}

const norm = (s: string) => s.trim().toLowerCase();
const words = (v: unknown): string[] =>
  (Array.isArray(v) ? v : typeof v === "string" ? v.split(",") : []).map((x) => norm(String(x))).filter(Boolean);

/**
 * Присутствие друзей (или участников сервера, или одного человека) с отбором по статусу и занятию.
 * `activity` — часть названия («minecraft»): подходит тот, кто сейчас в это играет, слушает или смотрит.
 */
export function presence(p: { guild?: string; user?: string; status?: unknown; activity?: unknown; limit?: number }) {
  const asked = words(p.status);
  const bad = asked.find((s) => !STATUSES.includes(s));
  if (bad) throw new Error(`"${bad}" is not a status (online, idle, dnd, offline)`);
  const wantStatus = asked;
  const wantActivity = words(p.activity);
  let ids: string[];
  let scope: string;
  if (p.user) {
    const u = findUser(p.user, p.guild);
    ids = [String(u.id)];
    scope = "user";
  } else if (p.guild) {
    const g = resolveGuild(p.guild);
    ids = guildMembersOf(String(g.id)).map((m) => String(m.userId));
    scope = `guild ${String(g.name)}`;
  } else {
    ids = (call("RelationshipStore", "getFriendIDs") as string[]).map(String);
    scope = "friends";
  }
  const all = ids.map((id) => {
    const u = getStore("UserStore").getUser?.(id) as Raw | undefined;
    return { user: userBrief(u) ?? { id, username: null, display_name: null, bot: false }, ...presenceOf(id) };
  });
  const rank = (s: string) => STATUSES.indexOf(s) + (s === "offline" ? 10 : 0);
  const matched = all
    .filter((x) => !wantStatus.length || wantStatus.includes(x.status))
    .filter((x) => !wantActivity.length || x.activities.some((a) => wantActivity.some((w) => norm(a.name).includes(w))))
    .sort((a, b) => rank(a.status) - rank(b.status) || String(a.user.display_name).localeCompare(String(b.user.display_name)));
  const limit = Math.min(Math.max(1, Math.floor(p.limit ?? 100) || 100), 500);
  return { scope, known: all.length, matched: matched.length, truncated: matched.length > limit, presences: matched.slice(0, limit) };
}

/** Условие подписки на присутствие. Пустое поле значит «любое». */
export interface PresenceFilter {
  status?: string[];
  activity?: string[];
  users?: string[];
}

/** Подходит ли событие: все заданные поля должны совпасть. «Статус» — вошёл в этот статус, «занятие» — начал или бросил его. */
export function presenceMatches(f: PresenceFilter, ev: Raw): boolean {
  if (f.users?.length && !f.users.includes(String(ev.user?.id))) return false;
  if (f.status?.length && !(f.status.includes(ev.status) && ev.prev_status !== ev.status)) return false;
  if (f.activity?.length) {
    const names: string[] = ((ev.changes ?? []) as Raw[]).map((c) => norm(String(c.name)));
    if (!names.some((n) => f.activity!.some((w) => n.includes(norm(w))))) return false;
  }
  return true;
}
