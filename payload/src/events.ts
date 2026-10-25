// События клиента: новые и изменённые сообщения, голосовые каналы, набор текста.
//
// Клиент Discord и так получает всё это от сервера и раздаёт внутри себя через Flux-диспетчер; мы подписываемся на
// его действия. Ничего не запрашивается у Discord, ничего не меняется. Подписка включается только пока демон просит
// (`watch_set`), события копятся в небольшой очереди, демон забирает их через `watch_drain`. Какие события кому
// положены, решает демон по правам приложений; здесь только отбор по видам.

import { describeChannel, userBrief } from "./directory";
import { toMessage } from "./messages";
import { findByFns } from "./modules";
import { activityBrief, activityKey, presenceMatches, type PresenceFilter } from "./presence";
import { getStore } from "./stores";

type Raw = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

/** Виды событий, которые умеем отдавать. */
export const KINDS = ["message", "message_edit", "message_delete", "voice_join", "voice_leave", "voice_move", "voice_state", "typing", "presence"];

const MAX_QUEUE = 1000;

interface Dispatcher {
  subscribe(type: string, handler: (action: Raw) => void): void;
  unsubscribe(type: string, handler: (action: Raw) => void): void;
}

let dispatcher: Dispatcher | null = null;

export function findDispatcher(): Dispatcher | null {
  if (dispatcher) return dispatcher;
  const found = findByFns(["subscribe", "unsubscribe", "dispatch", "addInterceptor"]) ?? findByFns(["subscribe", "unsubscribe", "dispatch"]);
  dispatcher = (found as unknown as Dispatcher | null) ?? null;
  return dispatcher;
}

let wanted = new Set<string>();
let queue: Raw[] = [];
let dropped = 0;
const handlers = new Map<string, (a: Raw) => void>();
/** Условия подписки на присутствие: `null` — все события, иначе подходит событие, прошедшее хотя бы одно условие. */
let presenceFilters: PresenceFilter[] | null = null;
/** Что было у человека в прошлый раз: из этого видно, что поменялось. */
const lastPresence = new Map<string, { status: string; acts: Set<string> }>();
/** Где сейчас каждый человек в голосе: из этого видно, вошёл он, вышел или перешёл. */
const voice = new Map<string, { channel: string | null; flags: string }>();

function me(): string | null {
  try {
    const u = getStore("UserStore").getCurrentUser?.() as Raw | undefined;
    return u ? String(u.id) : null;
  } catch {
    return null;
  }
}

function user(id: unknown) {
  try {
    const u = getStore("UserStore").getUser?.(id) as Raw | undefined;
    return userBrief(u) ?? { id: String(id), username: null, display_name: null, bot: false };
  } catch {
    return { id: String(id), username: null, display_name: null, bot: false };
  }
}

function channel(id: unknown) {
  try {
    const ch = getStore("ChannelStore").getChannel?.(id) as Raw | undefined;
    return ch ? describeChannel(ch) : null;
  } catch {
    return null;
  }
}

function push(kind: string, channelId: unknown, guildId: unknown, data: Raw) {
  if (!wanted.has(kind)) return;
  if (queue.length >= MAX_QUEUE) {
    queue.shift();
    dropped++;
  }
  const uid = data.user?.id;
  // У набора текста и части других событий сервера нет: берём его у канала, иначе канал сервера выглядел бы личным.
  const ch = channelId == null ? null : channel(channelId);
  const guild = guildId ?? ch?.guild_id ?? null;
  queue.push({
    kind,
    ts: Date.now(),
    channel_id: channelId == null ? null : String(channelId),
    guild_id: guild == null ? null : String(guild),
    channel: ch,
    mine: uid != null && uid === me(),
    ...data,
  });
}

const flagsOf = (v: Raw) => [v.mute || v.deaf ? "server-muted" : "", v.selfMute ? "muted" : "", v.selfDeaf ? "deafened" : "", v.selfStream ? "streaming" : "", v.selfVideo ? "video" : ""].filter(Boolean).join(",");

function seedVoice() {
  voice.clear();
  try {
    const all = (getStore("VoiceStateStore").getAllVoiceStates?.() ?? {}) as Record<string, Record<string, Raw>>;
    for (const states of Object.values(all)) {
      for (const [uid, v] of Object.entries(states ?? {})) {
        if (v?.channelId) voice.set(String(uid), { channel: String(v.channelId), flags: flagsOf(v) });
      }
    }
  } catch {
    // без исходного состояния первые события могут показать «вход» для тех, кто уже был в канале
  }
}

const onMessage = (a: Raw) => {
  const m = a.message as Raw | undefined;
  if (!m || a.optimistic) return; // локальное эхо своего сообщения придёт ещё раз настоящим
  push("message", a.channelId ?? m.channel_id, a.guildId ?? m.guild_id, { message: toMessage({ ...m, channel_id: m.channel_id ?? a.channelId }), user: user(m.author?.id) });
};

const onEdit = (a: Raw) => {
  const m = a.message as Raw | undefined;
  if (!m?.id) return;
  push("message_edit", a.channelId ?? m.channel_id, a.guildId ?? m.guild_id, {
    message_id: String(m.id),
    // Правка приходит частичной: есть то, что изменилось.
    content: typeof m.content === "string" ? m.content : null,
    edited_timestamp: m.edited_timestamp ?? null,
    user: m.author?.id ? user(m.author.id) : null,
  });
};

