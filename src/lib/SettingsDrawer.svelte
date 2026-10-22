<script lang="ts">
  import type { Component } from "svelte";
  import { X, Usb, FolderOpen, RotateCcw, Scan, ZoomIn, Sun, Focus, Thermometer } from "@lucide/svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import type { Control, Group, Mode } from "./api";
  import { app } from "./store.svelte";
  import { fps, fpsLabel, sameMode, sizeLabel, usbLabel } from "./format";
  import ControlRow from "./ControlRow.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const ICONS: Record<string, Component<{ size?: number }>> = {
    fov: Scan,
    zoom: ZoomIn,
    exposure_time: Sun,
    brightness: Sun,
    focus: Focus,
    white_balance_temperature: Thermometer,
  };

  const GROUPS: { id: Group; title: string }[] = [
    { id: "framing", title: "Framing" },
    { id: "exposure", title: "Exposure" },
    { id: "focus", title: "Focus" },
    { id: "color", title: "Color" },
  ];

  const byGroup = (g: Group) => app.controls.filter((c) => c.group === g);
  const zoom = $derived(app.controls.find((c) => c.id === "zoom"));
  const atMinZoom = $derived(!!zoom && zoom.value <= zoom.min);
  const blocked = (c: Control) => (c.id === "pan" || c.id === "tilt") && atMinZoom;

  // Format: pick a resolution, then a frame rate available for it.
  const sizes = $derived.by(() => {
    const seen = new Map<string, { width: number; height: number }>();
    for (const m of app.modes) seen.set(`${m.width}x${m.height}`, { width: m.width, height: m.height });
    return [...seen.values()];
  });
  const current = $derived(app.mode);
  const rates = $derived(
    current ? app.modes.filter((m) => m.width === current.width && m.height === current.height) : [],
  );
  // The stream mode belongs to whoever streams: locked while recording or another app has the camera.
  const modeLocked = $derived(app.isRecording || app.state === "noCamera" || app.state === "busy");

  function pickSize(s: { width: number; height: number }) {
    const candidates = app.modes.filter((m) => m.width === s.width && m.height === s.height);
    const want = current ? fps(current) : 30;
    const next: Mode | undefined =
      candidates.find((m) => fps(m) === want) ?? candidates.find((m) => fps(m) === 30) ?? candidates[0];
    if (next && !sameMode(next, current)) app.setMode(next);
  }

  async function chooseFolder(kind: "photo" | "video") {
    const prefs = app.prefs;
    if (!prefs) return;
    const dir = await open({
      directory: true,
      defaultPath: kind === "photo" ? prefs.photoDir : prefs.videoDir,
      title: kind === "photo" ? "Folder for photos" : "Folder for videos",
    });
    if (typeof dir === "string") await app.setPrefs(kind === "photo" ? { photoDir: dir } : { videoDir: dir });
  }

  /** "/home/me/Pictures" → "~/Pictures" (home taken from the path itself; no extra API). */
  function tildify(path: string) {
    const m = path.match(/^\/home\/[^/]+(\/.*)?$/);
    return m ? `~${m[1] ?? ""}` : path;
  }

  const resetFolder = (kind: "photo" | "video") => app.setPrefs(kind === "photo" ? { photoDir: "" } : { videoDir: "" });
</script>

