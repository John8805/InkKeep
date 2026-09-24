<script>
  // 設定 modal。
  //
  // 文案只留標籤，說明放 title 提示。
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { tick } from "svelte";
  import * as api from "./api.js";
  import { t, LANGUAGES } from "./i18n.svelte.js";
  import SecretPrompt from "./SecretPrompt.svelte";
  import Shortcuts from "./Shortcuts.svelte";
  import { keyLabel } from "./shortcuts.svelte.js";

  let { vault, onclose, onchanged, onpreferenceschanged } = $props();

  let settings = $state(null);
  /// "main" 是設定本身；"shortcuts" 是快捷鍵設定頁
  let page = $state("main");
  let error = $state(null);
  let notice = $state(null);
  let sample = $state("");
  let entropy = $state(0);

  // 更換主密碼
  let oldPw = $state("");
  let newPw = $state("");
  let newPw2 = $state("");
  let pwBusy = $state(false);

  // 第一層金鑰
  let vaultKey = $state(null);
  /// 非 null 時蓋一層主密碼提示，解開之後重跑 pendingAction
  let secretNeeded = $state(null);
  let pendingAction = null;

  // 工作區
  let workspaces = $state([]);
  /// 正在改名的工作區 id；null 表示沒有
  let renamingWs = $state(null);
  /// true 時最後一列換成新增用的輸入框
  let addingWs = $state(false);
  let newWsName = $state("");
  /// 兩種匯入共用的目標工作區
  let importWs = $state("");

  // 書籤匯入
  /// 瀏覽器設定檔清單；選中的用路徑當鍵，路徑一定唯一
  let profiles = $state([]);
  let importProfile = $state("");
  const selectedProfile = $derived(profiles.find((p) => p.path === importProfile) ?? null);
  let importPreview = $state(null);
  let importBusy = $state(false);

  // 密碼匯入。路徑交給後端讀，前端只拿得到筆數，看不到任何一筆密碼
  let csvPath = $state(null);
  let pwPreview = $state(null);
  let pwImportBusy = $state(false);
  /// 匯入完成後還留著的 CSV 路徑
  let csvLeft = $state(null);

  let firstInput;

  $effect(() => {
    api
      .settingsGet()
      .then(async (s) => {
        settings = s;
        await loadWorkspaces();
        profiles = await api.importSources().catch(() => []);
        importProfile = profiles[0]?.path ?? "";
        await refreshSample();
        await tick();
        firstInput?.focus();
      })
      .catch((e) => (error = api.describeError(e)));
  });

  async function loadWorkspaces() {
    try {
      workspaces = await api.workspaceList();
      if (!workspaces.some((w) => w.id === importWs)) importWs = workspaces[0]?.id ?? "";
    } catch (e) {
      error = api.describeError(e);
    }
  }

  async function afterWorkspaceChange(message) {
    await loadWorkspaces();
    notice = message ?? null;
    onchanged?.();
  }

  async function addWorkspace() {
    const name = newWsName.trim();
    if (!name) {
      addingWs = false;
      return;
    }
    error = null;
    try {
      await api.workspaceAdd(name);
      newWsName = "";
      addingWs = false;
      await afterWorkspaceChange(t("common.saved"));
    } catch (e) {
      error = api.describeError(e);
    }
  }

  async function renameWorkspace(w, input) {
    const name = input.value.trim();
    if (renamingWs !== w.id) return;
    if (name === w.name) {
      renamingWs = null;
      return;
    }
    error = null;
    try {
      await api.workspaceRename(w.id, name);
      renamingWs = null;
      await afterWorkspaceChange(t("common.saved"));
    } catch (e) {
      // 改名失敗（重名、空白）時留在輸入框
      error = api.describeError(e);
      input.focus();
    }
  }

  /// 清單裡的輸入框：Enter 送出、Esc 只取消這一格，不關掉設定
  function wsInputKeydown(e, submit, cancel) {
    if (e.key === "Enter") {
      e.preventDefault();
      submit(e.currentTarget);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    }
  }

  function autofocus(node) {
    node.focus();
    node.select();
  }

  async function deleteWorkspace(w) {
    error = null;
    try {
      await api.workspaceDelete(w.id);
      await afterWorkspaceChange(t("common.saved"));
    } catch (e) {
      error = api.describeError(e);
    }
  }

  /// 換了匯入目標，之前的預覽就不準了（重複是依目標工作區算的）
  async function onImportTargetChange() {
    importPreview = null;
    if (csvPath) {
      try {
        pwPreview = await api.passwordImportPreview(csvPath, importWs);
      } catch (e) {
        error = api.describeError(e);
      }
    }
  }

  async function refreshSample() {
    if (!settings) return;
    try {
      sample = await api.generatePassword(settings.password_gen);
      entropy = Math.round(await api.entropyBits(settings.password_gen));
    } catch {
      sample = "";
      entropy = 0;
    }
  }

  async function save() {
    error = null;
    try {
      settings = await api.settingsSet(settings);
      notice = t("common.saved");
      await onpreferenceschanged?.();
      onchanged?.();
    } catch (e) {
      error = api.describeError(e);
    }
  }

  async function changePassword() {
    error = null;
    notice = null;
    if (newPw !== newPw2) {
      error = t("wizard.mismatch");
      return;
    }
    if (!newPw.trim()) {
      error = t("error.emptyPassword");
      return;
    }
    pwBusy = true;
    try {
      await api.changePassword(oldPw, newPw);
      oldPw = newPw = newPw2 = "";
      notice = t("settings.changed");
    } catch (e) {
      error = api.describeError(e);
    } finally {
      pwBusy = false;
    }
  }

  /// 執行需要第二層的動作：第二層鎖著就先問主密碼，解開後重跑。
  async function needsSecret(reason, run) {
    error = null;
    try {
      await run();
    } catch (e) {
      if (e?.kind === "SecretsLocked") {
        pendingAction = run;
        secretNeeded = reason;
      } else {
        error = api.describeError(e);
      }
    }
  }

  async function showVaultKey() {
    await needsSecret(t("secret.showKey"), async () => {
      vaultKey = await api.revealVaultKey();
    });
  }

  async function forgetKey() {
    error = null;
    try {
      await api.forgetVaultKey();
      vaultKey = null;
      notice = t("settings.forgot");
    } catch (e) {
      error = api.describeError(e);
    }
  }

  async function previewImport() {
    error = null;
    importPreview = null;
    try {
      if (!selectedProfile) return;
      importPreview = await api.importPreview(selectedProfile.source, selectedProfile.path, importWs);
    } catch (e) {
      error = api.describeError(e);
    }
  }

  async function pickCsv() {
    error = null;
    notice = null;
    pwPreview = null;
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (typeof picked !== "string") return;
    csvPath = picked;
    try {
      pwPreview = await api.passwordImportPreview(picked, importWs);
    } catch (e) {
      csvPath = null;
      error = api.describeError(e);
    }
  }

  async function runPwImport() {
    if (!csvPath) return;
    error = null;
    pwImportBusy = true;
    try {
      const report = await api.passwordImport(csvPath, importWs);
      notice = t("settings.pwImported", { n: report.added });
      csvLeft = csvPath;
      csvPath = null;
      pwPreview = null;
      onchanged?.();
    } catch (e) {
      error = api.describeError(e);
    } finally {
      pwImportBusy = false;
    }
  }

  async function deleteCsv() {
    if (!csvLeft) return;
    try {
      await api.deleteImportFile(csvLeft);
      csvLeft = null;
      notice = t("settings.csvDeleted");
    } catch (e) {
      error = api.describeError(e);
    }
  }

  async function runImport() {
    error = null;
    importBusy = true;
    try {
      if (!selectedProfile) return;
      const report = await api.importBookmarks(
        selectedProfile.source,
        selectedProfile.path,
        importWs,
      );
      notice = t("settings.imported", { add: report.added, skip: report.skipped });
      importPreview = null;
      onchanged?.();
    } catch (e) {
      error = api.describeError(e);
    } finally {
      importBusy = false;
    }
  }

  /// Esc：在快捷鍵頁是回到設定，在設定是關閉
  function onKeydown(e) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (page === "shortcuts") page = "main";
      else onclose();
    }
  }

  const GEN_CLASSES = [
    ["lower", "settings.lower"],
    ["upper", "settings.upper"],
    ["digits", "settings.digits"],
    ["symbols", "settings.symbols"],
  ];
