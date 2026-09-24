<script>
  import { listen } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import * as api from "./api.js";
  import { t } from "./i18n.svelte.js";
  import { createMatcher, labelOf } from "./shortcuts.svelte.js";
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
  /// 搜尋框裡的標籤方塊，每個都要符合。跟 query 一樣從搜尋框的內容讀出來
  let tags = $state([]);
  /// 保險庫裡所有標籤與筆數，給 # 選單用
  let allTags = $state([]);
  /// 打 # 時的標籤選單：{ node, start, end, items, index }。
  /// node 是游標所在的文字節點，start..end 是「#片段」在裡面的位置
  let suggest = $state(null);
  /// 按 Esc 關掉選單後，直到內容再變動前不重開
  let suggestDismissed = false;
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
  /// 預覽展開失敗時的錯誤訊息，顯示在預覽區
  let previewError = $state(null);
  /// 密碼項目的預覽：帳號與網域（都沒加密），密碼本身只顯示遮罩
  let previewPw = $state(null);
  /// 非 null 時畫面切換成表單模式（原地切換，不開 modal）
  let pendingForm = $state(null);
  /// 非 null 時原地切換成主密碼提示。{ reason, retry }
  let secretPrompt = $state(null);
  /// 非 null 時原地問要送帳號還是密碼。{ item }
  let fieldChoice = $state(null);

  /// 搜尋框：可編輯區塊，文字與標籤方塊在同一行，游標可以停在任何位置
  let searchBox;
  /// 搜尋框是空的（沒有文字也沒有標籤）時顯示提示字
  let fieldEmpty = $state(true);
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
      hits = await api.search(query, tags, 200, kindFilter, wsFilter || null);
      cursor = 0;
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  async function pickKind(kind) {
    kindFilter = kind;
    await runSearch();
    // 換完類別焦點要回到搜尋框，不然接著打字會沒反應
    await focusEnd();
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
      allTags = await api.tagList();
      // 標籤被刪光的就從搜尋條件拿掉
      for (const chip of searchBox?.querySelectorAll(".chip") ?? []) {
        if (!allTags.some((x) => x.name === chip.dataset.tag)) chip.remove();
      }
      readField();
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  async function pickWorkspace(id) {
    wsFilter = id;
    await runSearch();
    await focusEnd();
  }

  /// 依下拉選單的順序循環：全部 → 各工作區
  function cycleWorkspace(delta) {
    const ids = ["", ...workspaces.map((w) => w.id)];
    const at = Math.max(ids.indexOf(wsFilter), 0);
    pickWorkspace(ids[(at + delta + ids.length) % ids.length]);
  }

  /// 從搜尋框的 DOM 讀出文字與標籤。標籤方塊在文字裡算一個空白，兩邊的字不會黏在一起
  function readField() {
    let text = "";
    const found = [];
    const walk = (node) => {
      for (const child of node.childNodes) {
        if (child.nodeType === Node.TEXT_NODE) text += child.data;
        else if (child.classList?.contains("chip")) {
          found.push(child.dataset.tag);
          text += " ";
        } else if (child.nodeName === "BR") text += " ";
        else walk(child);
      }
    };
    if (searchBox) walk(searchBox);
    // 可編輯區塊會把連續空白存成 &nbsp;
    query = text.replace(/\u00a0/g, " ");
    tags = [...new Set(found)];
    fieldEmpty = query.trim() === "" && tags.length === 0;
    // 清空後瀏覽器可能留下 <br>，會把搜尋框撐成兩行
    if (fieldEmpty && searchBox && searchBox.childNodes.length > 0 && query === "") {
      searchBox.replaceChildren();
    }
  }

  function onInput() {
    readField();
    suggestDismissed = false;
    updateSuggest();
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(runSearch, 30);
  }

  /// 游標前面是「#片段」就開標籤選單，列出名稱含這個片段的標籤，開頭相符的排前面
  function updateSuggest() {
    const sel = getSelection();
    const node = sel?.anchorNode;
    if (
      suggestDismissed ||
      !sel?.isCollapsed ||
      node?.nodeType !== Node.TEXT_NODE ||
      !searchBox?.contains(node)
    ) {
      suggest = null;
      return;
    }
    const caret = sel.anchorOffset;
    const m = node.data.slice(0, caret).match(/(^|[\s\u00a0])#([^\s\u00a0#]*)$/);
    if (!m) {
      suggest = null;
      return;
    }
    const frag = m[2].toLowerCase();
    const starts = (tag) => (tag.name.toLowerCase().startsWith(frag) ? 0 : 1);
    const items = allTags
      .filter((tag) => !tags.includes(tag.name) && tag.name.toLowerCase().includes(frag))
      .sort((a, b) => starts(a) - starts(b) || b.count - a.count)
      .slice(0, 8);
    suggest = items.length
      ? { node, start: caret - m[2].length - 1, end: caret, items, index: 0 }
      : null;
  }

  /// 標籤方塊：不能編輯，游標把它當成一個字跳過，Backspace／Delete 會整個刪掉
  function makeChip(name) {
    const chip = document.createElement("span");
    chip.className = "chip";
    chip.contentEditable = "false";
    chip.dataset.tag = name;
    chip.textContent = `#${name}`;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "chip-x";
    remove.tabIndex = -1;
    remove.textContent = "×";
    remove.title = t("search.removeTag", { tag: name });
    remove.setAttribute("aria-label", remove.title);
    chip.append(remove);
    return chip;
  }

  /// 把「#片段」換成標籤方塊，後面補一個空白並把游標放在空白後
  function pickTag(name) {
    if (!suggest) return;
    const { node, start, end } = suggest;
    const rest = node.splitText(start);
    rest.data = rest.data.slice(end - start);
    if (!/^[\s\u00a0]/.test(rest.data)) rest.data = "\u00a0" + rest.data;
    rest.parentNode.insertBefore(makeChip(name), rest);
    suggest = null;
    placeCaret(rest, 1);
    onInput();
  }

  function placeCaret(node, offset) {
    const range = document.createRange();
    range.setStart(node, offset);
    range.collapse(true);
    const sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  /// 按方塊上的 × 刪掉那個標籤
  function onFieldClick(e) {
    const chip = e.target.closest?.(".chip-x")?.parentElement;
    if (chip) {
      e.preventDefault();
      chip.remove();
      searchBox.focus();
      onInput();
      return;
    }
    updateSuggest();
  }

  /// 搜尋框本身的按鍵：不讓 Enter 換行
  function onFieldKeydown(e) {
    if (e.key === "Enter") e.preventDefault();
  }

  /// 貼上時只收純文字，換行換成空白
  function onFieldPaste(e) {
    e.preventDefault();
    const text = (e.clipboardData?.getData("text/plain") ?? "").replace(/\s+/g, " ");
    document.execCommand("insertText", false, text);
  }

  /// 標籤選單開著時，方向鍵、Enter、Tab、Esc 給選單用。回傳 true 表示已處理
  function suggestKeydown(e) {
    if (!suggest || e.isComposing) return false;
    const n = suggest.items.length;
    switch (e.key) {
      case "ArrowDown":
        suggest.index = (suggest.index + 1) % n;
        break;
      case "ArrowUp":
        suggest.index = (suggest.index - 1 + n) % n;
        break;
      case "Enter":
      case "Tab":
        pickTag(suggest.items[suggest.index].name);
        break;
      case "Escape":
        suggest = null;
        suggestDismissed = true;
        break;
      default:
        return false;
    }
    e.preventDefault();
    return true;
  }

  $effect(() => {
    const item = selected;
    previewPw = null;
    previewError = null;
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
      .catch((e) => {
        if (cancelled) return;
        previewText = "";
        previewError = api.describeError(e);
      });
    return () => {
      cancelled = true;
    };
  });

  /// 聚焦到搜尋框，游標放在最後面，已經打的字和標籤都留著
  async function focusEnd() {
    await tick();
    if (!searchBox) return;
    searchBox.focus();
    const range = document.createRange();
    range.selectNodeContents(searchBox);
    range.collapse(false);
    const sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  /// 聚焦到搜尋框並全選，接著打字會取代整個搜尋條件
  async function focusSearch() {
    await tick();
    if (!searchBox) return;
    searchBox.focus();
    const range = document.createRange();
    range.selectNodeContents(searchBox);
    const sel = getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
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
  async function activate(field = "body") {
    if (!selected) return;
    const item = selected;

    if (item.kind === "password" && field === "body" && item.has_username) {
      fieldChoice = { item };
      return;
    }
    await sendField(item, field);
  }

  /// 送出指定欄位。密碼要主密碼，帳號不用——帳號沒有加密。
  async function sendField(item, field) {
    try {
      if (item.kind === "password" && field === "body") {
        await withSecret(t("secret.send", { title: item.title }), () => doActivate(item, field));
      } else {
        await doActivate(item, field);
      }
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  async function doActivate(item, field) {
    const plan = await api.prepareInsert(item.id);
    if (plan.needs_inputs.length > 0) {
      pendingForm = { item, plan, field };
      return;
    }
    await api.insert(item.id, field, {});
    pendingForm = null;
  }

  async function send(id, field, inputs) {
    try {
      await api.insert(id, field, inputs);
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

  const keys = createMatcher();

  /// 表單、主密碼提示、欄位選擇、編輯、設定開著時，按鍵交給它們自己處理
  const keysBlocked = () => pendingForm || secretPrompt || fieldChoice || editor || showSettings;

  /// 依設定的快捷鍵分派。沒對應到動作的按鍵照常交給輸入框（打字、← → 移動游標）。
  function onKeydown(e) {
    if (keysBlocked()) {
      keys.reset();
      return;
    }
    if (suggestKeydown(e)) {
      keys.reset();
      return;
    }
    const { action, prevent } = keys.keydown(e);
    if (prevent) e.preventDefault();
    if (action) runAction(action);
  }

  function onKeyup(e) {
    if (keysBlocked()) return;
    const action = keys.keyup(e);
    if (action) runAction(action);
  }

  function runAction(action) {
    switch (action) {
      case "send":
        activate("body");
        break;
      case "copy":
        copy();
        break;
      case "openBookmark":
        openBookmark();
        break;
      case "kindNext":
        cycleKind(1);
        break;
      case "kindPrev":
        cycleKind(-1);
        break;
      case "workspaceNext":
        cycleWorkspace(1);
        break;
      case "workspacePrev":
        cycleWorkspace(-1);
        break;
      case "selectDown":
        move(1);
        break;
      case "selectUp":
        move(-1);
        break;
      case "pageDown":
        move(10, false);
        break;
      case "pageUp":
        move(-10, false);
        break;
      case "newItem":
        openNew();
        break;
      case "editItem":
        if (selected) editor = { id: selected.id };
        break;
      case "settings":
        showSettings = true;
        break;
      case "lock":
        api.lock().then(() => onvaultchanged?.());
        break;
      case "close":
        api.hideWindow();
        break;
    }
  }

  /// 用預設瀏覽器開啟選中的書籤；選中的不是書籤就提示
  async function openBookmark() {
    if (!selected) return;
    if (selected.kind !== "bookmark") {
      toast = t("search.onlyBookmarks");
      return;
    }
    try {
      await api.openBookmark(selected.id);
    } catch (e) {
      toast = api.describeError(e);
    }
  }

  /// 新增：搜尋框的文字當標題、標籤方塊當標籤，類別與工作區沿用搜尋畫面當下的
  function openNew() {
    editor = {
      id: null,
      title: query.replace(/\s+/g, " ").trim(),
      kind: kindFilter,
      workspace: wsFilter,
      tags: [...tags],
    };
  }

  /// 按鈕提示文字後面附上目前的快捷鍵，沒指定就只有文字
  function withKey(text, action) {
    const key = labelOf(action);
    return key ? `${text} (${key})` : text;
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

<svelte:window
  onkeydown={onKeydown}
  onkeyup={onKeyup}
  onblur={() => keys.reset()}
  onmousemove={onSplitMove}
  onmouseup={endSplit}
/>

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
      title={withKey(t("search.workspace"), "workspaceNext")}
    >
      <option value="">{t("search.allWorkspaces")}</option>
      {#each workspaces as w (w.id)}
        <option value={w.id}>{w.name}</option>
      {/each}
    </select>
    <!-- 標籤選單以這層定位 -->
    <div class="searchfield">
      <!-- 內容由 DOM 直接管理（打字、標籤方塊），Svelte 不重畫它；讀值走 readField() -->
      {#if fieldEmpty}
        <span class="placeholder" aria-hidden="true">{t("search.placeholder")}</span>
      {/if}
      <div
        class="field"
        bind:this={searchBox}
        contenteditable="true"
        role="searchbox"
        tabindex="0"
        spellcheck="false"
        aria-label={t("search.placeholder")}
        oninput={onInput}
        onclick={onFieldClick}
        onkeydown={onFieldKeydown}
        onpaste={onFieldPaste}
        onkeyup={(e) => ["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key) && updateSuggest()}
        onblur={() => (suggest = null)}
      ></div>
      {#if suggest}
        <ul class="suggest" role="listbox" aria-label={t("search.tagSuggest")}>
          {#each suggest.items as item, i (item.name)}
            <!-- mousedown 不搶焦點，輸入框的游標位置還在 -->
            <li
              role="option"
              aria-selected={i === suggest.index}
              class:sel={i === suggest.index}
              onmousedown={(e) => e.preventDefault()}
              onclick={() => pickTag(item.name)}
            >
              <span>#{item.name}</span>
              <span class="muted small">{item.count}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
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
        const { item } = fieldChoice;
        fieldChoice = null;
        await sendField(item, field);
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
        send(pendingForm.item.id, pendingForm.field, inputs)}
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
                title={withKey(t("search.copyAction"), "copy")}
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
              title={withKey(t("editor.edit"), "editItem")}
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
            {#if previewError}
              <p class="preview-error">{previewError}</p>
            {/if}
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
      onclick={openNew}
      aria-label={t("search.add")}
      title={withKey(t("search.add"), "newItem")}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 5v14M5 12h14" />
      </svg>
    </button>
  </div>

  <footer class="muted">
    {#each [["send", "search.hintSend"], ["copy", "search.hintCopy"], ["openBookmark", "search.hintOpen"], ["kindNext", "search.hintFilter"], ["workspaceNext", "search.hintWorkspace"], ["close", "search.hintClose"]] as [action, text] (action)}
      <!-- 「開啟」只對書籤有用，在書籤類別才顯示 -->
      {#if labelOf(action) && (action !== "openBookmark" || kindFilter === "bookmark")}
        <span>{labelOf(action)} {t(text)}</span>
      {/if}
    {/each}
  </footer>

  {#if editor}
    <Editor
      id={editor.id}
      initialTitle={editor.title ?? ""}
      initialKind={editor.kind ?? "snippet"}
      initialWorkspace={editor.workspace ?? ""}
      initialTags={editor.tags ?? []}
      onsaved={async () => {
        editor = null;
        // 可能新增或刪掉了標籤，# 選單要跟著更新
        await loadWorkspaces();
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

  .searchfield {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  /* 固定成一行高，跟一般輸入框一樣 38px；太長時水平捲動、不顯示捲軸 */
  .field {
    height: 38px;
    padding: 8px 10px;
    line-height: 20px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    white-space: nowrap;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    cursor: text;
  }
  .field::-webkit-scrollbar {
    display: none;
  }
  .placeholder {
    position: absolute;
    left: 11px;
    top: 9px;
    line-height: 20px;
    color: var(--fg-dim);
    pointer-events: none;
    white-space: nowrap;
  }
  .field:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  /* 標籤方塊是 DOM 直接建立的，不帶 Svelte 的 scoped class，要用 :global */
  .field :global(.chip) {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin: 0 2px;
    padding: 1px 2px 1px 8px;
    border-radius: 999px;
    background: var(--bg-sel);
    font-size: 0.85em;
    line-height: 18px;
    vertical-align: 1px;
    user-select: none;
  }
  .field :global(.chip-x) {
    padding: 0 4px;
    background: none;
    border: none;
    color: inherit;
    opacity: 0.7;
    cursor: default;
  }
  .field :global(.chip-x:hover) {
    opacity: 1;
  }
  .suggest {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 20;
    min-width: 12em;
    max-width: 100%;
    margin: 0;
    padding: 4px;
    list-style: none;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.3);
  }
  .suggest li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 5px 8px;
    border-radius: 6px;
    cursor: default;
  }
  .suggest li.sel {
    background: var(--bg-sel);
  }
  .suggest .small {
    font-size: 0.8em;
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

  .preview-error {
    margin: 0 0 8px;
    color: var(--danger);
    font-size: 0.85em;
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
