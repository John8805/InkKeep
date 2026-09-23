<script>
  // 表單模式：填寫模板的輸入欄位，旁邊即時預覽結果。
  import { tick } from "svelte";
  import * as api from "./api.js";
  import { t } from "./i18n.svelte.js";

  let { item, plan, onsubmit, oncancel } = $props();

  // label → 值，預設帶入模板給的 default
  let values = $state(
    Object.fromEntries(
      plan.needs_inputs.map((f) => [
        f.label,
        f.type === "select" ? f.options[0] : (f.default ?? ""),
      ]),
    ),
  );

  let previewText = $state("");
  let first;

  $effect(() => {
    tick().then(() => first?.focus());
  });

  $effect(() => {
    const snapshot = { ...values };
    let cancelled = false;
    api
      .preview(item.id, snapshot)
      .then((text) => {
        if (!cancelled) previewText = text;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  });

  function submit(e) {
    e?.preventDefault();
    onsubmit({ ...values });
  }

  function onKeydown(e) {
    const ctrl = e.ctrlKey || e.metaKey;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      oncancel();
      return;
    }
    if (e.key === "Enter" && (ctrl || e.target.tagName !== "TEXTAREA")) {
      e.preventDefault();
      e.stopPropagation();
      submit();
    }
  }
</script>

<form class="wrap" onsubmit={submit} onkeydown={onKeydown}>
  <div class="head">
    <strong>{item.title}</strong>
    <span class="muted">{t("form.hint")}</span>
  </div>

  <div class="body">
    <div class="fields">
      {#each plan.needs_inputs as field, i (field.label)}
        <label>
          {field.label}
          {#if field.type === "select"}
            <!-- 只有第一個欄位綁 first，否則 each 迴圈會一路覆寫成最後一個 -->
            {#if i === 0}
              <select bind:value={values[field.label]} bind:this={first}>
                {#each field.options as opt}<option value={opt}>{opt}</option>{/each}
              </select>
            {:else}
              <select bind:value={values[field.label]}>
                {#each field.options as opt}<option value={opt}>{opt}</option>{/each}
              </select>
            {/if}
          {:else if i === 0}
            <input bind:value={values[field.label]} bind:this={first} autocomplete="off" />
          {:else}
            <input bind:value={values[field.label]} autocomplete="off" />
          {/if}
        </label>
      {/each}
    </div>

    <aside class="preview">
      <pre>{previewText}</pre>
    </aside>
  </div>
</form>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .head {
    display: flex;
    gap: 12px;
    align-items: baseline;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 0.9em;
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .fields {
    flex: 1 1 60%;
    padding: 12px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 0.85em;
  }

  select {
    font: inherit;
    color: inherit;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 8px 10px;
  }

  .preview {
    flex: 0 0 40%;
    border-left: 1px solid var(--border);
    padding: 10px;
    overflow: auto;
  }
  .preview pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: "Cascadia Code", Consolas, monospace;
    font-size: 0.85em;
  }
</style>
