<script>
  // 建立/編輯 modal。建立與編輯共用同一個表單，靠有沒有 id 區分。
  import { tick } from "svelte";
  import * as api from "./api.js";
  import { t, describeTemplateError } from "./i18n.svelte.js";
  import SecretPrompt from "./SecretPrompt.svelte";

  let {
    id = null,
    initialTitle = "",
    initialKind = "snippet",
    initialWorkspace = "",
    initialTags = [],
    onsaved,
    oncancel,
  } = $props();

  let kind = $state(initialKind);
  let title = $state(initialTitle);
  let body = $state("");
  let username = $state("");
  let url = $state("");
  let tagText = $state(initialTags.join(", "));
  let workspaces = $state([]);
  let workspace = $state(initialWorkspace);
  let loading = $state(id !== null);
  let error = $state(null);
  /// 模板文法錯誤，擋存檔
  let templateIssues = $state([]);
  /// 引用的片語找不到、循環引用等，只提醒不擋存檔
  let templateWarnings = $state([]);
  let showPassword = $state(false);
  let entropy = $state(null);
  /// 這一筆已經有密碼。body 留空表示沿用原密碼。
  let hasPassword = $state(false);
  /// 非 null 時畫面換成主密碼提示。{ reason, after }
  let secretNeeded = $state(null);
  let titleInput;

  let debounceTimer;

  const canSave = $derived(title.trim().length > 0 && templateIssues.length === 0 && !loading);

  $effect(() => {
    api.workspaceList().then((list) => {
      workspaces = list;
      if (!list.some((w) => w.id === workspace)) workspace = list[0]?.id ?? "";
    });
  });

  $effect(() => {
    if (id === null) {
      tick().then(() => titleInput?.focus());
      return;
    }
    let cancelled = false;
    api
      .itemGet(id)
      .then((item) => {
        if (cancelled) return;
        kind = item.kind;
        title = item.title;
        // 密碼項目的 body 是空的，原文要按「顯示」另外取
        body = item.body;
        hasPassword = item.has_password;
        username = item.username ?? "";
        url = item.url ?? "";
        if (item.workspace) workspace = item.workspace;
        tagText = item.tags.join(", ");
        loading = false;
        tick().then(() => titleInput?.focus());
      })
      .catch((e) => {
        if (cancelled) return;
        error = api.describeError(e);
        loading = false;
      });
    return () => {
      cancelled = true;
    };
  });

  // 只有片語需要檢查模板文法
  function onBodyInput() {
    if (kind !== "snippet") {
      templateIssues = [];
      templateWarnings = [];
      return;
    }
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(async () => {
      const check = await api.templateValidate(body);
      templateIssues = check.errors;
      templateWarnings = check.warnings;
    }, 300);
  }

  /// 後端的 offset 是 UTF-8 位元組位置，轉成從 1 起算的字數
  function charPosition(text, byteOffset) {
    const encoder = new TextEncoder();
    let bytes = 0;
    let pos = 0;
    for (const ch of text) {
      if (bytes >= byteOffset) break;
      bytes += encoder.encode(ch).length;
      pos += 1;
    }
    return pos + 1;
  }

  $effect(() => {
    // 切換類型時重新檢查：換成非片語會清掉模板錯誤
    kind;
    onBodyInput();
  });

  /// 切換密碼欄的可見度。欄位是空的而這筆已有密碼時，先解密取回原文（需要主密碼）。
  async function toggleShow() {
    if (showPassword) {
      showPassword = false;
      return;
    }
    if (!body && hasPassword && id) {
      try {
        body = await api.revealPassword(id);
      } catch (e) {
        if (e?.kind === "SecretsLocked") {
          secretNeeded = { reason: t("secret.edit"), after: toggleShow };
          return;
        }
        error = api.describeError(e);
        return;
      }
    }
    showPassword = true;
  }

  async function generate() {
    error = null;
    try {
      const opts = (await api.settingsGet()).password_gen;
      body = await api.generatePassword(opts);
      entropy = Math.round(await api.entropyBits(opts));
      showPassword = true;
    } catch (e) {
      error = api.describeError(e);
    }
  }

  async function save(e) {
    e?.preventDefault();
    if (!canSave) return;
    error = null;
    try {
      const savedId = await api.itemUpsert({
        id,
        kind,
        title,
        body,
        username: kind === "password" ? username : null,
        url: kind === "password" ? url : null,
        workspace: workspace || null,
        tags: tagText
          .split(/[,，]/)
          .map((s) => s.trim())
          .filter(Boolean),
      });
      onsaved(savedId);
    } catch (err) {
      // 存密碼只要公鑰。SecretsLocked 表示保險庫還沒有公鑰，解鎖一次會補上。
      if (err?.kind === "SecretsLocked") {
        secretNeeded = { reason: t("secret.saveItem"), after: save };
      } else {
        error = api.describeError(err);
      }
    }
  }

  async function remove() {
    if (!id) return;
    if (!confirm(t("editor.confirmDelete", { title }))) return;
    try {
      await api.itemDelete(id);
      onsaved(null);
    } catch (e) {
      error = api.describeError(e);
    }
  }

  function onKeydown(e) {
    const ctrl = e.ctrlKey || e.metaKey;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      oncancel();
    } else if (ctrl && (e.key === "s" || e.key === "Enter")) {
      e.preventDefault();
      save();
    }
  }

  const KINDS = [
    ["snippet", "kind.snippet"],
    ["bookmark", "kind.bookmark"],
    ["password", "kind.password"],
  ];
