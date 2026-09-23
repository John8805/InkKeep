<script>
  // 第一層解鎖：輸入主密碼開保險庫。
  import * as api from "./api.js";
  import { t } from "./i18n.svelte.js";

  let { path, onUnlocked } = $props();

  let password = $state("");
  let error = $state(null);
  let busy = $state(false);
  let input;

  $effect(() => {
    input?.focus();
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
      await api.unlock(password);
      password = "";
      await onUnlocked();
    } catch (e) {
      error = api.describeError(e);
    } finally {
      busy = false;
    }
  }
</script>

<form class="wrap" onsubmit={submit}>
  <h1>{t("unlock.title")}</h1>
  <p class="muted path mono">{path}</p>

  <input
    bind:this={input}
    bind:value={password}
    type="password"
    autocomplete="current-password"
    aria-label={t("secret.title")}
    disabled={busy}
  />

  <p class="muted hint">{t("unlock.hint")}</p>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <button type="submit" disabled={busy}>
    {busy ? t("common.unlocking") : t("common.unlock")}
  </button>
</form>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 420px;
    margin: auto;
    padding: 32px;
    width: 100%;
  }

  h1 {
    font-size: 1.2em;
    margin: 0;
  }

  .path {
    font-size: 0.78em;
    margin: 0;
    word-break: break-all;
  }

  .hint {
    font-size: 0.8em;
    margin: 0;
  }

  .error {
    color: var(--danger);
    margin: 0;
    font-size: 0.85em;
  }
</style>
