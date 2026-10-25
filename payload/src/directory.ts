// Серверы, каналы, личные чаты, друзья и разбор ссылок на каналы по имени или id.

import { getStore } from "./stores";

type Raw = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

const CHANNEL_TYPES: Record<number, string> = {
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
  16: "media",
};

const call = (store: string, method: string, ...args: unknown[]): any => {
  const fn = getStore(store)[method];
  if (typeof fn !== "function") throw new Error(`${store}.${method} is missing (Discord internals changed?)`);
  return fn.apply(getStore(store), args);
};

const isId = (s: string) => /^\d{15,25}$/.test(s);
const norm = (s: string) => s.trim().toLowerCase();

export function userName(u: Raw | undefined): string {
  return u ? String(u.globalName ?? u.global_name ?? u.username ?? u.id) : "?";
}

export function userBrief(u: Raw | undefined) {
  return u ? { id: String(u.id), username: String(u.username), display_name: userName(u), bot: !!u.bot } : null;
}

function friendNick(userId: string): string | null {
  try {
    const n = call("RelationshipStore", "getNickname", userId);
    return typeof n === "string" && n ? n : null;
  } catch {
    return null;
  }
}

function recipientsOf(ch: Raw): Raw[] {
  const ids: string[] = ch.recipients ?? [];
  return ids.map((id) => call("UserStore", "getUser", id)).filter(Boolean);
}

/** Имя личного чата: у группы своё название или имена участников, у ЛС имя собеседника. */
export function dmTitle(ch: Raw): string {
  if (ch.name) return String(ch.name);
  return recipientsOf(ch)
    .map((u) => friendNick(String(u.id)) ?? userName(u))
    .join(", ");
}

export function dmBrief(ch: Raw) {
  return {
    id: String(ch.id),
    kind: CHANNEL_TYPES[ch.type] ?? String(ch.type),
    name: dmTitle(ch),
    recipients: recipientsOf(ch).map((u) => userBrief(u)),
    last_message_id: ch.lastMessageId ?? null,
  };
}

export function dms() {
  const list: Raw[] = call("ChannelStore", "getSortedPrivateChannels");
  return list.map(dmBrief);
}

