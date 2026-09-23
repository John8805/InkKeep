<script>
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import * as api from "./api.js";
  import { t } from "./i18n.svelte.js";
  import InputForm from "./InputForm.svelte";
  import SecretPrompt from "./SecretPrompt.svelte";
  import FieldChoice from "./FieldChoice.svelte";
  import Editor from "./Editor.svelte";
  import Settings from "./Settings.svelte";

  let { vault, onvaultchanged, onpreferenceschanged } = $props();

  /// null = 不開；{ id } = 編輯；{ id: null } = 新增
  let editor = $state(null);
  let showSettings = $state(false);

  let query = $state("");
  /// 永遠有一個類別被選中，沒有「全部」
  let kindFilter = $state("snippet");
  const KIND_FILTERS = ["snippet", "bookmark", "password"];
  /// 工作區清單與目前選的。空字串是「全部」
  let workspaces = $state([]);
  let wsFilter = $state("");
  const wsName = $derived(Object.fromEntries(workspaces.map((w) => [w.id, w.name])));
  const showWsLabel = $derived(wsFilter === "" && workspaces.length > 1);
  let hits = $state([]);
  let cursor = $state(0);
  let toast = $state(null);
  let previewText = $state("");
  /// 密碼項目的預覽：帳號與網域（都沒加密），密碼本身只顯示遮罩
  let previewPw = $state(null);
  /// 非 null 時畫面切換成表單模式（原地切換，不開 modal）
  let pendingForm = $state(null);
  /// 非 null 時原地切換成主密碼提示。{ reason, retry }
  let secretPrompt = $state(null);
  /// 非 null 時原地問要送帳號還是密碼。{ item, alternate }
  let fieldChoice = $state(null);

  let searchBox;
  let listEl;
  let bodyEl;
  let debounceTimer;

  /// 清單佔的寬度百分比，剩下的給預覽
  let splitPct = $state(60);
  let draggingSplit = $state(false);

  function startSplit(e) {
    e.preventDefault();
    draggingSplit = true;
  }

  function onSplitMove(e) {
    if (!draggingSplit || !bodyEl) return;
    const rect = bodyEl.getBoundingClientRect();
    const pct = ((e.clientX - rect.left) / rect.width) * 100;
    splitPct = Math.min(85, Math.max(20, pct));
  }

  function endSplit() {
    draggingSplit = false;
  }

  const selected = $derived(hits[cursor] ?? null);

  async function runSearch() {
    try {
      hits = await api.search(query, 200, kindFilter, wsFilter || null);
      cursor = 0;
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  async function pickKind(kind) {
    kindFilter = kind;
    await runSearch();
    // 換完類別焦點要回到輸入框，不然接著打字會沒反應
    await focusSearch();
  }

  function cycleKind(delta) {
    const at = KIND_FILTERS.indexOf(kindFilter);
    const next = (at + delta + KIND_FILTERS.length) % KIND_FILTERS.length;
    pickKind(KIND_FILTERS[next]);
  }

  async function loadWorkspaces() {
    try {
      workspaces = await api.workspaceList();
      if (wsFilter && !workspaces.some((w) => w.id === wsFilter)) wsFilter = "";
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  async function pickWorkspace(id) {
    wsFilter = id;
    await runSearch();
    await focusSearch();
  }

  function onInput() {
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(runSearch, 30);
  }

  $effect(() => {
    const item = selected;
    previewPw = null;
    if (!item) {
      previewText = "";
      return;
    }
    let cancelled = false;
    if (item.kind === "password") {
      // item_get 不解密，拿到的只有帳號與網域，不需要主密碼
      api
        .itemGet(item.id)
        .then((full) => {
          if (!cancelled) previewPw = { username: full.username, url: full.url };
        })
        .catch(() => {});
      return () => {
        cancelled = true;
      };
    }
    api
      .preview(item.id, null)
      .then((text) => {
        if (!cancelled) previewText = text;
      })
      .catch(() => {
        if (!cancelled) previewText = "";
      });
    return () => {
      cancelled = true;
    };
  });

  async function focusSearch() {
    await tick();
    searchBox?.focus();
    searchBox?.select();
  }

  onMount(async () => {
    await loadWorkspaces();
    await runSearch();
    await focusSearch();
    const unlisten = await listen("window:shown", async () => {
      // 搜尋字串與類別過濾保留：視窗開著時按快捷鍵也會觸發 window:shown，
      // 清掉會吃掉打到一半的字。
      pendingForm = null;
      secretPrompt = null;
      fieldChoice = null;
      // 保險庫可能在別處被改過，重跑一次
      await loadWorkspaces();
      await runSearch();
      await focusSearch();
    });
    return unlisten;
  });

  /// 方向鍵到頭會繞回另一端；翻頁停在頭尾。
  function move(delta, wrap = true) {
    const n = hits.length;
    if (n === 0) return;
    const next = cursor + delta;
    cursor = wrap ? ((next % n) + n) % n : Math.min(Math.max(next, 0), n - 1);
    scrollIntoView();
  }

  async function scrollIntoView() {
    await tick();
    listEl?.querySelector("[aria-selected='true']")?.scrollIntoView({ block: "nearest" });
  }

  /// 要用到密碼就先問主密碼，解開之後重跑同一個動作。
  async function withSecret(reason, run) {
    if (!vault.secrets_locked) {
      try {
        return await run();
      } catch (e) {
        // 剛好在這個當口被閒置鎖定，退回去問
        if (e?.kind !== "SecretsLocked") throw e;
      }
    }
    secretPrompt = { reason, retry: run };
  }

  async function afterSecretUnlocked() {
    const pending = secretPrompt;
    secretPrompt = null;
    await onvaultchanged?.();
    try {
      await pending?.retry();
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  /// 密碼項目先問送哪個欄位，需要輸入就切表單，否則直接送出
  async function activate(field = "body", alternate = false) {
    if (!selected) return;
    const item = selected;

    // alternate 是換輸入法的逃生口，跳過欄位選擇直接送密碼。
    if (item.kind === "password" && field === "body" && item.has_username && !alternate) {
      fieldChoice = { item, alternate };
      return;
    }
    await sendField(item, field, alternate);
  }

  /// 送出指定欄位。密碼要主密碼，帳號不用——帳號沒有加密。
  async function sendField(item, field, alternate) {
    try {
      if (item.kind === "password" && field === "body") {
        await withSecret(t("secret.send", { title: item.title }), () =>
          doActivate(item, field, alternate),
        );
      } else {
        await doActivate(item, field, alternate);
      }
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  async function doActivate(item, field, alternate) {
    const plan = await api.prepareInsert(item.id);
    if (plan.needs_inputs.length > 0) {
      pendingForm = { item, plan, field, alternate };
      return;
    }
    await api.insert(item.id, field, {}, alternate);
    pendingForm = null;
  }

  async function send(id, field, inputs, alternate) {
    try {
      await api.insert(id, field, inputs, alternate);
      pendingForm = null;
    } catch (e) {
      toast = api.describeError(e);
      pendingForm = null;
    }
  }

  /// field：body（密碼要主密碼）、username、url（密碼項目才有，不需主密碼）
  async function copy(item = selected, field = "body") {
    if (!item) return;
    const run = async () => {
      await api.copyOnly(item.id, field, {});
      // 視窗留著；「已複製」自己消失，錯誤訊息則留到點掉為止
      const copied = t("search.copied");
      toast = copied;
      setTimeout(() => {
        if (toast === copied) toast = null;
      }, 1500);
      searchBox?.focus();
    };
    try {
      if (item.kind === "password" && field === "body") {
        await withSecret(t("secret.copy", { title: item.title }), run);
      } else {
        await run();
      }
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  function onKeydown(e) {
    if (pendingForm || secretPrompt || fieldChoice || editor || showSettings) return;

    const ctrl = e.ctrlKey || e.metaKey;
    switch (e.key) {
      // Tab 換類別，取代焦點巡覽；← → 留給搜尋框移動游標。
      case "Tab":
        e.preventDefault();
        cycleKind(e.shiftKey ? -1 : 1);
        break;
      case "ArrowDown":
        e.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        e.preventDefault();
        move(-1);
        break;
      case "PageDown":
        e.preventDefault();
        move(10, false);
        break;
      case "PageUp":
        e.preventDefault();
        move(-10, false);
        break;
      case "Enter":
        e.preventDefault();
        if (ctrl && e.shiftKey) activate("body", true);
        else if (e.shiftKey) copy();
        else activate("body");
        break;
      case "Escape":
        e.preventDefault();
        api.hideWindow();
        break;
      case "l":
        if (ctrl) {
          e.preventDefault();
          api.lock().then(() => onvaultchanged?.());
        }
        break;
      case "e":
        if (ctrl && selected) {
          e.preventDefault();
          editor = { id: selected.id };
        }
        break;
      case "n":
        if (ctrl) {
          e.preventDefault();
          editor = { id: null, title: query, kind: kindFilter, workspace: wsFilter };
        }
        break;
      case ",":
        if (ctrl) {
          e.preventDefault();
          showSettings = true;
        }
        break;
    }
  }
</script>

{#snippet copyIcon()}
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="9" y="9" width="11" height="11" rx="2" />
    <path d="M5 15V6a2 2 0 0 1 2-2h9" />
  </svg>
{/snippet}

{#snippet fieldRow(field, label, value)}
  <dt class="muted">
    <button
      class="act"
      aria-label={`${t("search.copyAction")} ${t(label)}`}
      title={t("search.copyAction")}
      onclick={() => copy(selected, field)}
    >
      {@render copyIcon()}
    </button>
    {t(label)}
  </dt>
  <dd>{value}</dd>
{/snippet}

<svelte:window onkeydown={onKeydown} onmousemove={onSplitMove} onmouseup={endSplit} />

<div class="wrap">
  {#if vault.conflict_files?.length > 0}
    <div class="banner" role="alert">
      {t("search.conflict", { n: vault.conflict_files.length })}
    </div>
  {/if}

  <div class="searchbar">
    <select
      class="ws"
      value={wsFilter}
      onchange={(e) => pickWorkspace(e.currentTarget.value)}
      aria-label={t("search.workspace")}
    >
      <option value="">{t("search.allWorkspaces")}</option>
      {#each workspaces as w (w.id)}
        <option value={w.id}>{w.name}</option>
      {/each}
    </select>
    <input
      bind:this={searchBox}
      bind:value={query}
      oninput={onInput}
      placeholder={t("search.placeholder")}
      aria-label={t("search.placeholder")}
      spellcheck="false"
      autocomplete="off"
    />
    <div class="kinds" role="tablist" aria-label={t("kind.snippet")}>
      {#each KIND_FILTERS as kind (kind)}
        <button
          role="tab"
          aria-selected={kindFilter === kind}
          class:on={kindFilter === kind}
          onclick={() => pickKind(kind)}
        >
          {t(`kind.${kind}`)}
        </button>
      {/each}
    </div>

  </div>

  {#if fieldChoice}
    <FieldChoice
      item={fieldChoice.item}
      onpick={async (field) => {
        const { item, alternate } = fieldChoice;
        fieldChoice = null;
        await sendField(item, field, alternate);
      }}
      oncancel={() => {
        fieldChoice = null;
        focusSearch();
      }}
    />
  {:else if secretPrompt}
    <SecretPrompt
      reason={secretPrompt.reason}
      onunlocked={afterSecretUnlocked}
      oncancel={() => {
        secretPrompt = null;
        focusSearch();
      }}
    />
  {:else if pendingForm}
    <InputForm
      item={pendingForm.item}
      plan={pendingForm.plan}
      onsubmit={(inputs) =>
        send(pendingForm.item.id, pendingForm.field, inputs, pendingForm.alternate)}
      oncancel={() => {
        pendingForm = null;
        focusSearch();
      }}
    />
  {:else}
    <div class="body" class:dragging={draggingSplit} bind:this={bodyEl}>
      <ul
        class="list"
        style="flex: 0 0 {splitPct}%"
        bind:this={listEl}
        role="listbox"
        aria-label={t("search.placeholder")}
        tabindex="-1"
      >
        {#each hits as hit, i (hit.id)}
          <li
            role="option"
            aria-selected={i === cursor}
            class:sel={i === cursor}
            onclick={() => (cursor = i)}
            ondblclick={() => activate("body")}
          >
            <span class="title">{hit.title}</span>
            {#if showWsLabel && hit.workspace}
              <span class="ws-label muted">{wsName[hit.workspace]}</span>
            {/if}
            {#each hit.tags as tag}
              <span class="tag">{tag}</span>
            {/each}
            <span class="excerpt muted">{hit.excerpt}</span>
            <!-- 兩個鈕都要擋住冒泡，否則會連帶觸發整列的單擊（選取）與雙擊（送出）。
                 密碼項目的複製鈕在預覽的各欄位旁 -->
            {#if hit.kind !== "password"}
              <button
                class="act"
                aria-label={t("search.copyAction")}
                title={t("search.copyHint")}
                onclick={(e) => {
                  e.stopPropagation();
                  copy(hit);
                }}
                ondblclick={(e) => e.stopPropagation()}
              >
                {@render copyIcon()}
              </button>
            {/if}
            <button
              class="act"
              aria-label={t("editor.edit")}
              title={t("editor.editHint")}
              onclick={(e) => {
                e.stopPropagation();
                editor = { id: hit.id };
              }}
              ondblclick={(e) => e.stopPropagation()}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M4 20h4L19 9l-4-4L4 16z" />
                <path d="M14 6l4 4" />
              </svg>
            </button>
          </li>
        {:else}
          <li class="empty muted">{t("search.empty")}</li>
        {/each}
      </ul>

      <!-- 拖這條改左右比例 -->
      <div
        class="splitter"
        role="separator"
        aria-orientation="vertical"
        aria-label={t("search.resize")}
        onmousedown={startSplit}
      ></div>

      <aside class="preview">
        {#if selected}
          <div class="preview-head muted">{selected.title}</div>
          {#if selected.kind === "password"}
            {#if previewPw}
              <dl class="pw">
                {#if previewPw.username}
                  {@render fieldRow("username", "editor.username", previewPw.username)}
                {/if}
                {#if previewPw.url}
                  {@render fieldRow("url", "editor.site", previewPw.url)}
                {/if}
                {@render fieldRow("body", "editor.password", "••••••••")}
              </dl>
            {/if}
          {:else}
            <pre>{previewText}</pre>
          {/if}
        {/if}
      </aside>
    </div>
  {/if}

  <div class="fabs">
    <!-- 用 SVG 而非 ⚙ ／＋ 這類字元：字形的字身框不在正中央，全形加號尤其偏 -->
    <button
      class="fab"
      onclick={() => (showSettings = true)}
      aria-label={t("search.settings")}
      title="{t('search.settings')} (Ctrl+,)"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="3.2" />
        <path
          d="M19.4 13.5a7.6 7.6 0 0 0 0-3l1.9-1.5-1.9-3.3-2.3.9a7.6 7.6 0 0 0-2.6-1.5L14.1 2.7h-3.8l-.4 2.4a7.6 7.6 0 0 0-2.6 1.5l-2.3-.9L3.1 9l1.9 1.5a7.6 7.6 0 0 0 0 3L3.1 15l1.9 3.3 2.3-.9a7.6 7.6 0 0 0 2.6 1.5l.4 2.4h3.8l.4-2.4a7.6 7.6 0 0 0 2.6-1.5l2.3.9 1.9-3.3z"
        />
      </svg>
    </button>
    <button
      class="fab primary"
      onclick={() => (editor = { id: null, title: query, kind: kindFilter, workspace: wsFilter })}
      aria-label={t("search.add")}
      title="{t('search.add')} (Ctrl+N)"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 5v14M5 12h14" />
      </svg>
    </button>
  </div>

  <footer class="muted">
    <span>{t("search.hintSend")}</span>
    <span>{t("search.hintCopy")}</span>
    <span>{t("search.hintFilter")}</span>
    <span>{t("search.hintClose")}</span>
  </footer>

  {#if editor}
    <Editor
      id={editor.id}
      initialTitle={editor.title ?? ""}
      initialKind={editor.kind ?? "snippet"}
      initialWorkspace={editor.workspace ?? ""}
      onsaved={async () => {
        editor = null;
        await runSearch();
        await onvaultchanged?.();
        focusSearch();
      }}
      oncancel={() => {
        editor = null;
        focusSearch();
      }}
    />
  {/if}

  {#if showSettings}
    <Settings
      {vault}
      {onpreferenceschanged}
      onclose={() => {
        showSettings = false;
        focusSearch();
      }}
      onchanged={async () => {
        await loadWorkspaces();
        await runSearch();
        await onvaultchanged?.();
      }}
    />
  {/if}

  {#if toast}
    <button class="toast" onclick={() => (toast = null)} aria-live="polite">{toast}</button>
  {/if}
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    position: relative;
  }

  .searchbar {
    display: flex;
    align-items: center;
    padding: 10px;
    border-bottom: 1px solid var(--border);
  }
  .ws {
    flex: none;
    max-width: 9em;
    margin-right: 8px;
    font: inherit;
    font-size: 0.9em;
    color: inherit;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 7px 6px;
  }
  .ws-label {
    flex: none;
    font-size: 0.72em;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
  }

  .searchbar input {
    /* 全域的 input 是 width:100%，在 flex 列裡會把類別鈕擠掉 */
    flex: 1;
    min-width: 0;
    width: auto;
  }

  .kinds {
    flex: none;
    display: flex;
    gap: 2px;
    margin-left: 8px;
  }
  .kinds button {
    padding: 4px 10px;
    font-size: 0.78em;
    border-radius: 999px;
    background: none;
    border: 1px solid transparent;
    color: var(--fg-dim);
    cursor: default;
  }
  .kinds button:hover {
    background: var(--bg-hover);
  }
  .kinds button.on {
    background: var(--bg-sel);
    border-color: var(--accent);
    color: var(--fg);
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .list {
    margin: 0;
    padding: 4px;
    list-style: none;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .list li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius);
    cursor: default;
  }
  .list li.sel {
    background: var(--bg-sel);
  }

  .title {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-weight: 600;
  }

  .tag {
    flex: none;
    font-size: 0.75em;
    color: var(--accent);
  }

  .act {
    flex: none;
    width: 24px;
    height: 24px;
    padding: 0;
    margin-left: 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    border-radius: var(--radius);
    color: var(--fg-dim);
    opacity: 0.55;
    cursor: default;
  }
  .act:hover {
    opacity: 1;
    background: var(--bg-hover);
    color: var(--fg);
  }
  .list li.sel .act {
    opacity: 1;
  }
  .act svg {
    width: 14px;
    height: 14px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .excerpt {
    /* basis 0：空間不夠時先犧牲摘要，標題撐到最後 */
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 0.85em;
  }

  .empty {
    justify-content: center;
    padding: 24px;
  }

  .splitter {
    flex: none;
    width: 5px;
    cursor: col-resize;
    background: var(--border);
    opacity: 0.6;
  }
  .splitter:hover,
  .body.dragging .splitter {
    opacity: 1;
    background: var(--accent);
  }
  /* 拖曳中整個區域禁選，否則會把清單文字一起反白 */
  .body.dragging {
    user-select: none;
    cursor: col-resize;
  }

  .preview {
    flex: 1 1 0;
    min-width: 0;
    padding: 10px;
    overflow: auto;
  }
  .preview-head {
    font-size: 0.8em;
    margin-bottom: 6px;
  }
  .pw {
    margin: 0;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 12px;
    font-size: 0.85em;
  }
  .pw dt {
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .pw dt .act {
    margin-left: -4px;
    opacity: 0.8;
  }
  .pw dd {
    align-self: center;
    margin: 0;
    word-break: break-all;
    font-family: "Cascadia Code", Consolas, monospace;
  }

  .preview pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 0.85em;
  }

  footer {
    display: flex;
    gap: 14px;
    padding: 6px 12px;
    border-top: 1px solid var(--border);
    font-size: 0.75em;
  }

  .fabs {
    position: absolute;
    bottom: 34px;
    left: 0;
    right: 0;
    display: flex;
    justify-content: space-between;
    padding: 0 12px;
    pointer-events: none;
  }
  .fab {
    pointer-events: auto;
    width: 36px;
    height: 36px;
    padding: 0;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.3);
  }
  .fab svg {
    width: 18px;
    height: 18px;
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .fab.primary {
    background: var(--accent);
    color: #0b1220;
    border-color: transparent;
  }
  .fab.primary svg {
    stroke-width: 2.2;
  }

  .toast {
    position: absolute;
    left: 50%;
    bottom: 42px;
    transform: translateX(-50%);
    background: var(--bg-raised);
    border: 1px solid var(--border);
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.3);
    max-width: 90%;
  }
</style>