</script>

<div class="backdrop" role="presentation">
  <div
    class="modal"
    role="dialog"
    tabindex="-1"
    aria-modal="true"
    aria-label={t("settings.title")}
    onkeydown={onKeydown}
  >
    <header>
      <strong>{t("settings.title")}</strong>
      <button class="close" onclick={onclose} aria-label={t("common.close")}>✕</button>
    </header>

    {#if secretNeeded}
      <SecretPrompt
        reason={secretNeeded}
        onunlocked={async () => {
          const run = pendingAction;
          secretNeeded = null;
          pendingAction = null;
          try {
            await run?.();
          } catch (e) {
            error = api.describeError(e);
          }
        }}
        oncancel={() => {
          secretNeeded = null;
          pendingAction = null;
        }}
      />
    {:else if !settings}
      <p class="muted pad">{t("common.loading")}</p>
    {:else if page === "shortcuts"}
      <Shortcuts
        onback={() => (page = "main")}
        onchanged={(saved) => {
          // 讓設定頁之後按儲存時不會把快捷鍵蓋回舊值
          settings.hotkey = saved.hotkey;
          settings.shortcuts = saved.shortcuts;
          onchanged?.();
        }}
      />
    {:else}
      <div class="body">
        <section>
          <h2>{t("settings.vault")}</h2>
          <label>
            <span class="lbl">
              {t("settings.path")}
              <span class="muted">· {t("settings.items", { n: vault.item_count })}</span>
            </span>
            <input
              bind:this={firstInput}
              bind:value={settings.vault_path}
              spellcheck="false"
              title={t("settings.restartHint")}
            />
          </label>
          {#if vault.conflict_files?.length}
            <p class="warn">{t("search.conflict", { n: vault.conflict_files.length })}</p>
          {/if}
        </section>

        <section>
          <h2>{t("settings.workspaces")}</h2>
          <ul class="ws">
            {#each workspaces as w (w.id)}
              <li>
                {#if renamingWs === w.id}
                  <input
                    value={w.name}
                    use:autofocus
                    aria-label={t("settings.workspaces")}
                    onkeydown={(e) =>
                      wsInputKeydown(
                        e,
                        (el) => renameWorkspace(w, el),
                        () => (renamingWs = null),
                      )}
                    onblur={(e) => renameWorkspace(w, e.currentTarget)}
                  />
                {:else}
                  <span class="name">{w.name}</span>
                {/if}
                <span class="muted small count">{t("settings.itemCount", { n: w.count })}</span>
                <button
                  class="act"
                  aria-label={t("editor.edit")}
                  title={t("editor.edit")}
                  onclick={() => (renamingWs = w.id)}
                >
                  <svg viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M4 20h4L19 9l-4-4L4 16z" />
                    <path d="M14 6l4 4" />
                  </svg>
                </button>
                <!-- 只有空的能刪：kdbx 刪群組會連裡面的項目一起刪 -->
                {#if w.count === 0 && workspaces.length > 1}
                  <button
                    class="act"
                    aria-label={t("common.delete")}
                    title={t("common.delete")}
                    onclick={() => deleteWorkspace(w)}
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13" />
                    </svg>
                  </button>
                {:else}
                  <span class="act-gap"></span>
                {/if}
              </li>
            {/each}
            <li class="add">
              {#if addingWs}
                <input
                  bind:value={newWsName}
                  use:autofocus
                  placeholder={t("settings.newWorkspace")}
                  onkeydown={(e) =>
                    wsInputKeydown(e, addWorkspace, () => {
                      newWsName = "";
                      addingWs = false;
                    })}
                  onblur={addWorkspace}
                />
              {:else}
                <button class="add-btn" onclick={() => (addingWs = true)}>
                  ＋ {t("settings.newWorkspace")}
                </button>
              {/if}
            </li>
          </ul>
        </section>

        <section>
          <h2>{t("settings.security")}</h2>
          <label class="inline" title={t("settings.idleLockHint")}>
            {t("settings.idleLock")}
            <input type="number" min="0" max="1440" bind:value={settings.lock_after_minutes} />
          </label>
          <label class="check" title={t("settings.askAtStartupHint")}>
            <input type="checkbox" bind:checked={settings.require_password_at_startup} />
            {t("settings.askAtStartup")}
          </label>

          <h3>{t("settings.vaultKey")}</h3>
          <p class="muted small">{t("settings.vaultKeyHint")}</p>
          {#if vaultKey}
            <p class="sample mono">{vaultKey}</p>
          {/if}
          <div class="row">
            {#if vaultKey}
              <button onclick={() => (vaultKey = null)}>{t("settings.hideKey")}</button>
            {:else}
              <button onclick={showVaultKey}>{t("settings.showKey")}</button>
            {/if}
            <button onclick={forgetKey}>{t("settings.forgetKey")}</button>
          </div>

          <h3>{t("settings.changePassword")}</h3>
          <div class="row">
            <input
              type="password"
              bind:value={oldPw}
              placeholder={t("settings.currentPassword")}
              aria-label={t("settings.currentPassword")}
            />
            <input
              type="password"
              bind:value={newPw}
              placeholder={t("settings.newPassword")}
              aria-label={t("settings.newPassword")}
            />
            <input
              type="password"
              bind:value={newPw2}
              placeholder={t("wizard.again")}
              aria-label={t("wizard.again")}
            />
            <button onclick={changePassword} disabled={pwBusy}>{t("settings.change")}</button>
          </div>
        </section>

        <section>
          <h2>{t("settings.gen")}</h2>
          <label class="inline">
            {t("settings.length")}
            <input
              type="range"
              min="8"
              max="64"
              bind:value={settings.password_gen.length}
              oninput={refreshSample}
            />
            <span class="mono num">{settings.password_gen.length}</span>
          </label>
          <div class="checks">
            {#each GEN_CLASSES as [key, label]}
              <label class="check">
                <input
                  type="checkbox"
                  bind:checked={settings.password_gen[key]}
                  onchange={refreshSample}
                />
                {t(label)}
              </label>
            {/each}
          </div>
          <div class="checks">
            <label class="check" title="0O 1lI 5S 2Z 8B">
              <input
                type="checkbox"
                bind:checked={settings.password_gen.exclude_lookalike}
                onchange={refreshSample}
              />
              {t("settings.excludeLookalike")}
            </label>
            <label class="check">
              <input
                type="checkbox"
                bind:checked={settings.password_gen.every_class}
                onchange={refreshSample}
              />
              {t("settings.everyClass")}
            </label>
          </div>
          <div class="row">
            <p class="sample mono">{sample}</p>
            <button onclick={refreshSample}>{t("settings.regenerate")}</button>
          </div>
          <p class="muted small">{t("editor.bits", { n: entropy })}</p>
        </section>

        <section>
          <h2>{t("settings.hotkey")}</h2>
          <div class="row">
            <span class="grow">
              {t("settings.globalHotkey")}
              <span class="mono muted">{keyLabel(settings.hotkey)}</span>
            </span>
            <button onclick={() => (page = "shortcuts")}>{t("settings.editShortcuts")}</button>
          </div>
        </section>

        <section>
          <h2>{t("settings.import")}</h2>
          <div class="row">
            <select bind:value={importWs} onchange={onImportTargetChange} aria-label={t("settings.importInto")}>
              {#each workspaces as w (w.id)}
                <option value={w.id}>{w.name}</option>
              {/each}
            </select>
            <select
              bind:value={importProfile}
              onchange={() => (importPreview = null)}
              disabled={profiles.length === 0}
              aria-label={t("settings.source")}
            >
              {#each profiles as p (p.path)}
                <option value={p.path}>{p.label}</option>
              {:else}
                <option value="">{t("settings.noBrowsers")}</option>
              {/each}
            </select>
            <button onclick={previewImport}>{t("settings.preview")}</button>
            <button onclick={runImport} disabled={importBusy || !importPreview}>
              {importBusy ? t("settings.importing") : t("settings.doImport")}
            </button>
          </div>
          {#if importPreview}
            <p class="muted small">
              {t("settings.importPreview", {
                total: importPreview.total,
                add: importPreview.would_add,
                skip: importPreview.would_skip,
              })}
            </p>
          {/if}
        </section>

        <section>
          <h2>{t("settings.pwImport")}</h2>
          <div class="row">
            <select bind:value={importWs} onchange={onImportTargetChange} aria-label={t("settings.importInto")}>
              {#each workspaces as w (w.id)}
                <option value={w.id}>{w.name}</option>
              {/each}
            </select>
            <button onclick={pickCsv}>{t("settings.pickCsv")}</button>
            <button onclick={runPwImport} disabled={pwImportBusy || !pwPreview || pwPreview.added === 0}>
              {pwImportBusy ? t("settings.importing") : t("settings.doImport")}
            </button>
          </div>
          {#if pwPreview}
            <p class="muted small">
              {t("settings.pwPreview", {
                total: pwPreview.total,
                add: pwPreview.added,
                dup: pwPreview.duplicates,
                empty: pwPreview.no_password,
              })}
            </p>
            {#if pwPreview.notes_dropped > 0}
              <p class="muted small">{t("settings.pwNotes", { n: pwPreview.notes_dropped })}</p>
            {/if}
          {/if}
          {#if csvLeft}
            <p class="warn">{t("settings.csvWarn")}</p>
            <div class="row">
              <button onclick={deleteCsv}>{t("settings.deleteCsv")}</button>
            </div>
          {/if}
        </section>

        <section>
          <h2>{t("settings.appearance")}</h2>
          <label class="inline">
            {t("settings.theme")}
            <select bind:value={settings.theme}>
              <option value="system">{t("settings.themeSystem")}</option>
              <option value="light">{t("settings.themeLight")}</option>
              <option value="dark">{t("settings.themeDark")}</option>
            </select>
          </label>
          <label class="inline">
            {t("settings.language")}
            <select bind:value={settings.language}>
              <option value="auto">{t("settings.languageAuto")}</option>
              {#each LANGUAGES as [code, name]}
                <option value={code}>{name}</option>
              {/each}
            </select>
          </label>
        </section>
      </div>
    {/if}

    {#if error}<p class="error pad" role="alert">{error}</p>{/if}
    {#if notice}<p class="notice pad" role="status">{notice}</p>{/if}

    <footer>
      <span class="spacer"></span>
      <button onclick={onclose}>{t("common.close")}</button>
      <!-- 快捷鍵頁的修改當場就存了 -->
      {#if page === "main"}
        <button onclick={save} disabled={!settings}>{t("common.save")}</button>
      {/if}
    </footer>
  </div>
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
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
  }
  .close {
    margin-left: auto;
    border: none;
    background: none;
    padding: 2px 8px;
  }

  .body {
    overflow-y: auto;
    padding: 4px 16px 16px;
    flex: 1;
    min-height: 0;
  }

  section {
    padding: 12px 0;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  section:last-child {
    border-bottom: none;
  }

  h2 {
    font-size: 0.95em;
    margin: 0;
  }
  h3 {
    font-size: 0.85em;
    margin: 4px 0 0;
    color: var(--fg-dim);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 0.85em;
  }
  label.inline {
    flex-direction: row;
    align-items: center;
    gap: 10px;
  }
  label.inline input[type="number"] {
    width: 80px;
    margin-left: auto;
  }
  label.inline input[type="range"] {
    flex: 1;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  label.check input {
    width: auto;
  }
  .lbl {
    display: flex;
    gap: 6px;
  }

  .checks {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
  }

  /* 外觀在 app.css，這裡只管在設定列裡靠右 */
  label.inline select {
    margin-left: auto;
  }

  .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .row input {
    flex: 1;
    min-width: 0;
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .count {
    flex: none;
    min-width: 4em;
    text-align: right;
  }

  /* 工作區清單 */
  .ws {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }
  .ws li {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 38px;
    padding: 0 6px 0 12px;
  }
  .ws li + li {
    border-top: 1px solid var(--border);
  }
  .ws li:hover .act {
    opacity: 1;
  }
  .ws .name,
  .ws input {
    flex: 1;
    min-width: 0;
  }
  .ws .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ws input {
    margin-left: -8px;
    padding: 4px 7px;
  }
  .ws li.add {
    padding: 0 6px;
  }
  .ws li.add input {
    margin-left: 0;
  }
  .add-btn {
    flex: 1;
    text-align: left;
    background: none;
    border: none;
    padding: 6px;
    color: var(--fg-dim);
  }
  .add-btn:hover {
    color: var(--fg);
  }

  .act,
  .act-gap {
    flex: none;
    width: 24px;
    height: 24px;
  }
  .act {
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    border-radius: var(--radius);
    color: var(--fg-dim);
    opacity: 0.55;
  }
  .act:hover {
    background: var(--bg-hover);
    color: var(--fg);
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

  .sample {
    flex: 1;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 8px 10px;
    margin: 0;
    word-break: break-all;
    font-size: 0.85em;
  }

  .small {
    font-size: 0.78em;
    margin: 0;
  }
  .num {
    min-width: 2.5em;
    text-align: right;
  }

  .warn {
    color: #e0a34a;
    font-size: 0.82em;
    margin: 0;
  }
  .error {
    color: var(--danger);
    margin: 0;
    font-size: 0.85em;
  }
  .notice {
    color: var(--accent);
    margin: 0;
    font-size: 0.85em;
  }
  .pad {
    padding: 0 16px 8px;
  }

  footer {
    display: flex;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid var(--border);
  }
  .spacer {
    flex: 1;
  }
</style>