<aside class="drawer surface" aria-label="Settings">
  <header>
    <h2>Settings</h2>
    <button class="icon-btn" title="Close (Esc)" onclick={onclose}><X size={18} /></button>
  </header>

  <div class="scroll">
    <section>
      <h3>Format</h3>
      {#if app.usbSpeed > 0 && app.usbSpeed <= 480}
        <div class="notice">
          <Usb size={16} />
          <span>Connected at USB 2. Use a USB 3 cable and port for 4K and 60 fps.</span>
        </div>
      {/if}
      {#if sizes.length}
        <div class="sizes" role="radiogroup" aria-label="Resolution">
          {#each sizes as s (`${s.width}x${s.height}`)}
            {@const active = current?.width === s.width && current?.height === s.height}
            <button role="radio" aria-checked={active} class:active disabled={modeLocked} onclick={() => pickSize(s)}>
              <span>{sizeLabel(s)}</span>

            </button>
          {/each}
        </div>
        {#if rates.length > 1}
          <div class="sub">Frame rate</div>
          <div class="segmented rates" role="radiogroup" aria-label="Frame rate">
            {#each rates as m (fps(m))}
              <button
                role="radio"
                aria-checked={sameMode(m, current)}
                class:active={sameMode(m, current)}
                disabled={modeLocked}
                onclick={() => app.setMode(m)}
              >
                {fpsLabel(m)}
              </button>
            {/each}
          </div>
        {/if}
        {#if app.isRecording}<p class="hint">The mode is locked while recording.</p>{/if}
        {#if app.state === "busy"}
          <p class="hint">The app using the camera sets the resolution. The other settings below still apply to its picture.</p>
        {/if}
      {:else}
        <p class="hint">Connect the camera to see its modes.</p>
      {/if}
    </section>

    {#each GROUPS as g (g.id)}
      {@const controls = byGroup(g.id)}
      {#if controls.length}
        <section>
          <h3>{g.title}</h3>
          {#each controls as c (c.id)}
            <ControlRow control={c} icon={ICONS[c.id]} blocked={blocked(c)} blockedReason="Zoom in to pan and tilt" />
          {/each}
        </section>
      {/if}
    {/each}

    {#if app.prefs}
      <section>
        <h3>Output</h3>
        {#each [{ kind: "photo", label: "Photos", dir: app.prefs.photoDir, isDefault: app.prefs.photoDirIsDefault }, { kind: "video", label: "Videos", dir: app.prefs.videoDir, isDefault: app.prefs.videoDirIsDefault }] as const as f (f.kind)}
          <div class="folder">
            <div class="folder-text">
              <span>{f.label}</span>
              <span class="dim path" title={f.dir}>{tildify(f.dir)}</span>
            </div>
            {#if !f.isDefault}
              <button class="icon-btn small" title="Use the default folder" onclick={() => resetFolder(f.kind)}>
                <RotateCcw size={13} />
              </button>
            {/if}
            <button class="icon-btn" title="Choose folder" onclick={() => chooseFolder(f.kind)}>
              <FolderOpen size={17} />
            </button>
          </div>
        {/each}
        <div class="sub">Video quality</div>
        <div class="segmented full" role="radiogroup" aria-label="Video quality">
          <button
            role="radio"
            aria-checked={app.prefs.videoQuality === "original"}
            class:active={app.prefs.videoQuality === "original"}
            disabled={app.isRecording}
            onclick={() => app.setPrefs({ videoQuality: "original" })}>Original</button
          >
          {#if app.compactAvailable}
            <button
              role="radio"
              aria-checked={app.prefs.videoQuality === "compact"}
              class:active={app.prefs.videoQuality === "compact"}
              disabled={app.isRecording}
              onclick={() => app.setPrefs({ videoQuality: "compact" })}>Compact</button
            >
          {/if}
        </div>
        <p class="hint">
          {app.prefs.videoQuality === "original"
            ? "The camera's own MJPEG frames, not re-encoded. Large files (about 1.8 GB per minute at 4K)."
            : "AV1, much smaller files. Re-encoded, so slightly less detail."}
        </p>
      </section>
    {/if}

    {#if app.device}
      <section>
        <h3>Camera info</h3>
        <dl>
          <dt>Model</dt><dd>{app.device.model}</dd>
          <dt>Serial</dt><dd>{app.device.serial || "–"}</dd>
          <dt>Firmware</dt><dd>{app.device.firmware || "–"}</dd>
          <dt>USB link</dt><dd>{usbLabel(app.device.usbSpeedMbps)}</dd>
          <dt>Device</dt><dd>{app.device.node}</dd>
        </dl>
      </section>
    {/if}

    <section class="footer">
      <button class="button" disabled={!app.controlsAvailable} onclick={() => app.resetAll()}>
        <RotateCcw size={15} /> Reset all
      </button>
    </section>
  </div>
</aside>

<style>
  .drawer {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 340px;
    max-width: 100vw;
    display: flex;
    flex-direction: column;
    border-radius: 0;
    border-left: 1px solid var(--border);
    z-index: 20;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 12px 8px 20px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  .scroll {
    overflow-y: auto;
    padding: 0 20px 20px;
    flex: 1;
  }
  section {
    padding: 12px 0;
    border-bottom: 1px solid var(--border);
  }
  section:last-child {
    border-bottom: none;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .sub {
    font-size: 12px;
    color: var(--muted);
    margin: 12px 0 6px;
  }
  .hint {
    font-size: 12px;
    color: var(--muted);
    margin: 8px 0 0;
    line-height: 1.4;
  }
  .notice {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    font-size: 12px;
    line-height: 1.4;
    padding: 10px 12px;
    border-radius: 10px;
    background: rgba(250, 204, 21, 0.12);
    color: #fde68a;
    margin: 6px 0 10px;
  }
  .notice :global(svg) {
    flex: none;
    margin-top: 1px;
  }
  .sizes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2px 6px;
  }
  .sizes button {
    display: flex;
    justify-content: space-between;
    padding: 7px 10px;
    border-radius: 8px;
    font-size: 13px;
    color: var(--fg);
    text-align: left;
  }
  .sizes button:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.07);
  }
  .sizes button.active {
    background: rgba(255, 255, 255, 0.16);
  }
  .sizes button:disabled:not(.active) {
    opacity: 0.45;
  }
  .dim {
    color: var(--muted);
    font-size: 12px;
  }
  .rates {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
  }
  .folder {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 0;
  }
  .folder-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    font-size: 13px;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 16px;
    margin: 4px 0 0;
    font-size: 13px;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .footer {
    display: flex;
    justify-content: center;
    padding-top: 16px;
  }
</style>
