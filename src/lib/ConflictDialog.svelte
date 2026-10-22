<script lang="ts">
  import { X } from "@lucide/svelte";
  import { app } from "./store.svelte";
  import { settingText } from "./format";

  const rows = $derived(
    (app.conflict ?? []).map((d) => {
      const c = app.controls.find((x) => x.id === d.id);
      return { ...d, camera: settingText(c, d.camera), app: settingText(c, d.app) };
    }),
  );
  let busy = $state(false);

  async function choose(adopt: boolean) {
    busy = true;
    await app.resolveConflict(adopt);
    busy = false;
  }
</script>

<div class="backdrop">
  <div class="dialog surface" role="dialog" aria-modal="true" aria-labelledby="conflict-title">
    <header>
      <h2 id="conflict-title">Camera settings differ</h2>
      <button class="icon-btn small" title="Decide later (leave the camera as it is)" onclick={() => (app.conflictDismissed = true)}>
        <X size={15} />
      </button>
    </header>
    <p>
      Another app is using the camera, and its current settings differ from yours. Nothing has been changed
      yet.
    </p>
    <table>
      <thead>
        <tr><th></th><th>Camera now</th><th>Your settings</th></tr>
      </thead>
      <tbody>
        {#each rows as r (r.id)}
          <tr><td>{r.label}</td><td>{r.camera}</td><td>{r.app}</td></tr>
        {/each}
      </tbody>
    </table>
    <div class="actions">
      <button class="button" disabled={busy} onclick={() => choose(true)}>Keep the camera's settings</button>
      <button class="button primary" disabled={busy} onclick={() => choose(false)}>Apply my settings</button>
    </div>
    <p class="hint">
      "Keep" makes the camera's current values your settings. "Apply" changes the picture the other app is
      showing.
    </p>
  </div>
</div>

<style>
  .backdrop {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.45);
    z-index: 30;
    padding: 16px;
  }
  .dialog {
    width: min(480px, 100%);
    max-height: 100%;
    overflow-y: auto;
    border-radius: 16px;
    padding: 16px 20px 18px;
    background: var(--surface-solid);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  p {
    font-size: 13px;
    line-height: 1.45;
    color: var(--muted);
    margin: 8px 0 12px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th {
    text-align: left;
    font-weight: 500;
    color: var(--muted);
    font-size: 12px;
    padding: 0 8px 6px 0;
  }
  td {
    padding: 6px 8px 6px 0;
    border-top: 1px solid var(--border);
    font-variant-numeric: tabular-nums;
  }
  td:first-child {
    color: var(--muted);
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 16px;
    flex-wrap: wrap;
  }
  .button.primary {
    background: rgba(255, 255, 255, 0.92);
    color: #111;
  }
  .button.primary:hover:not(:disabled) {
    background: #fff;
  }
  .hint {
    font-size: 12px;
    margin: 10px 0 0;
  }
</style>
