<script>
  // 快捷鍵設定頁。點一下組合鍵再按下新的組合就改好，改完立刻存檔、立刻生效。
  import * as api from "./api.js";
  import { t } from "./i18n.svelte.js";
  import {
    ACTIONS,
    binding,
    defaultBindings,
    keyLabel,
    keyOf,
    modifiersOf,
    splitCombo,
    overridesFor,
    setOverrides,
    typesText,
  } from "./shortcuts.svelte.js";

  let { onback, onchanged } = $props();

  const DEFAULT_HOTKEY = "Alt+Period";
  const DEFAULTS = { ...defaultBindings(), global: DEFAULT_HOTKEY };

  let bindings = $state(Object.fromEntries(ACTIONS.map((a) => [a.id, binding(a.id)])));
  let hotkey = $state("");
  /// 正在錄製的動作 id；"global" 是全域快捷鍵；null 表示沒有在錄
  let recording = $state(null);
  /// 錄製中已經按下的一般按鍵（依按下順序）與最後一次按下時的修飾鍵
  let captured = $state({ keys: [], mods: [] });
  /// 定案後還按著的鍵：放開時不能讓按鈕把它當成點擊
  let lingering = new Set();
  let busy = $state(false);
  /// 顯示在某一列正下方的訊息：{ id, text, error, takeFrom?, combo? }
  /// takeFrom 有值時附「改給這個」按鈕，把 combo 從那個動作移過來
  let rowMsg = $state(null);
  /// 讀不到設定這類跟單一列無關的錯誤
  let pageError = $state(null);

  $effect(() => {
    api
      .settingsGet()
      .then((s) => (hotkey = s.hotkey))
      .catch((e) => (pageError = api.describeError(e)));
  });

  function nameOf(id) {
    return id === "global" ? t("shortcuts.global") : t(`shortcut.${id}`);
  }

  /// 已經用了這個組合的另一個動作
  function takenBy(id, combo) {
    if (id !== "global" && combo === hotkey) return "global";
    return ACTIONS.find((a) => a.id !== id && bindings[a.id] === combo)?.id ?? null;
  }

  /// 只改快捷鍵相關的欄位：設定頁上還沒按儲存的其他修改不會被一起存進去
  async function persist(nextBindings, nextHotkey, id) {
    busy = true;
    rowMsg = null;
    try {
      const s = await api.settingsGet();
      s.shortcuts = overridesFor(nextBindings);
      s.hotkey = nextHotkey;
      const saved = await api.settingsSet(s);
      bindings = nextBindings;
      hotkey = saved.hotkey;
      setOverrides(saved.shortcuts);
      if (id) rowMsg = { id, text: t("common.saved"), error: false };
      await onchanged?.(saved);
    } catch (e) {
      rowMsg = { id, text: api.describeError(e), error: true };
    } finally {
      busy = false;
    }
  }

  function startRecording(id) {
    rowMsg = null;
    captured = { keys: [], mods: [] };
    recording = id;
  }

  /// 錄製中所有按鍵都攔下來，包括 Esc 與 Tab，不交給設定視窗。
  /// 按下時只記錄，放開第一個一般按鍵時才定案：按住 Tab 再按 → 錄成 Tab+→。
  function onRecordKeydown(e, id) {
    if (recording !== id) return;
    e.preventDefault();
    e.stopPropagation();
    const key = keyOf(e);
    if (!key || e.repeat) return;
    captured = {
      keys: captured.keys.includes(key) ? captured.keys : [...captured.keys, key],
      mods: modifiersOf(e),
    };
  }

  function onRecordKeyup(e, id) {
    const key = keyOf(e);
    if (recording !== id) {
      if (key && lingering.delete(key)) e.preventDefault();
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (!key || !captured.keys.includes(key)) return;
    const combo = [...captured.mods, ...captured.keys].join("+");
    lingering = new Set(captured.keys.filter((k) => k !== key));
    captured = { keys: [], mods: [] };
    recording = null;
    trySet(id, combo);
  }

  function cancelRecording() {
    captured = { keys: [], mods: [] };
    recording = null;
  }

  /// 檢查能不能用，能用就存；不能用就在那一列下面說明原因
  function trySet(id, combo) {
    if (id === "global") {
      // Windows 的全域快捷鍵只能是修飾鍵加一個鍵
      const { mods, keys } = splitCombo(combo);
      if (keys.length > 1) {
        rowMsg = { id, text: t("shortcuts.globalOneKey"), error: true };
        return;
      }
      if (mods.length === 0 && !/^F\d+$/.test(keys[0])) {
        rowMsg = { id, text: t("shortcuts.globalNeedsModifier"), error: true };
        return;
      }
    } else if (typesText(combo)) {
      rowMsg = { id, text: t("shortcuts.needsModifier"), error: true };
      return;
    }
    const other = takenBy(id, combo);
    if (other) {
      rowMsg = {
        id,
        text: t("shortcuts.inUse", { combo: keyLabel(combo), name: nameOf(other) }),
        error: true,
        // 全域快捷鍵一定要有值，不能被搶走
        takeFrom: other === "global" ? null : other,
        combo,
      };
      return;
    }
    assign(id, combo);
  }

  function assign(id, combo, takeFrom = null) {
    const next = { ...bindings };
    if (takeFrom) next[takeFrom] = "";
    if (id === "global") persist(next, combo, id);
    else persist({ ...next, [id]: combo }, hotkey, id);
  }

  function resetOne(id) {
    rowMsg = null;
    if (!DEFAULTS[id]) clear(id);
    else trySet(id, DEFAULTS[id]);
  }

  function clear(id) {
    persist({ ...bindings, [id]: "" }, hotkey, id);
  }

  function resetAll() {
    persist(defaultBindings(), DEFAULT_HOTKEY, null);
  }
</script>

{#snippet row(id, value, clearable)}
  {@const isDefault = value === DEFAULTS[id]}
  <li>
    <span class="name">{nameOf(id)}</span>
    <button
      class="key mono"
      class:rec={recording === id}
      class:unset={!value && recording !== id}
      disabled={busy}
      onclick={() => startRecording(id)}
      onkeydown={(e) => onRecordKeydown(e, id)}
      onkeyup={(e) => onRecordKeyup(e, id)}
      onblur={() => recording === id && cancelRecording()}
    >
      {#if recording === id}
        {captured.keys.length
          ? `${keyLabel([...captured.mods, ...captured.keys].join("+"))}…`
          : t("shortcuts.recording")}
      {:else}
        {keyLabel(value) || t("shortcuts.unassigned")}
      {/if}
    </button>
    {#if recording === id}
      <!-- mousedown 不搶焦點，錄製狀態留到 click 才結束 -->
      <button class="cancel" onmousedown={(e) => e.preventDefault()} onclick={cancelRecording}>
        {t("common.cancel")}
      </button>
    {:else}
      <button
        class="icon"
        disabled={busy || isDefault}
        onclick={() => resetOne(id)}
        aria-label={t("shortcuts.resetOne")}
        title={t("shortcuts.resetOne")}>↺</button
      >
      {#if clearable}
        <button
          class="icon"
          disabled={busy || !value}
          onclick={() => clear(id)}
          aria-label={t("shortcuts.clear")}
          title={t("shortcuts.clear")}>✕</button
        >
      {:else}
        <span class="icon-gap"></span>
      {/if}
    {/if}
  </li>
  {#if rowMsg?.id === id}
    <li class="msg" class:error={rowMsg.error} role={rowMsg.error ? "alert" : "status"}>
      <span>{rowMsg.text}</span>
      {#if rowMsg.takeFrom}
        <button onclick={() => assign(id, rowMsg.combo, rowMsg.takeFrom)} disabled={busy}>
          {t("shortcuts.takeOver")}
        </button>
      {/if}
    </li>
  {/if}
{/snippet}

<!-- Tab 不在按鈕之間移動焦點。錄製中的 Tab 已在按鈕上攔下並錄成組合鍵，不會傳到這裡 -->
<div
  class="page"
  role="presentation"
  onkeydown={(e) => {
    if (e.key === "Tab") e.preventDefault();
  }}
>
  <div class="top">
    <button class="back" onclick={onback}>‹ {t("settings.title")}</button>
    <button onclick={resetAll} disabled={busy}>{t("shortcuts.reset")}</button>
  </div>

  <p class="muted small">{t("shortcuts.hint")}</p>

  <h2>{t("shortcuts.globalGroup")}</h2>
  <ul>
    {@render row("global", hotkey, false)}
  </ul>

  <h2>{t("shortcuts.searchGroup")}</h2>
  <ul>
    {#each ACTIONS as a (a.id)}
      {@render row(a.id, bindings[a.id], true)}
    {/each}
  </ul>

  {#if pageError}<p class="error" role="alert">{pageError}</p>{/if}
</div>

<style>
  .page {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .top {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .back {
    background: none;
    border: none;
    padding: 4px 0;
    color: var(--accent);
  }
  h2 {
    font-size: 0.85em;
    margin: 8px 0 0;
    color: var(--fg-dim);
    font-weight: 600;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
    padding: 0 6px 0 12px;
  }
  li + li {
    border-top: 1px solid var(--border);
  }
  /* 緊貼在被改的那一列下面，不畫分隔線 */
  li.msg {
    border-top: none;
    min-height: 0;
    padding: 0 12px 8px;
    gap: 10px;
    font-size: 0.85em;
    color: var(--accent);
  }
  li.msg.error {
    color: var(--danger);
  }
  li.msg button {
    padding: 2px 10px;
    font-size: 0.95em;
  }
  .name {
    flex: 1;
    min-width: 0;
  }
  .key {
    flex: none;
    min-width: 10em;
    padding: 3px 10px;
    text-align: center;
    font-size: 0.85em;
  }
  .key.rec {
    border-color: var(--accent);
    color: var(--accent);
  }
  .key.unset {
    color: var(--fg-dim);
  }
  .icon,
  .icon-gap {
    flex: none;
    width: 24px;
  }
  .icon {
    height: 24px;
    padding: 0;
    background: none;
    border: none;
    color: var(--fg-dim);
  }
  .icon:hover:not(:disabled) {
    color: var(--fg);
  }
  .icon:disabled {
    visibility: hidden;
  }
  /* 佔住 ↺ 與 ✕ 兩格的寬度，錄製時整列不跳動 */
  .cancel {
    flex: none;
    width: 56px;
    padding: 2px 0;
    font-size: 0.85em;
  }
  .small {
    font-size: 0.8em;
    margin: 0;
  }
  p.error {
    color: var(--danger);
    margin: 0;
    font-size: 0.85em;
  }
</style>
