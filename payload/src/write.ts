// Запись: отправка, ответ, правка, удаление, реакции. Все запросы идут через общий HTTP-слой с лимитом частоты.
// Права и подтверждения проверяет демон (шаг 7), payload только исполняет. `dry_run` показывает запрос и ничего не шлёт.

import { apiRequest } from "./http";
import { describeChannel, resolveChannel } from "./directory";
import { toMessage } from "./messages";
import { getStore } from "./stores";

type Raw = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

const MAX_CONTENT = 4000; // с Nitro 4000, без него 2000: точный предел проверит сервер
const isId = (s: string) => /^\d{15,25}$/.test(s);

const DISCORD_EPOCH = 1420070400000n;
/** Nonce, как у настоящего клиента: снежинка по текущему времени. */
const nonce = () => String(((BigInt(Date.now()) - DISCORD_EPOCH) << 22n) | BigInt(Math.floor(Math.random() * 4194304)));

function me(): string {
  const u = getStore("UserStore").getCurrentUser?.() as Raw | undefined;
  if (!u) throw new Error("not logged in yet");
  return String(u.id);
}

function checkContent(content: unknown): string {
  if (typeof content !== "string" || !content.trim()) throw new Error("message content is empty");
  if (content.length > MAX_CONTENT) throw new Error(`message is ${content.length} characters, the limit is ${MAX_CONTENT}`);
  return content;
}

function checkId(value: unknown, what: string): string {
  const s = String(value ?? "");
  if (!isId(s)) throw new Error(`${what} must be a message id (digits), got "${s}"`);
  return s;
}

/** По умолчанию никого не пингуем (ни @everyone, ни ответ автору): текст приходит от агента, а не от человека. */
const allowedMentions = (mentions: boolean) => (mentions ? { parse: ["users"], replied_user: true } : { parse: [], replied_user: false });

interface Common {
  channel: string;
  dry_run?: boolean;
}

async function run(c: Common, method: "post" | "patch" | "put" | "delete", url: string, body?: unknown, wrap?: (r: unknown) => Raw): Promise<Raw> {
  const uid = me();
  if (c.dry_run) return { dry_run: true, request: { method: method.toUpperCase(), url, body: body ?? null } };
  const res = await apiRequest(uid, method, url, { body });
  return wrap ? wrap(res) : { result: res };
}

export async function sendMessage(p: Common & { content: string; reply_to?: string; mentions?: boolean }) {
  const content = checkContent(p.content);
  const ch = resolveChannel(p.channel);
  const body: Raw = { content, nonce: nonce(), tts: false, flags: 0, allowed_mentions: allowedMentions(!!p.mentions) };
  if (p.reply_to) {
    const ref: Raw = { message_id: checkId(p.reply_to, "reply_to"), channel_id: String(ch.id) };
    if (ch.guild_id) ref.guild_id = String(ch.guild_id);
    body.message_reference = ref;
  }
  const out = await run(p, "post", `/channels/${ch.id}/messages`, body, (r) => ({ sent: true, message: toMessage(r as Raw) }));
  return { channel: describeChannel(ch), ...out };
}

export async function editMessage(p: Common & { message: string; content: string; mentions?: boolean }) {
  const content = checkContent(p.content);
  const ch = resolveChannel(p.channel);
  const id = checkId(p.message, "message");
  const body = { content, allowed_mentions: allowedMentions(!!p.mentions) };
  const out = await run(p, "patch", `/channels/${ch.id}/messages/${id}`, body, (r) => ({ edited: true, message: toMessage(r as Raw) }));
  return { channel: describeChannel(ch), ...out };
}

export async function deleteMessage(p: Common & { message: string }) {
  const ch = resolveChannel(p.channel);
  const id = checkId(p.message, "message");
  const out = await run(p, "delete", `/channels/${ch.id}/messages/${id}`, undefined, () => ({ deleted: true, message_id: id }));
  return { channel: describeChannel(ch), ...out };
}

/** Эмодзи: сам символ (👍) или `имя:id` для кастомных. */
export async function react(p: Common & { message: string; emoji: string; remove?: boolean }) {
  const emoji = String(p.emoji ?? "").trim();
  if (!emoji) throw new Error("emoji is empty");
  const ch = resolveChannel(p.channel);
  const id = checkId(p.message, "message");
  const url = `/channels/${ch.id}/messages/${id}/reactions/${encodeURIComponent(emoji)}/@me`;
  const out = await run(p, p.remove ? "delete" : "put", url, undefined, () => ({ reacted: !p.remove, emoji, message_id: id }));
  return { channel: describeChannel(ch), ...out };
}
