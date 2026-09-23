// 介面字串。
//
// 要加一種語言：在 locales/ 放一個檔案，並在 BUNDLES 與 LANGUAGES 各登記一筆。
// 缺的 key 會退回英文。
//
// 副檔名必須是 .svelte.js：裡面用了 $state，Svelte 5 只在 .svelte.js / .svelte.ts
// 裡編譯 rune。
import en from "./locales/en.js";
import zhHant from "./locales/zh-Hant.js";

const BUNDLES = {
  en,
  "zh-Hant": zhHant,
};

/// 缺 key 時退回這裡。en.js 必須是完整的。
const FALLBACK = "en";

export const LANGUAGES = [
  ["zh-Hant", "繁體中文"],
  ["en", "English"],
];

/// 把 navigator.language（zh-TW、zh-Hant-HK、en-US…）對到我們有的語言包。
function detect() {
  const tags = navigator.languages?.length ? navigator.languages : [navigator.language ?? ""];
  for (const tag of tags) {
    const lower = tag.toLowerCase();
    // zh-TW / zh-HK / zh-MO 都是繁體；zh-CN 沒有語言包，落到英文
    if (lower.startsWith("zh-hant") || /^zh-(tw|hk|mo)/.test(lower)) return "zh-Hant";
    const base = lower.split("-")[0];
    if (BUNDLES[base]) return base;
    if (BUNDLES[tag]) return tag;
  }
  return FALLBACK;
}

let lang = $state(detect());

/// `code` 是 "auto" 或語言代碼。認不得的值當成 auto。
export function setLanguage(code) {
  lang = code && code !== "auto" && BUNDLES[code] ? code : detect();
  document.documentElement.lang = lang;
}

export function currentLanguage() {
  return lang;
}

/// 取一句字串。`{name}` 會被 vars.name 取代。
export function t(key, vars) {
  const text = BUNDLES[lang]?.[key] ?? BUNDLES[FALLBACK][key] ?? key;
  if (!vars) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name) =>
    vars[name] === undefined ? whole : String(vars[name]),
  );
}

/// 後端錯誤依 kind 轉成一句話。
export function describeError(e) {
  if (!e || typeof e !== "object") return String(e ?? t("error.unknown"));
  switch (e.kind) {
    case "Locked":
      return t("error.locked");
    case "SecretsLocked":
      return t("error.secretsLocked");
    case "WrongPassword":
      return t("error.wrongPassword");
    case "NotFound":
      return t("error.notFound");
    case "Validation":
      // message 是後端給的代碼（password-empty 之類）。沒有對應翻譯時顯示「field: message」原文。
      return (e.detail ?? [])
        .map((v) => {
          const key = `valid.${v.message}`;
          const text = t(key);
          return text === key ? `${v.field}: ${v.message}` : text;
        })
        .join("、");
    case "Template":
      return t("error.template", { message: e.detail?.message ?? "" });
    case "VaultIo":
      return t("error.vaultIo", { message: e.detail?.message ?? "" });
    case "NotPasswordExport":
      return t("error.notPasswordExport");
    case "WorkspaceNotFound":
      return t("error.workspaceNotFound");
    case "WorkspaceNotEmpty":
      return t("error.workspaceNotEmpty", { n: e.detail?.count ?? 0 });
    case "LastWorkspace":
      return t("error.lastWorkspace");
    case "WorkspaceName":
      return t(`error.workspaceName.${e.detail?.reason ?? "empty"}`);
    case "VaultVerifyFailed":
      return t("error.verifyFailed");
    case "SendFailed":
      return t(e.detail?.clipboard ? "error.sendFailedClipboard" : "error.sendFailed", {
        reason: e.detail?.reason ?? "",
      });
    default:
      // Other：後端傳上來的技術訊息沒有翻譯，原樣顯示
      return e.detail?.message ?? t("error.unknown");
  }
}
