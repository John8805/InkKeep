<script>
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import * as api from "./api.js";
  import { t, setLanguage } from "./i18n.svelte.js";
  import { setOverrides } from "./shortcuts.svelte.js";
  import Unlock from "./Unlock.svelte";
  import Wizard from "./Wizard.svelte";
  import Search from "./Search.svelte";

  let vault = $state(null);
  let hotkeyConflict = $state(null);
  let conflictDismissed = $state(false);

  async function refresh() {
    vault = await api.vaultState();
    // 啟動時的註冊失敗發生在畫面載入之前，事件收不到，從狀態補讀；改好快捷鍵後也從這裡清掉
    hotkeyConflict = vault.hotkey_conflict ?? null;
  }

  /// 讀取 settings.toml 的主題、語言與快捷鍵，套用到目前畫面。
  async function applyPreferences() {
    try {
      const s = await api.settingsGet();
      document.documentElement.dataset.theme = s.theme ?? "system";
      setLanguage(s.language);
      setOverrides(s.shortcuts);
    } catch {
      document.documentElement.dataset.theme = "system";
      setLanguage("auto");
    }
  }

  onMount(async () => {
    await applyPreferences();
    await refresh();

    // 快捷鍵叫出視窗時重新整理狀態：可能在背景被閒置鎖定了
    const unlistenShown = await listen("window:shown", refresh);
    const unlistenLocked = await listen("vault:locked", refresh);
    const unlistenConflict = await listen("hotkey:conflict", (e) => {
      hotkeyConflict = e.payload;
      conflictDismissed = false;
    });

    return () => {
      unlistenShown();
      unlistenLocked();
      unlistenConflict();
    };
  });
</script>

<main>
  {#if hotkeyConflict && !conflictDismissed}
    <div class="banner" role="alert">
      {t("error.hotkeyConflict", { combo: hotkeyConflict })}
      <button onclick={() => (conflictDismissed = true)}>{t("common.close")}</button>
    </div>
  {/if}

  {#if !vault}
    <div class="center muted">{t("common.loading")}</div>
  {:else if vault.locked && !vault.exists}
    <Wizard defaultPath={vault.path} onDone={refresh} />
  {:else if vault.locked}
    <Unlock path={vault.path} onUnlocked={refresh} />
  {:else}
    <Search {vault} onvaultchanged={refresh} onpreferenceschanged={applyPreferences} />
  {/if}
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .center {
    display: grid;
    place-items: center;
    height: 100%;
  }

  .banner {
    background: #7a4a00;
    color: #fff;
    padding: 8px 12px;
    display: flex;
    gap: 12px;
    align-items: center;
    font-size: 0.9em;
  }
  .banner button {
    margin-left: auto;
  }
</style>
