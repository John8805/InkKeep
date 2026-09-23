<script>
  // 首次啟動精靈：保險庫位置 → 主密碼 → 完成。
  //
  // 保險庫在最後一步才建立；中途關閉視窗，下次啟動會重新顯示精靈。
  import { open } from "@tauri-apps/plugin-dialog";
  import { tick } from "svelte";
  import * as api from "./api.js";
  import { t } from "./i18n.svelte.js";

  let { defaultPath, onDone } = $props();

  let step = $state(1);
  let path = $state(defaultPath);
  let pathNote = $state(null);
  let password = $state("");
  let confirm = $state("");
  let withSamples = $state(true);
  let error = $state(null);
  let busy = $state(false);
  let focusTarget;

  $effect(() => {
    step;
    tick().then(() => focusTarget?.focus());
  });

  const strength = $derived.by(() => {
    const p = password;
    if (!p) return null;
    const classes = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(p)).length;
    if (p.length < 8) return { key: "strength.short", level: 0 };
    if (p.length >= 16 && classes >= 3) return { key: "strength.strong", level: 2 };
    if (p.length >= 12 || classes >= 3) return { key: "strength.ok", level: 1 };
    return { key: "strength.weak", level: 0 };
  });

  async function pickFolder() {
    const dir = await open({ directory: true, multiple: false, title: t("wizard.pick") });
    if (typeof dir === "string") {
      // 用反斜線接檔名，與對話框回傳的 Windows 路徑分隔符一致
      path = `${dir.replace(/[\\/]+$/, "")}\\vault.kdbx`;
      await checkPath();
    }
  }

  async function checkPath() {
    pathNote = null;
    error = null;
    try {
      const r = await api.checkVaultPath(path);
      if (r.exists) {
        pathNote = t("wizard.exists");
      } else if (!r.parent_ok) {
        error = t("wizard.badParent");
      }
    } catch (e) {
      error = api.describeError(e);
    }
  }

  async function next() {
    error = null;
    if (step === 1) {
      await api.setVaultPath(path);
      await checkPath();
      if (error) return;
      step = 2;
      return;
    }
    if (step === 2) {
      if (password.length < 8) {
        error = t("wizard.tooShort");
        return;
      }
      if (password !== confirm) {
        error = t("wizard.mismatch");
        return;
      }
      busy = true;
      try {
        await api.createVault(password, withSamples);
        password = confirm = "";
        await onDone();
      } catch (e) {
        error = api.describeError(e);
      } finally {
        busy = false;
      }
    }
  }

  function onKeydown(e) {
    if (e.key === "Enter" && !busy) {
      e.preventDefault();
      next();
    }
  }
</script>

<div class="wrap" onkeydown={onKeydown} role="group" aria-label={t("wizard.whereTitle")}>
  <ol class="steps">
    <li class:on={step === 1}>{t("wizard.stepLocation")}</li>
    <li class:on={step === 2}>{t("wizard.stepPassword")}</li>
  </ol>

  {#if step === 1}
    <h1>{t("wizard.whereTitle")}</h1>
    <p class="muted">{t("wizard.whereHint")}</p>
    <label>
      {t("wizard.path")}
      <input bind:this={focusTarget} bind:value={path} onchange={checkPath} spellcheck="false" />
    </label>
    <button type="button" onclick={pickFolder}>{t("wizard.pick")}</button>
    {#if pathNote}<p class="note">{pathNote}</p>{/if}
  {:else}
    <h1>{t("wizard.passwordTitle")}</h1>
    <p class="muted">{t("wizard.passwordHint")}</p>
    <p class="warn">{t("wizard.passwordWarn")}</p>
    <label>
      {t("common.masterPassword")}
      <input
        bind:this={focusTarget}
        bind:value={password}
        type="password"
        autocomplete="new-password"
        disabled={busy}
      />
    </label>
    {#if strength}
      <p class="strength" data-level={strength.level}>
        {t("strength.label")}: {t(strength.key)}
      </p>
    {/if}
    <label>
      {t("wizard.again")}
      <input bind:value={confirm} type="password" autocomplete="new-password" disabled={busy} />
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={withSamples} disabled={busy} />
      {t("wizard.samples")}
    </label>
  {/if}

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  <div class="actions">
    {#if step > 1}
      <button type="button" onclick={() => (step -= 1)} disabled={busy}>{t("common.back")}</button>
    {/if}
    <span class="spacer"></span>
    <button type="button" onclick={next} disabled={busy}>
      {busy ? t("wizard.creating") : step === 2 ? t("wizard.create") : t("common.next")}
    </button>
  </div>
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 520px;
    margin: auto;
    padding: 32px;
    width: 100%;
  }

  .steps {
    display: flex;
    gap: 8px;
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 0.78em;
    color: var(--fg-dim);
  }
  .steps li {
    padding: 2px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
  }
  .steps li.on {
    border-color: var(--accent);
    color: var(--accent);
  }

  h1 {
    font-size: 1.2em;
    margin: 0;
  }

  p {
    margin: 0;
    font-size: 0.85em;
    line-height: 1.5;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 0.85em;
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  label.check input {
    width: auto;
  }

  .warn {
    color: #e0a34a;
  }
  .note {
    color: var(--accent);
  }
  .error {
    color: var(--danger);
  }

  .strength[data-level="0"] {
    color: var(--danger);
  }
  .strength[data-level="1"] {
    color: #e0a34a;
  }
  .strength[data-level="2"] {
    color: #5bc87a;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
  .spacer {
    flex: 1;
  }
</style>