export function guilds() {
  const all: Raw[] = Object.values(call("GuildStore", "getGuilds"));
  const me = call("UserStore", "getCurrentUser");
  return all
    .map((g) => ({ id: String(g.id), name: String(g.name), owner: g.ownerId === me?.id, member_count: g.memberCount ?? null }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

export function friends() {
  const ids: string[] = call("RelationshipStore", "getFriendIDs");
  return ids
    .map((id) => {
      const u = call("UserStore", "getUser", id);
      return u ? { ...userBrief(u)!, nickname: friendNick(id) } : { id, username: null, display_name: null, bot: false, nickname: null };
    })
    .sort((a, b) => String(a.display_name).localeCompare(String(b.display_name)));
}

type Names<T> = (t: T) => (string | null | undefined)[];

/**
 * Выбор по id, затем по имени. `tiers` — группы имён по убыванию приоритета: точное совпадение в более
 * приоритетной группе побеждает (чат с именем alice важнее группы, где alice лишь участник).
 * Сначала точные совпадения по всем группам, потом подстрока. Несколько совпадений в одной группе — ошибка.
 */
function pick<T extends { id: string }>(items: T[], query: string, tiers: Names<T>[], what: string): T {
  const q = norm(query);
  const byId = items.find((t) => t.id === query);
  if (byId) return byId;
  for (const test of [(n: string) => n === q, (n: string) => n.includes(q)]) {
    for (const names of tiers) {
      const hit = items.filter((t) => names(t).some((n) => n && test(norm(n))));
      if (hit.length === 1) return hit[0]!;
      if (hit.length > 1) {
        throw new Error(`${what} "${query}" is ambiguous: ${hit.slice(0, 8).map((t) => `${tiers[0]!(t)[0]} (${t.id})`).join(", ")}`);
      }
    }
  }
  throw new Error(`${what} "${query}" not found`);
}

export function resolveGuild(query: string): Raw {
  const list: Raw[] = Object.values(call("GuildStore", "getGuilds"));
  return pick<Raw & { id: string }>(list.map((g) => ({ ...g, id: String(g.id) })), query, [(g) => [g.name]], "guild");
}

export function guildChannels(guildId: string): Raw[] {
  const map = call("ChannelStore", "getMutableGuildChannelsForGuild", guildId) as Record<string, Raw>;
  return Object.values(map).sort((a, b) => (a.position ?? 0) - (b.position ?? 0));
}

const MEMBERS_DEFAULT = 100;
const MEMBERS_MAX = 500;

/**
 * Участники сервера, которые клиент уже загрузил. Discord не отдаёт весь список большого сервера сразу: он подгружается
 * по мере просмотра списка участников и событий, поэтому `loaded` может быть меньше `total_members`.
 */
export function members(p: { guild: string; query?: string; limit?: number }) {
  const g = resolveGuild(p.guild);
  const gid = String(g.id);
  const raw = (call("GuildMemberStore", "getMembers", gid) ?? []) as Raw[];
  let total: number | null = null;
  try {
    const n = call("GuildMemberCountStore", "getMemberCount", gid);
    total = typeof n === "number" ? n : null;
  } catch {
    // счётчика может не быть: это только подсказка
  }
  let roleNames: Record<string, string> = {};
  try {
    const roles = (call("GuildStore", "getRoles", gid) ?? {}) as Record<string, Raw>;
    roleNames = Object.fromEntries(Object.entries(roles).map(([id, r]) => [id, String(r.name ?? id)]));
  } catch {
    // без названий ролей остаются их id
  }
  const q = norm(p.query ?? "");
  const all = raw
    .map((m) => {
      const u = call("UserStore", "getUser", m.userId) as Raw | undefined;
      const brief = userBrief(u) ?? { id: String(m.userId), username: null, display_name: null, bot: false };
      const roles = ((m.roles as string[] | undefined) ?? []).map((id) => roleNames[id] ?? id);
      return { ...brief, nickname: m.nick ?? null, roles: roles.slice(0, 10), joined_at: m.joinedAt ?? null };
    })
    .filter((m) => !q || [m.display_name, m.username, m.nickname].some((n) => n && norm(String(n)).includes(q)))
    .sort((a, b) => String(a.nickname ?? a.display_name).localeCompare(String(b.nickname ?? b.display_name)));
  const limit = Math.min(Math.max(1, Math.floor(p.limit ?? MEMBERS_DEFAULT) || MEMBERS_DEFAULT), MEMBERS_MAX);
  return {
    guild: { id: gid, name: String(g.name) },
    loaded: raw.length,
    total_members: total,
    matched: all.length,
    truncated: all.length > limit,
    members: all.slice(0, limit),
  };
}

export function channels(guildQuery: string) {
  const g = resolveGuild(guildQuery);
  const all = guildChannels(String(g.id));
  const catName = (id: unknown) => (id ? all.find((c) => c.id === id)?.name ?? null : null);
  return all
    .filter((c) => c.type !== 4)
    .map((c) => ({
      id: String(c.id),
      name: String(c.name),
      kind: CHANNEL_TYPES[c.type] ?? String(c.type),
      category: catName(c.parent_id),
      topic: c.topic ?? null,
    }));
}

/**
 * Ссылка на канал:
 *   `123…`                 id канала (или id пользователя: тогда его ЛС)
 *   `@<имя|id>`            личный чат или группа по имени, нику, имени участника (то же, что `dm:<имя>`)
 *   `guild:<сервер>/channel:<канал>` или `<сервер>/<канал>`
 */
export function resolveChannel(ref: string, opts: { voice?: boolean } = {}): Raw {
  const r = ref.trim();
  if (!r) throw new Error("empty channel reference");
  const dmMatch = /^(?:(?:dm|group):|@)(.+)$/i.exec(r);
  if (dmMatch) return resolveDm(dmMatch[1]!);
  const gm = /^(?:guild:)?(.+?)\/(?:channel:)?(.+)$/i.exec(r);
  if (gm) {
    const g = resolveGuild(gm[1]!);
    const list = guildChannels(String(g.id))
      .filter((c) => (opts.voice ? c.type === 2 || c.type === 13 : c.type !== 4))
      .map((c) => ({ ...c, id: String(c.id) }));
    return pick<Raw & { id: string }>(list, gm[2]!, [(c) => [c.name]], "channel");
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

const memberNames = (c: Raw) => recipientsOf(c).flatMap((u) => [userName(u), String(u.username), friendNick(String(u.id))]);

function resolveDm(query: string): Raw {
  const list: Raw[] = call("ChannelStore", "getSortedPrivateChannels");
  const items = list.map((c) => ({ ...c, id: String(c.id) }));
  const q = query.trim();
  if (isId(q)) {
    const byUser = call("ChannelStore", "getDMFromUserId", q);
    if (byUser) return call("ChannelStore", "getChannel", byUser);
  }
  return pick<Raw & { id: string }>(
    items,
    q,
    [
      (c) => [dmTitle(c)],
      // 1:1 чат важнее группы, где человек просто участник.
      (c) => (c.type === 1 ? memberNames(c) : []),
      (c) => (c.type === 3 ? memberNames(c) : []),
    ],
    "direct chat",
  );
}

export function describeChannel(ch: Raw) {
  const kind = CHANNEL_TYPES[ch.type] ?? String(ch.type);
  return {
    id: String(ch.id),
    kind,
    name: ch.type === 1 || ch.type === 3 ? dmTitle(ch) : String(ch.name ?? ""),
    guild_id: ch.guild_id ?? null,
  };
}
