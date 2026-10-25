// История канала через HTTP-модуль клиента и стабильная схема сообщения.
// Текст сообщений — недоверенные данные чужих людей: интерфейсы (MCP) обязаны так их и помечать.

import { apiGet } from "./http";
import { describeChannel, resolveChannel, userBrief } from "./directory";
import { getStore } from "./stores";

type Raw = Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any

const PAGE = 100;
const MAX_TOTAL = 2000;

function author(a: Raw | undefined) {
  if (!a) return null;
  return userBrief({ id: a.id, username: a.username, global_name: a.global_name, bot: a.bot });
}

/** Сырой JSON Discord → наша схема (версия схемы в поле `schema` ответа). */
export function toMessage(m: Raw) {
  const ref = m.message_reference as Raw | undefined;
  const replied = m.referenced_message as Raw | null | undefined;
  return {
    id: String(m.id),
    channel_id: String(m.channel_id),
    author: author(m.author),
    content: String(m.content ?? ""),
    timestamp: m.timestamp ?? null,
    edited_timestamp: m.edited_timestamp ?? null,
    type: m.type ?? 0,
    pinned: !!m.pinned,
    reply_to: ref?.message_id
      ? {
          message_id: String(ref.message_id),
          author: replied ? author(replied.author) : null,
          content_preview: replied ? String(replied.content ?? "").slice(0, 200) : null,
        }
      : null,
    attachments: ((m.attachments as Raw[]) ?? []).map((a) => ({
      id: String(a.id),
      filename: String(a.filename),
      size: a.size ?? null,
      content_type: a.content_type ?? null,
      url: a.url ?? null,
    })),
    embeds: ((m.embeds as Raw[]) ?? []).map((e) => ({
      type: e.type ?? null,
      title: e.title ?? null,
      description: e.description ?? null,
      url: e.url ?? null,
    })),
    reactions: ((m.reactions as Raw[]) ?? []).map((r) => ({
      emoji: r.emoji?.id ? `${r.emoji.name}:${r.emoji.id}` : String(r.emoji?.name ?? ""),
      count: r.count ?? 0,
      me: !!r.me,
    })),
    mentions: ((m.mentions as Raw[]) ?? []).map((u) => String(u.id)),
    stickers: ((m.sticker_items as Raw[]) ?? []).map((s) => ({ id: String(s.id), name: String(s.name) })),
  };
}

export interface ReadParams {
  channel: string;
  limit?: number;
  before?: string;
  after?: string;
}

/**
 * Читает историю. Без `after` идёт назад от `before` (или от конца), с `after` вперёд.
 * Результат всегда от старых к новым. `next` — курсор для следующей страницы в том же направлении.
 */
export async function readMessages(p: ReadParams) {
  const ch = resolveChannel(p.channel);
  const me = getStore("UserStore").getCurrentUser?.() as Raw | undefined;
  if (!me) throw new Error("not logged in yet");
  const want = Math.min(Math.max(1, Math.floor(p.limit ?? 50)), MAX_TOTAL);
  const forward = !!p.after;
  let cursor = forward ? p.after : p.before;
  const got: Raw[] = [];
  let hasMore = true;
  while (got.length < want && hasMore) {
    const limit = Math.min(PAGE, want - got.length);
    const query: Record<string, unknown> = { limit };
    if (cursor) query[forward ? "after" : "before"] = cursor;
    const page = (await apiGet(String(me.id), `/channels/${ch.id}/messages`, query)) as Raw[];
    if (!Array.isArray(page)) throw new Error("unexpected response from the messages endpoint");
    if (page.length < limit) hasMore = false;
    if (!page.length) break;
    got.push(...page);
    // API отдаёт от новых к старым и для `after`; следующий курсор: самый новый (вперёд) или самый старый (назад).
    const ids = page.map((m) => BigInt(m.id));
    const edge = forward ? ids.reduce((a, b) => (a > b ? a : b)) : ids.reduce((a, b) => (a < b ? a : b));
    cursor = String(edge);
  }
  got.sort((a, b) => (BigInt(a.id) < BigInt(b.id) ? -1 : 1));
  return {
    schema: 1,
    channel: describeChannel(ch),
    direction: forward ? "after" : "before",
    messages: got.map(toMessage),
    has_more: hasMore,
    next: got.length ? cursor : null,
  };
}
