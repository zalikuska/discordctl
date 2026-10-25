// Трансляция окна или экрана. Запуск обходит окно выбора источника в Discord, но не проверки клиента:
// мы вызываем ту же функцию, которую клиент зовёт после выбора (startStreamWithSource), и она сама проверяет
// вход в голос, право на запись экрана и ограничения качества (без Nitro остаётся 720p30).
// Трансляция показывает чужим людям содержимое экрана: права и подтверждение задаёт демон, звук по умолчанию выключен.

import { describeChannel } from "./directory";
import { findFnBySource, requireFns } from "./modules";
import { getStore } from "./stores";
import { voiceStatus } from "./voice";

type Raw = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const store = (name: string, method: string, ...args: unknown[]): any => { // eslint-disable-line @typescript-eslint/no-explicit-any
  const s = getStore(name);
  const fn = s[method];
  if (typeof fn !== "function") throw new Error(`${name}.${method} is missing (Discord internals changed?)`);
  return fn.apply(s, args);
};

async function until(cond: () => boolean, ms: number): Promise<boolean> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try {
      if (cond()) return true;
    } catch {
      // состояние ещё не готово
    }
    await sleep(300);
  }
  return false;
}

const kindOf = (id: string) => (id.startsWith("screen:") ? "screen" : "window");

/** Источники захвата так, как их отдаёт мост клиента с Electron (тот же вызов, что у окна выбора). Объекты целиком, с миниатюрами. */
async function rawSources(thumbs = false): Promise<Raw[]> {
  const native = (window as unknown as { DiscordNative?: Raw }).DiscordNative;
  const fn = native?.desktopCapture?.getDesktopCaptureSources;
  if (!native || typeof fn !== "function") throw new Error("DiscordNative.desktopCapture.getDesktopCaptureSources is missing (Discord internals changed?)");
  const size = thumbs ? { width: 150, height: 150 } : { width: 0, height: 0 };
  return (await fn.call(native.desktopCapture, { types: ["screen", "window"], thumbnailSize: size })) as Raw[];
}

export async function streamSources() {
  return (await rawSources()).map((s) => ({ id: String(s.id), name: String(s.name ?? ""), kind: kindOf(String(s.id)) }));
}

function currentStream(): Raw | null {
  return store("ApplicationStreamingStore", "getCurrentUserActiveStream") ?? null;
}

export function streamStatus() {
  const s = currentStream();
  const ch = s ? store("ChannelStore", "getChannel", s.channelId) : null;
  return { streaming: !!s, channel: ch ? describeChannel(ch) : null, voice: voiceStatus().channel };
}

/** Ключ трансляции, как его строит клиент: `guild:<сервер>:<канал>:<владелец>` или `call:<канал>:<владелец>`. */
function streamKey(s: Raw): string {
  return s.streamType === "guild" ? `guild:${s.guildId}:${s.channelId}:${s.ownerId}` : `call:${s.channelId}:${s.ownerId}`;
}

export async function startStream(p: { source: string; audio?: boolean; dry_run?: boolean }) {
  const list = await rawSources();
  const src = list.find((s) => String(s.id) === p.source);
  if (!src) throw new Error(`source "${p.source}" not found (use the list of sources; window ids change when windows close)`);
  const plan = { source: { id: String(src.id), name: String(src.name ?? ""), kind: kindOf(String(src.id)) }, audio: !!p.audio };
  if (p.dry_run) return { dry_run: true, ...plan, ...streamStatus() };
  if (currentStream()) throw new Error("a stream is already running; stop it first");
  if (!voiceStatus().connected) throw new Error("not in a voice channel: join one first (the stream goes to the channel you are in)");
  const start = findFnBySource(['location:"startStreamWithSource"', "Starting stream for source id"]);
  if (!start) throw new Error("client function startStreamWithSource not found (Discord internals changed?)");
  const result = (await start(src, { soundshareEnabled: !!p.audio })) as [boolean, string | undefined];
  if (!Array.isArray(result) || result[0] !== true) throw new Error(`the client refused to start the stream: ${result?.[1] ?? "unknown reason"}`);
  if (!(await until(() => !!currentStream(), 15000))) throw new Error("the stream did not appear (the client accepted the request but nothing started)");
  return { started: true, ...plan, ...streamStatus() };
}

export async function stopStream(p: { dry_run?: boolean }) {
  const s = currentStream();
  if (p.dry_run) return { dry_run: true, would_stop: !!s, ...streamStatus() };
  if (!s) return { stopped: false, reason: "no stream is running", ...streamStatus() };
  const stop = findFnBySource(['type:"STREAM_STOP"']);
  if (!stop) throw new Error("client function that stops streams not found (Discord internals changed?)");
  // Как это делает клиент: сначала сбрасываем источник захвата, потом останавливаем трансляцию.
  requireFns(["toggleSelfMute", "setGoLiveSource"]).setGoLiveSource!(null);
  stop(streamKey(s), true, true);
  if (!(await until(() => !currentStream(), 10000))) throw new Error("the stream is still running after stop");
  return { stopped: true, ...streamStatus() };
}