const onDelete = (a: Raw) => {
  const ids: unknown[] = a.ids ?? (a.id ? [a.id] : []);
  for (const id of ids) push("message_delete", a.channelId, a.guildId, { message_id: String(id) });
};

const onVoice = (a: Raw) => {
  for (const v of (a.voiceStates ?? []) as Raw[]) {
    const uid = String(v.userId);
    const prev = voice.get(uid);
    const now = v.channelId ? String(v.channelId) : null;
    const flags = flagsOf(v);
    const who = user(uid);
    if ((prev?.channel ?? null) !== now) {
      if (prev?.channel && now) {
        push("voice_move", now, v.guildId, { user: who, from_channel_id: prev.channel, from_channel: channel(prev.channel) });
      } else if (now) {
        push("voice_join", now, v.guildId, { user: who, flags });
      } else if (prev?.channel) {
        push("voice_leave", prev.channel, v.guildId, { user: who });
      }
    } else if (now && prev && prev.flags !== flags) {
      push("voice_state", now, v.guildId, { user: who, flags });
    }
    if (now) voice.set(uid, { channel: now, flags });
    else voice.delete(uid);
  }
};

const onTyping = (a: Raw) => push("typing", a.channelId, a.guildId, { user: user(a.userId) });

function seedPresence() {
  lastPresence.clear();
  try {
    const store = getStore("PresenceStore");
    const ids = (getStore("RelationshipStore").getFriendIDs?.() ?? []) as string[];
    for (const id of ids.slice(0, 5000)) {
      const acts = ((store.getActivities?.(id) ?? []) as Raw[]).map(activityBrief);
      lastPresence.set(String(id), { status: String(store.getStatus?.(id) ?? "offline"), acts: new Set(acts.map(activityKey)) });
    }
  } catch {
    // без исходного состояния первое событие человека покажет всё его текущее как новое
  }
}

const onPresence = (a: Raw) => {
  if (!wanted.has("presence")) return;
  const updates: Raw[] = a.updates ?? (a.user ? [a] : []);
  for (const u of updates) {
    const uid = String(u.user?.id ?? u.userId ?? "");
    if (!uid) continue;
    const status = String(u.status ?? "offline");
    const acts = ((u.activities ?? []) as Raw[]).map(activityBrief);
    const keys = new Set(acts.map(activityKey));
    const prev = lastPresence.get(uid);
    lastPresence.set(uid, { status, acts: keys });
    const changes = [
      ...acts.filter((x) => !prev?.acts.has(activityKey(x))).map((x) => ({ change: "started", type: x.type, name: x.name })),
      ...[...(prev?.acts ?? [])].filter((k) => !keys.has(k)).map((k) => ({ change: "stopped", type: k.split(":")[0], name: k.slice(k.indexOf(":") + 1) })),
    ];
    // Обновление только деталей (песня, уровень) без смены статуса и занятия не событие.
    if (prev && prev.status === status && !changes.length) continue;
    const ev = { user: user(uid), status, prev_status: prev?.status ?? null, client_status: u.clientStatus ?? {}, activities: acts, changes };
    if (presenceFilters && !presenceFilters.some((f) => presenceMatches(f, ev))) continue;
    push("presence", null, u.guildId, ev);
  }
};

const TYPES: [string, (a: Raw) => void][] = [
  ["MESSAGE_CREATE", onMessage],
  ["MESSAGE_UPDATE", onEdit],
  ["MESSAGE_DELETE", onDelete],
  ["MESSAGE_DELETE_BULK", onDelete],
  ["VOICE_STATE_UPDATES", onVoice],
  ["TYPING_START", onTyping],
  ["PRESENCE_UPDATES", onPresence],
  ["PRESENCE_UPDATE", onPresence],
];

function attach(d: Dispatcher) {
  if (handlers.size) return;
  seedVoice();
  for (const [type, fn] of TYPES) {
    handlers.set(type, fn);
    d.subscribe(type, fn);
  }
}

function detach() {
  const d = dispatcher;
  for (const [type, fn] of handlers) {
    try {
      d?.unsubscribe(type, fn);
    } catch {
      // страница могла уже сменить диспетчер
    }
  }
  handlers.clear();
  lastPresence.clear();
  queue = [];
}

/** Включает нужные виды событий (пустой список выключает всё). Можно вызывать сколько угодно раз. */
export function watchSet(p: { kinds?: string[]; presence?: PresenceFilter[] | null }) {
  const kinds = (p.kinds ?? []).filter((k) => KINDS.includes(k));
  const hadPresence = wanted.has("presence");
  wanted = new Set(kinds);
  presenceFilters = Array.isArray(p.presence) ? p.presence : null;
  if (wanted.has("presence") && !hadPresence) seedPresence();
  if (!wanted.size) {
    detach();
    return { watching: [] as string[], dispatcher: !!findDispatcher() };
  }
  const d = findDispatcher();
  if (!d) throw new Error("the client's event dispatcher was not found (Discord internals changed?)");
  attach(d);
  return { watching: [...wanted], dispatcher: true };
}

/** Забирает накопленные события. `dropped` — сколько не поместилось с прошлого раза. */
export function watchDrain() {
  const events = queue;
  queue = [];
  const lost = dropped;
  dropped = 0;
  return { events, dropped: lost, watching: [...wanted] };
}
