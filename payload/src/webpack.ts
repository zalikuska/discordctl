// Перехват webpack runtime Discord. Скрипт выполняется до кода страницы (addScriptToEvaluateOnNewDocument).

export interface WebpackRequire {
  c: Record<string, { exports?: unknown } | undefined>;
}

const GLOBAL = "webpackChunkdiscord_app";
const runtimes: WebpackRequire[] = [];
/** Сколько раз Discord заменял push; нужно для диагностики «хук не сработал». */
let pushReplacements = 0;

function onRuntime(req: WebpackRequire | undefined): void {
  if (req?.c && !runtimes.includes(req)) runtimes.push(req);
}

function hookArray(arr: unknown): void {
  if (!Array.isArray(arr)) return;
  let current = arr.push;
  Object.defineProperty(arr, "push", {
    configurable: true,
    get: () => current,
    set(fn: typeof Array.prototype.push) {
      // Runtime Discord заменил push: проталкиваем свой чанк и получаем его __webpack_require__.
      current = fn;
      pushReplacements++;
      try {
        fn.call(arr, [[Symbol("discordctl")], {}, onRuntime]);
      } catch {
        // чужой push не должен ломать загрузку страницы
      }
    },
  });
}

export function installWebpackHook(): void {
  let chunk: unknown;
  Object.defineProperty(window, GLOBAL, {
    configurable: true,
    get: () => chunk,
    set(v: unknown) {
      chunk = v;
      hookArray(v);
    },
  });
}

/** Основной runtime — тот, в чьём кэше больше всего модулей (побочные ~100). Выбираем в момент запроса: кэш растёт до конца загрузки. */
export function mainRuntime(): { req: WebpackRequire; modules: number } | null {
  let best: { req: WebpackRequire; modules: number } | null = null;
  for (const req of runtimes) {
    const modules = Object.keys(req.c).length;
    if (!best || modules > best.modules) best = { req, modules };
  }
  return best;
}

export function webpackInfo() {
  return { runtimes: runtimes.length, push_replacements: pushReplacements };
}
