// Точка входа payload. Выполняется до кода страницы Discord.

import { call, status } from "./methods";
import { installWebpackHook } from "./webpack";

if (!("__discordctl" in window)) {
  installWebpackHook();
  // Не перечисляется, чтобы не светиться в `Object.keys(window)`.
  Object.defineProperty(window, "__discordctl", {
    value: Object.freeze({ status: () => status(), call }),
    enumerable: false,
  });
}