</script>

<!-- 點背景時不讓焦點離開視窗：焦點掉到外面的話 Tab 與 Esc 都會失效 -->
<!-- Esc 聽整個視窗：焦點在哪裡都能關 -->
<svelte:window onkeydown={onKeydown} />

<div
  class="backdrop"
  role="presentation"
  onmousedown={(e) => e.target === e.currentTarget && e.preventDefault()}
>
  <form
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-label={id ? t("editor.edit") : t("editor.new")}
    onsubmit={save}
  >
    <header>
      <strong>{id ? t("editor.edit") : t("editor.new")}</strong>
      <div class="kinds" role="radiogroup" aria-label={t("editor.title")}>
        {#each KINDS as [value, label]}
          <button
            type="button"
            role="radio"
            aria-checked={kind === value}
            class:active={kind === value}
            onclick={() => (kind = value)}
          >
            {t(label)}
          </button>
        {/each}
      </div>
    </header>

    {#if secretNeeded}
      <SecretPrompt
        reason={secretNeeded.reason}
        onunlocked={async () => {
          const after = secretNeeded.after;
          secretNeeded = null;
          await after?.();
        }}
        oncancel={() => (secretNeeded = null)}
      />
    {:else if loading}
      <p class="muted pad">{t("common.loading")}</p>
    {:else}
      <div class="fields">
        <label>
          {t("editor.title")}
          <input bind:value={title} bind:this={titleInput} autocomplete="off" />
        </label>

        {#if kind === "password"}
          <label>
            {t("editor.site")}
            <input bind:value={url} autocomplete="off" placeholder="example.com" />
          </label>
          <label>
            {t("editor.username")}
            <input bind:value={username} autocomplete="off" />
          </label>
          <label>
            {t("editor.password")}
            <span class="row">
              {#if showPassword}
                <input
                  bind:value={body}
                  autocomplete="off"
                  placeholder={hasPassword ? t("editor.keepPassword") : ""}
                />
              {:else}
                <input
                  bind:value={body}
                  type="password"
                  autocomplete="new-password"
                  placeholder={hasPassword ? t("editor.keepPassword") : ""}
                />
              {/if}
              <button type="button" onclick={toggleShow}>
                {showPassword ? t("editor.hide") : t("editor.show")}
              </button>
              <button type="button" onclick={generate}>{t("editor.generate")}</button>
            </span>
          </label>
          {#if entropy !== null}
            <p class="muted hint">{t("editor.bits", { n: entropy })}</p>
          {/if}
        {:else if kind === "bookmark"}
          <label>
            {t("editor.url")}
            <input bind:value={body} autocomplete="off" placeholder="https://" />
          </label>
        {:else}
          <label class="grow">
            {t("editor.body")}
            <textarea
              bind:value={body}
              oninput={onBodyInput}
              class="mono"
              spellcheck="false"
              class:invalid={templateIssues.length > 0}
            ></textarea>
          </label>
          {#each templateIssues as issue}
            <p class="error" role="alert">
              {t("editor.templateError", {
                message: describeTemplateError(issue),
                where: t("editor.templateAt", { pos: charPosition(body, issue.offset) }),
              })}
            </p>
          {/each}
          {#each templateWarnings as warning}
            <p class="warn" role="status">{describeTemplateError(warning)}</p>
          {/each}
          <details>
            <summary class="muted">{t("editor.placeholders")}</summary>
            <ul class="muted placeholders">
              <li><code>{"${date}"}</code> <code>{"${date:+7d}"}</code> <code>{"${date::long}"}</code></li>
              <li><code>{"${time}"}</code> <code>{"${datetime}"}</code></li>
              <li><code>{"${input:x}"}</code> <code>{"${select:x:a|b}"}</code></li>
              <li><code>{"${clipboard}"}</code> <code>{"${cursor}"}</code> <code>{"${uuid}"}</code></li>
              <li><code>{"${snippet:title}"}</code></li>
            </ul>
          </details>
        {/if}

        {#if workspaces.length > 1}
          <label class="tags">
            {t("editor.workspace")}
            <select bind:value={workspace}>
              {#each workspaces as w (w.id)}
                <option value={w.id}>{w.name}</option>
              {/each}
            </select>
          </label>
        {/if}

        <label class="tags">
          {t("editor.tags")}
          <input bind:value={tagText} autocomplete="off" placeholder={t("editor.tagsPlaceholder")} />
        </label>
      </div>
    {/if}

    {#if error}
      <p class="error pad" role="alert">{error}</p>
    {/if}

    <footer class:hidden={secretNeeded}>
      {#if id}
        <button type="button" class="danger" onclick={remove}>{t("common.delete")}</button>
      {/if}
      <span class="spacer"></span>
      <button type="button" onclick={oncancel}>{t("common.cancel")}</button>
      <button type="submit" disabled={!canSave}>{t("common.save")}</button>
    </footer>
  </form>
</div>

<style>
  /* 用 flex 而非 grid 置中，並加上內距：
     modal 的 max-height 吃的是這一層的內容高度，有內距就一定切不到底部按鈕。 */
  .backdrop {
    position: absolute;
    inset: 0;
    padding: 12px;
    background: rgb(0 0 0 / 0.45);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10;
  }

  .modal {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    width: min(680px, 92%);
    max-height: 100%;
    display: flex;
    flex-direction: column;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.4);
  }

  header {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }

  .kinds {
    display: flex;
    gap: 4px;
    margin-left: auto;
  }
  .kinds button {
    padding: 4px 12px;
    font-size: 0.85em;
  }
  .kinds button.active {
    background: var(--bg-sel);
    border-color: var(--accent);
  }

  .fields {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 0.85em;
  }
  label.grow {
    flex: 1;
    min-height: 200px;
  }
  label.tags {
    flex: none;
  }

  .row {
    display: flex;
    gap: 6px;
  }
  .row input {
    flex: 1;
  }

  textarea {
    flex: 1;
    min-height: 180px;
    resize: vertical;
    line-height: 1.5;
  }
  textarea.invalid {
    border-color: var(--danger);
  }

  .placeholders {
    margin: 6px 0 0;
    padding-left: 18px;
    font-size: 0.85em;
    line-height: 1.8;
  }

  .error {
    color: var(--danger);
    margin: 0;
    font-size: 0.85em;
  }
  .warn {
    color: #e0a34a;
    margin: 0;
    font-size: 0.85em;
  }
  .hint {
    margin: 0;
    font-size: 0.8em;
  }
  .pad {
    padding: 0 16px;
  }

  footer {
    display: flex;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid var(--border);
  }
  footer.hidden {
    display: none;
  }
  .spacer {
    flex: 1;
  }
  .danger {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
