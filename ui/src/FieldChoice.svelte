<script>
  // 問密碼項目要送哪一個欄位。
  import { tick } from "svelte";
  import { t } from "./i18n.svelte.js";

  let { item, onpick, oncancel } = $props();

  const OPTIONS = [
    { field: "username", label: "editor.username", value: item.username },
    { field: "body", label: "editor.password", value: null },
  ];

  let cursor = $state(0);
  let box;

  $effect(() => {
    tick().then(() => box?.focus());
  });

  function onKeydown(e) {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        cursor = (cursor + 1) % OPTIONS.length;
        break;
      case "ArrowUp":
        e.preventDefault();
        cursor = (cursor + OPTIONS.length - 1) % OPTIONS.length;
        break;
      case "Enter":
        e.preventDefault();
        onpick(OPTIONS[cursor].field);
        break;
      case "Escape":
        e.preventDefault();
        e.stopPropagation();
        oncancel();
        break;
      case "1":
      case "2":
        e.preventDefault();
        onpick(OPTIONS[Number(e.key) - 1].field);
        break;
    }
  }
</script>

<div
  class="wrap"
  bind:this={box}
  tabindex="-1"
  role="listbox"
  aria-label={t("choice.title")}
  onkeydown={onKeydown}
>
  <div class="head">
    <strong>{item.title}</strong>
    <span class="muted">{t("choice.title")}</span>
  </div>

  <ul>
    {#each OPTIONS as opt, i (opt.field)}
      <li
        role="option"
        aria-selected={i === cursor}
        class:sel={i === cursor}
        onclick={() => onpick(opt.field)}
        onmouseenter={() => (cursor = i)}
      >
        <span class="num muted">{i + 1}</span>
        <span class="label">{t(opt.label)}</span>
        {#if opt.value}<span class="value muted">{opt.value}</span>{/if}
      </li>
    {/each}
  </ul>

  <p class="muted hint">{t("choice.hint")}</p>
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    padding: 16px 20px;
    justify-content: center;
    gap: 10px;
    outline: none;
  }

  .head {
    display: flex;
    gap: 12px;
    align-items: baseline;
    font-size: 0.9em;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  li {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--radius);
    cursor: default;
  }
  li.sel {
    background: var(--bg-sel);
  }

  .num {
    flex: none;
    font-size: 0.78em;
    min-width: 1em;
  }
  .label {
    flex: none;
    font-weight: 600;
  }
  .value {
    flex: 1 1 auto;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 0.85em;
  }

  .hint {
    margin: 0;
    font-size: 0.78em;
  }
</style>
