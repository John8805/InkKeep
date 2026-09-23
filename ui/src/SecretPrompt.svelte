<script>
  // 第二層解鎖：輸入主密碼解開密碼項目，維持到閒置鎖定。
  import { tick } from "svelte";
  import * as api from "./api.js";
  import { t } from "./i18n.svelte.js";

  let { reason = "", onunlocked, oncancel } = $props();

  let password = $state("");
  let error = $state(null);
  let busy = $state(false);
  let input;

  $effect(() => {
    tick().then(() => input?.focus());
  });

  async function submit(e) {
    e?.preventDefault();
    if (busy) return;
    error = null;
    if (!password) {
      error = t("secret.required");
      return;
    }
    busy = true;
    try {
      // Argon2id 64 MiB 要跑幾百毫秒
      await api.unlockSecrets(password);
      password = "";
      await onunlocked();
    } catch (err) {
      error = api.describeError(err);
    } finally {
      busy = false;
    }
  }

  function onKeydown(e) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      oncancel();
    }
  }
</script>

<form class="wrap" onsubmit={submit} onkeydown={onKeydown}>
  <h2>{t("secret.title")}</h2>
  {#if reason}<p class="muted">{reason}</p>{/if}

  <div class="row">
    <input
      bind:this={input}
      bind:value={password}
      type="password"
      autocomplete="current-password"
      aria-label={t("secret.title")}
      disabled={busy}
    />
    <button type="button" onclick={oncancel} disabled={busy}>{t("common.cancel")}</button>
    <button type="submit" disabled={busy}>
      {busy ? t("common.unlocking") : t("common.unlock")}
    </button>
  </div>

  {#if error}<p class="error" role="alert">{error}</p>{/if}
</form>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 20px 24px;
    flex: 1;
    min-height: 0;
    justify-content: center;
  }

  h2 {
    font-size: 1.05em;
    margin: 0;
  }

  p {
    margin: 0;
    font-size: 0.85em;
  }

  .row {
    display: flex;
    gap: 8px;
  }
  .row input {
    flex: 1;
    min-width: 0;
  }

  .error {
    color: var(--danger);
  }
</style>
