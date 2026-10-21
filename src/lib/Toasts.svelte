<script lang="ts">
  import { FolderOpen, TriangleAlert, X } from "@lucide/svelte";
  import { api } from "./api";
  import { app } from "./store.svelte";
</script>

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast surface" class:error={t.kind === "error"}>
      {#if t.kind === "error"}<TriangleAlert size={16} />{/if}
      <span class="text">{t.text}</span>
      {#if t.path}
        <button
          class="link"
          onclick={() => t.path && void api.reveal(t.path).catch((e) => app.toast(String(e), "error"))}
        >
          <FolderOpen size={15} /> Show in folder
        </button>
      {/if}
      <button class="icon-btn small" title="Dismiss" onclick={() => app.dismiss(t.id)}><X size={14} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 14px;
    border-radius: 12px;
    font-size: 13px;
    max-width: min(560px, 90vw);
    pointer-events: auto;
  }
  .toast.error {
    color: #fecaca;
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: #fff;
    font-weight: 600;
    white-space: nowrap;
    padding: 4px 8px;
    border-radius: 8px;
  }
  .link:hover {
    background: rgba(255, 255, 255, 0.12);
  }
</style>
