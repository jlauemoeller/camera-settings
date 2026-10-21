<script lang="ts">
  import { Pause, Play } from "@lucide/svelte";
  import { app } from "./store.svelte";
  import { duration, modeLabel } from "./format";

  const label = $derived.by(() => {
    switch (app.state) {
      case "recording":
        return duration(app.recording?.elapsedMs ?? 0);
      case "connecting":
        return "Connecting…";
      case "paused":
        return "Paused";
      case "busy":
        return "In use";
      case "error":
        return "Error";
      case "noCamera":
        return "No camera";
      default:
        return null;
    }
  });
  const canPause = $derived(app.state === "streaming" || app.state === "connecting" || app.state === "paused");
</script>

<div class="chip surface" class:recording={app.isRecording}>
  {#if app.isRecording}
    <span class="dot rec"></span>
  {:else}
    <span class="dot" class:live={app.state === "streaming"}></span>
  {/if}
  {#if label}<span class="label" class:mono={app.isRecording}>{label}</span>{/if}
  {#if app.mode && app.state !== "noCamera"}
    <span class="mode">{modeLabel(app.mode)}</span>
  {/if}
  {#if canPause}
    <button
      class="icon-btn small"
      title={app.state === "paused" ? "Resume preview" : "Pause preview and release the camera"}
      onclick={() => app.setPaused(app.state !== "paused")}
    >
      {#if app.state === "paused"}<Play size={14} />{:else}<Pause size={14} />{/if}
    </button>
  {/if}
</div>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 6px 0 12px;
    border-radius: 16px;
    font-size: 13px;
    font-weight: 500;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.35);
  }
  .dot.live {
    background: #4ade80;
  }
  .dot.rec {
    background: var(--red);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .mono {
    font-variant-numeric: tabular-nums;
  }
  .mode {
    color: var(--muted);
  }
  .chip:not(:has(button)) {
    padding-right: 12px;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
</style>
