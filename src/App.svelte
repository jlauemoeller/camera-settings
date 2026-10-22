<script lang="ts">
  import { onMount } from "svelte";
  import { SlidersHorizontal, Info, Power } from "@lucide/svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { listen } from "@tauri-apps/api/event";
  import { api } from "./lib/api";
  import { app } from "./lib/store.svelte";
  import Viewport from "./lib/Viewport.svelte";
  import StatusChip from "./lib/StatusChip.svelte";
  import CaptureBar from "./lib/CaptureBar.svelte";
  import SettingsDrawer from "./lib/SettingsDrawer.svelte";
  import Toasts from "./lib/Toasts.svelte";
  import InfoOverlay from "./lib/InfoOverlay.svelte";
  import ConflictDialog from "./lib/ConflictDialog.svelte";

  let drawerOpen = $state(false);

  // While another app has the stream, the settings are the only thing to work with: open them
  // once per busy episode (closing the drawer is respected until the camera frees up again).
  let wasBusy = false;
  $effect(() => {
    const busy = app.state === "busy" && app.controlsAvailable;
    if (busy && !wasBusy) drawerOpen = true;
    wasBusy = busy;
  });
  let infoOpen = $state(false);

  // No window decorations on some compositors (niri), so the app needs its own way out.
  // A running recording is stopped and saved by the backend's shutdown path.
  function quit() {
    void api.quit();
  }

  // The webview's own context menu (Reload, Inspect Element) only confuses users; `--debug` keeps it.
  function onContextMenu(e: MouseEvent) {
    if (!app.debug) e.preventDefault();
  }

  async function toggleFullscreen() {
    const w = getCurrentWindow();
    await w.setFullscreen(!(await w.isFullscreen()));
  }

  function onKey(e: KeyboardEvent) {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" && (t as HTMLInputElement).type === "text")) return;
    if (e.ctrlKey && ["q", "w"].includes(e.key.toLowerCase())) {
      e.preventDefault();
      quit();
      return;
    }
    if (e.ctrlKey && e.key === ",") {
      e.preventDefault();
      drawerOpen = !drawerOpen;
      return;
    }
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    switch (e.key) {
      case " ":
        e.preventDefault();
        app.shutter();
        break;
      case "p":
      case "P":
        app.setCaptureKind("photo");
        break;
      case "v":
      case "V":
        app.setCaptureKind("video");
        break;
      case "m":
      case "M":
        app.toggleAudio();
        break;
      case "s":
      case "S":
        drawerOpen = !drawerOpen;
        break;
      case "Escape":
        drawerOpen = false;
        break;
      case "i":
      case "I":
        infoOpen = !infoOpen;
        break;
      case "f":
      case "F":
      case "F11":
        e.preventDefault();
        void toggleFullscreen();
        break;
    }
  }

  onMount(() => {
    void app.init();
    if (import.meta.env.DEV) {
      // Headless UI driving from the backend's debug console (debug builds only).
      void listen<string>("debug-ui", ({ payload }) => {
        const [cmd, arg] = payload.split(" ");
        if (cmd === "drawer") drawerOpen = !drawerOpen;
        else if (cmd === "info") infoOpen = !infoOpen;
        else if (cmd === "quit") quit();
        else if (cmd === "scroll") document.querySelector(".scroll")?.scrollTo(0, Number(arg));
      });
    }
    const onVisibility = () => void api.setHidden(document.hidden);
    document.addEventListener("visibilitychange", onVisibility);
    return () => document.removeEventListener("visibilitychange", onVisibility);
  });
</script>

<svelte:window onkeydown={onKey} oncontextmenu={onContextMenu} />

<main>
  <Viewport />

  <div class="overlay">
    <div
      class="top"
      role="toolbar"
      tabindex="-1"
    >
      <StatusChip />
      <div class="top-right">
        <button class="icon-btn surface round" class:active={infoOpen} title="Info (I)" onclick={() => (infoOpen = !infoOpen)}>
          <Info size={18} />
        </button>
        <button class="icon-btn surface round" title="Settings (S)" onclick={() => (drawerOpen = !drawerOpen)}>
          <SlidersHorizontal size={18} />
        </button>
        <button
          class="icon-btn surface round quit"
          title={app.isRecording ? "Stop recording and quit (Ctrl+Q)" : "Quit (Ctrl+Q)"}
          aria-label="Quit"
          onclick={quit}
        >
          <Power size={18} />
        </button>
      </div>
    </div>

    {#if infoOpen}
      <div class="info-pos"><InfoOverlay /></div>
    {/if}

    <div class="bottom">
      <Toasts />
      <div
        role="toolbar"
        tabindex="-1"
      >
        <CaptureBar />
      </div>
    </div>
  </div>

  {#if drawerOpen}
    <SettingsDrawer onclose={() => (drawerOpen = false)} />
  {/if}

  {#if app.conflict && !app.conflictDismissed}
    <ConflictDialog />
  {/if}
</main>

<style>
  main {
    position: fixed;
    inset: 0;
    overflow: hidden;
    background: #000;
  }
  .overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .top {
    position: absolute;
    top: 16px;
    left: 16px;
    right: 16px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    pointer-events: auto;
    outline: none;
  }
  .top-right {
    display: flex;
    gap: 8px;
  }
  /* Keep Quit well apart from Settings so a near miss doesn't close the app. */
  .quit {
    margin-left: 24px;
  }
  .info-pos {
    position: absolute;
    top: 60px;
    left: 16px;
    pointer-events: auto;
  }
  .bottom {
    position: absolute;
    bottom: 24px;
    left: 0;
    right: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
  }
  .bottom > [role="toolbar"] {
    pointer-events: auto;
    outline: none;
  }
  .round {
    width: 40px;
    height: 40px;
    border-radius: 50%;
  }
</style>
