// Голос: состояние, вход и выход, mute и deafen. Звук никуда не передаётся и не читается: только управление клиентом.
// Вход в личный звонок без активного разговора звонит людям, поэтому требует явного `ring`.

import { describeChannel, guildChannels, resolveChannel, resolveGuild, userBrief, userName } from "./directory";
import { requireFns } from "./modules";
import { getStore } from "./stores";

type Raw = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const store = (name: string, method: string, ...args: unknown[]): any => { // eslint-disable-line @typescript-eslint/no-explicit-any
  const s = getStore(name);
  const fn = s[method];
  if (typeof fn !== "function") throw new Error(`${name}.${method} is missing (Discord internals changed?)`);
  return fn.apply(s, args);
};

/** Ждёт, пока условие станет истинным (клиент применяет действия асинхронно). */
async function until(cond: () => boolean, ms: number): Promise<boolean> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try {
      if (cond()) return true;
    } catch {
      // состояние ещё не готово
    }
    await sleep(250);
  }
  try {
    return cond();
  } catch {
    return false;
  }
}

const VOICE_KINDS = new Set([1, 2, 3, 13]); // ЛС, голосовой, группа, трибуна

function participants(channelId: string) {
  const states = (store("VoiceStateStore", "getVoiceStatesForChannel", channelId) ?? {}) as Record<string, Raw>;
  return Object.values(states).map((v) => {
    const u = store("UserStore", "getUser", v.userId);
    return {
      user: userBrief(u) ?? { id: String(v.userId), username: null, display_name: userName(u), bot: false },
      muted: !!(v.mute || v.selfMute),
      deafened: !!(v.deaf || v.selfDeaf),
      streaming: !!v.selfStream,
      video: !!v.selfVideo,
    };
  });
}

/**
 * Кто сейчас в голосе. С `channel`: участники этого канала (голосового, трибуны или звонка в чате). С `guild`: все голосовые
 * каналы сервера, в которых кто-то есть. Данные берутся из состояния клиента, ничего не запрашивается и не меняется.
 */
export function voiceWho(p: { channel?: string; guild?: string }) {
  if (p.channel) {
    const ch = resolveChannel(p.channel, { voice: true });
    const info = describeChannel(ch);
    if (!VOICE_KINDS.has(ch.type)) throw new Error(`${info.kind} "${info.name}" is not a voice channel`);
    return { channel: info, participants: participants(String(ch.id)) };
  }
  if (!p.guild) throw new Error("give a voice channel (Server/channel) or a server name");
  const g = resolveGuild(p.guild);
  const voice = guildChannels(String(g.id)).filter((c) => c.type === 2 || c.type === 13);
  const occupied = voice
    .map((c) => ({ channel: describeChannel(c), participants: participants(String(c.id)) }))
    .filter((c) => c.participants.length > 0);
  return {
    guild: { id: String(g.id), name: String(g.name) },
    voice_channels: voice.length,
    people_in_voice: occupied.reduce((n, c) => n + c.participants.length, 0),
    channels: occupied,
  };
}

export function voiceStatus() {
  const channelId: string | null = store("SelectedChannelStore", "getVoiceChannelId") ?? store("RTCConnectionStore", "getChannelId") ?? null;
  const ch = channelId ? store("ChannelStore", "getChannel", channelId) : null;
  return {
    connected: !!channelId && !!store("RTCConnectionStore", "isConnected"),
    state: String(store("RTCConnectionStore", "getState") ?? "unknown"),
    channel: ch ? describeChannel(ch) : null,
    self_mute: !!store("MediaEngineStore", "isSelfMute"),
    self_deaf: !!store("MediaEngineStore", "isSelfDeaf"),
    participants: channelId ? participants(String(channelId)) : [],
  };
}

export async function joinVoice(p: { channel: string; ring?: boolean; dry_run?: boolean }) {
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
  await actions.selectVoiceChannel!(id);
  const ok = await until(() => String(store("RTCConnectionStore", "getChannelId")) === id && !!store("RTCConnectionStore", "isConnected"), 20000);
  if (!ok) {
    throw new Error("the client did not connect (the channel may need a confirmation in the Discord window, or you lack the permission to connect)");
  }
  return { joined: true, already: false, ...voiceStatus() };
}

export async function leaveVoice(p: { dry_run?: boolean }) {
  const cur = voiceStatus();
  if (p.dry_run) return { dry_run: true, would_leave: cur.channel };
  if (!cur.channel) return { left: false, reason: "not in a voice channel", ...cur };
  const actions = requireFns(["selectVoiceChannel", "disconnect"]);
  await actions.disconnect!();
  const ok = await until(() => !store("SelectedChannelStore", "getVoiceChannelId"), 10000);
  if (!ok) throw new Error("the client is still connected after disconnect");
  // После остановки трансляции клиент может переподключиться к каналу и «отменить» выход: проверяем ещё раз через паузу.
  await sleep(1500);
  if (store("SelectedChannelStore", "getVoiceChannelId")) {
    await actions.disconnect!();
    if (!(await until(() => !store("SelectedChannelStore", "getVoiceChannelId"), 10000))) throw new Error("the client keeps reconnecting after disconnect");
  }
  return { left: true, ...voiceStatus() };
}

/** Приводит mute или deafen к нужному значению. Переключатель клиента, поэтому нажимаем только если состояние отличается. */
async function setFlag(kind: "mute" | "deaf", want: boolean, dry: boolean) {
  const read = () => !!store("MediaEngineStore", kind === "mute" ? "isSelfMute" : "isSelfDeaf");
  if (dry) return { dry_run: true, [kind]: want, now: read() };
  if (read() === want) return { changed: false, ...voiceStatus() };
  const actions = requireFns(["toggleSelfMute", "toggleSelfDeaf"]);
  const toggle = () => (kind === "mute" ? actions.toggleSelfMute!() : actions.toggleSelfDeaf!());
  await toggle();
  // Если медиадвижок был выключен, первый вызов лишь включает его: пробуем ещё раз, но только один.
  if (!(await until(() => read() === want, 1500))) {
    await toggle();
    if (!(await until(() => read() === want, 3000))) throw new Error(`could not change self ${kind}`);
  }
  return { changed: true, ...voiceStatus() };
}

export const setMute = (mute: boolean, dry = false) => setFlag("mute", mute, dry);
export const setDeafen = (deafen: boolean, dry = false) => setFlag("deaf", deafen, dry);
