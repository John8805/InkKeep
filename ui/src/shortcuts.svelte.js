// 搜尋視窗的快捷鍵。
//
// 組合鍵的字串格式跟全域快捷鍵一樣：修飾鍵依 Ctrl、Alt、Shift、Super 的順序，
// 最後是按鍵，按鍵名稱取自 `KeyboardEvent.code`（`KeyK` 寫成 `K`、`Digit1` 寫成 `1`）。
// 用實體按鍵位置而不是輸入的字元，中文輸入法開著時一樣認得。
//
// 副檔名必須是 .svelte.js：裡面用了 $state。

/// 所有動作與預設組合鍵，順序就是設定頁的顯示順序。空字串表示預設不指定。
///
/// `kinds` 是這個動作適用的項目類別，沒寫就是全部。同一組合鍵可以給好幾個動作，
/// 按下時依選中項目的類別挑適用的；都適用時取排在前面的。
export const ACTIONS = [
  { id: "send", default: "Enter", kinds: ["snippet", "password"] },
  { id: "sendBookmark", default: "Enter", kinds: ["bookmark"] },
  { id: "openBookmark", default: "Ctrl+Enter", kinds: ["bookmark"] },
  { id: "copy", default: "Shift+Enter" },
  { id: "kindNext", default: "Tab" },
  { id: "kindPrev", default: "" },
  { id: "workspaceNext", default: "Tab+ArrowDown" },
  { id: "workspacePrev", default: "Tab+ArrowUp" },
  { id: "selectDown", default: "ArrowDown" },
  { id: "selectUp", default: "ArrowUp" },
  { id: "pageDown", default: "PageDown" },
  { id: "pageUp", default: "PageUp" },
  { id: "newItem", default: "Ctrl+N" },
  { id: "editItem", default: "Ctrl+E" },
  { id: "settings", default: "Ctrl+Comma" },
  { id: "lock", default: "Ctrl+L" },
  { id: "close", default: "Escape" },
];

const DEFAULTS = Object.fromEntries(ACTIONS.map((a) => [a.id, a.default]));

/// settings.toml 的 [shortcuts]：只有使用者改過的動作
let overrides = $state({});

export function setOverrides(map) {
  overrides = { ...(map ?? {}) };
}

/// 目前生效的組合鍵，沒指定是空字串
export function binding(id) {
  return Object.hasOwn(overrides, id) ? overrides[id] : (DEFAULTS[id] ?? "");
}

/// 跟預設值不同的才寫進設定檔
export function overridesFor(bindings) {
  const out = {};
  for (const { id } of ACTIONS) {
    if ((bindings[id] ?? "") !== DEFAULTS[id]) out[id] = bindings[id] ?? "";
  }
  return out;
}

export function defaultBindings() {
  return { ...DEFAULTS };
}

const MODIFIER_CODES = new Set([
  "ShiftLeft",
  "ShiftRight",
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
]);

/// `e.key` 對應到 `e.code` 的名稱，給沒帶掃描碼的模擬按鍵用
const CODE_FROM_KEY = {
  " ": "Space",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "`": "Backquote",
  "-": "Minus",
  "=": "Equal",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  Esc: "Escape",
  Up: "ArrowUp",
  Down: "ArrowDown",
  Left: "ArrowLeft",
  Right: "ArrowRight",
};

const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta", "OS", "AltGraph"]);

/// 按鍵名稱。優先用 `e.code`；別的程式模擬的按鍵可能沒帶掃描碼，`code` 是空的，改從 `e.key` 推。
function keyName(e) {
  if (e.code && e.code !== "Unidentified") {
    return e.code.replace(/^Key([A-Z])$/, "$1").replace(/^Digit(\d)$/, "$1");
  }
  if (/^[a-z0-9]$/i.test(e.key)) return e.key.toUpperCase();
  return CODE_FROM_KEY[e.key] ?? e.key;
}

const MODIFIER_NAMES = new Set(["Ctrl", "Alt", "Shift", "Super"]);

/// 一般按鍵的名稱。只按了修飾鍵、正在組字、或認不出來時回 null。
export function keyOf(e) {
  if (e.isComposing || e.key === "Process" || e.key === "Unidentified") return null;
  if (MODIFIER_CODES.has(e.code) || MODIFIER_KEYS.has(e.key)) return null;
  return keyName(e) || null;
}

/// 事件當下按著的修飾鍵，依 Ctrl、Alt、Shift、Super 的順序
export function modifiersOf(e) {
  const parts = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Super");
  return parts;
}

