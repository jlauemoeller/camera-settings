<script lang="ts">
  import { app } from "./store.svelte";
  import { usbLabel } from "./format";

  const s = $derived(app.stats);
</script>

<div class="info surface">
  <div><span>resolution</span>{s?.resolution ?? "–"}</div>
  <div><span>format</span>{s?.format ?? "–"}</div>
  <div><span>camera fps</span>{s ? s.fps.toFixed(1) : "–"}</div>
  <div><span>display fps</span>{app.displayFps}</div>
  <div><span>dropped</span>{s?.dropped ?? "–"}</div>
  <div><span>usb</span>{usbLabel(s?.usbSpeed ?? app.usbSpeed)}</div>
  {#if s?.bitrate}
    <div><span>rec bitrate</span>{(s.bitrate / 1e6).toFixed(1)} Mbit/s</div>
  {/if}
</div>

<style>
  .info {
    font-family: ui-monospace, "SF Mono", "Cascadia Code", "DejaVu Sans Mono", monospace;
    font-size: 12px;
    line-height: 1.6;
    padding: 10px 14px;
    border-radius: 10px;
    min-width: 230px;
  }
  span {
    display: inline-block;
    width: 96px;
    color: var(--muted);
  }
</style>