/// 組合鍵拆成修飾鍵與一般按鍵。`Tab+ArrowRight` 的一般按鍵有兩個：按住 Tab 再按 →
export function splitCombo(combo) {
  const parts = combo ? combo.split("+") : [];
  return {
    mods: parts.filter((p) => MODIFIER_NAMES.has(p)),
    keys: parts.filter((p) => !MODIFIER_NAMES.has(p)),
  };
}

const ALL_KINDS = ["snippet", "bookmark", "password"];

/// 動作適用的類別
export function kindsOf(id) {
  return ACTIONS.find((a) => a.id === id)?.kinds ?? ALL_KINDS;
}

/// 用這個組合、而且適用於 kind 的第一個動作。kind 是 null（沒選中項目）時不看類別。
function actionOf(combo, kind) {
  return (
    ACTIONS.find(
      (a) => binding(a.id) === combo && (kind == null || kindsOf(a.id).includes(kind)),
    )?.id ?? null
  );
}

/// 有沒有哪個組合是「按住 key 再按別的鍵」
function isChordPrefix(mods, key) {
  const want = mods.join("+");
  return ACTIONS.some((a) => {
    const { mods: m, keys } = splitCombo(binding(a.id));
    return keys.length > 1 && keys[0] === key && m.join("+") === want;
  });
}

/// 搜尋視窗用的按鍵比對。記住目前按著哪些一般按鍵，才認得出「按住 Tab 再按 →」。
///
/// 某個鍵同時是單鍵快捷鍵、也是組合的開頭時（例如 Tab 單按換類別、Tab+→ 另有動作），
/// 單按的動作延到放開時才執行；放開前接了別的鍵就算組合，單按的動作不執行。
///
/// `currentKind()` 回傳目前選中項目的類別，用來在同一組合的多個動作中挑適用的。
export function createMatcher(currentKind = () => null) {
  let held = [];
  let pending = null;
  let chordUsed = false;

  return {
    /// 回傳 { action, prevent }：action 是要執行的動作，prevent 表示要擋掉預設行為
    keydown(e) {
      const key = keyOf(e);
      if (!key) return { action: null, prevent: false };
      const mods = modifiersOf(e);
      if (e.repeat && pending === key) return { action: null, prevent: true };
      if (!held.includes(key)) held.push(key);

      if (held.length > 1) {
        const action = actionOf([...mods, ...held].join("+"), currentKind());
        if (action) chordUsed = true;
        return { action, prevent: action !== null };
      }
      if (isChordPrefix(mods, key)) {
        pending = key;
        chordUsed = false;
        return { action: null, prevent: true };
      }
      const action = actionOf([...mods, key].join("+"), currentKind());
      return { action, prevent: action !== null };
    },

    /// 放開延後的那個鍵、而且中間沒接別的鍵時，回傳它單按的動作
    keyup(e) {
      const key = keyOf(e);
      if (!key) return null;
      held = held.filter((k) => k !== key);
      if (pending !== key) return null;
      pending = null;
      return chordUsed ? null : actionOf([...modifiersOf(e), key].join("+"), currentKind());
    },

    /// 視窗失焦或被別的畫面蓋住時呼叫：放開的事件可能收不到
    reset() {
      held = [];
      pending = null;
      chordUsed = false;
    },
  };
}

/// 會打出字的按鍵：不搭配 Ctrl、Alt、Super 就指定成快捷鍵的話，打字時會被攔走
export function typesText(combo) {
  const { mods, keys } = splitCombo(combo);
  if (mods.some((m) => m !== "Shift")) return false;
  return keys.some((k) => /^[A-Z0-9]$/.test(k) || PRINTABLE.has(k) || k.startsWith("Numpad"));
}

const PRINTABLE = new Set([
  "Space",
  "Comma",
  "Period",
  "Slash",
  "Semicolon",
  "Quote",
  "Backquote",
  "Minus",
  "Equal",
  "BracketLeft",
  "BracketRight",
  "Backslash",
  "IntlBackslash",
  "IntlRo",
  "IntlYen",
]);

const KEY_LABELS = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Semicolon: ";",
  Quote: "'",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Escape: "Esc",
  Super: "Win",
};

/// 給人看的寫法，例如 `Ctrl+Comma` → `Ctrl+,`
export function keyLabel(combo) {
  if (!combo) return "";
  return combo
    .split("+")
    .map((p) => KEY_LABELS[p] ?? p)
    .join("+");
}

/// 動作目前的組合鍵，給提示文字用；沒指定是空字串
export function labelOf(id) {
  return keyLabel(binding(id));
}
