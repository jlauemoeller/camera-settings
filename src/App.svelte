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

  let drawerOpen = $state(false);
  let infoOpen = $state(false);
  let idle = $state(false);
  let hovering = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  // SPEC §7.1: controls fade after 3 s without mouse movement, except while recording, while a
  // panel is open, or while the pointer rests on a control.
  const keepVisible = $derived(app.isRecording || drawerOpen || hovering || !app.live);
  const hidden = $derived(idle && !keepVisible);

  function poke() {
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (idle = true), 3000);
  }

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
    poke();
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
        else if (cmd === "show") poke();
        else if (cmd === "quit") quit();
        else if (cmd === "scroll") document.querySelector(".scroll")?.scrollTo(0, Number(arg));
      });
    }
    const onVisibility = () => void api.setHidden(document.hidden);
    document.addEventListener("visibilitychange", onVisibility);
    poke();
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      clearTimeout(idleTimer);
    };
  });
</script>

<svelte:window onkeydown={onKey} onmousemove={poke} onmousedown={poke} oncontextmenu={onContextMenu} />

<main class:cursor-hidden={hidden}>
  <Viewport />

  <div class="overlay" class:faded={hidden}>
    <div
      class="top"
      role="toolbar"
      tabindex="-1"
      onmouseenter={() => (hovering = true)}
      onmouseleave={() => (hovering = false)}
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
          class="icon-btn surface round"
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
        onmouseenter={() => (hovering = true)}
        onmouseleave={() => (hovering = false)}
      >
        <CaptureBar />
      </div>
    </div>
  </div>

  {#if drawerOpen}
    <SettingsDrawer onclose={() => (drawerOpen = false)} />
  {/if}
</main>

<style>
  main {
    position: fixed;
    inset: 0;
    overflow: hidden;
    background: #000;
  }
  main.cursor-hidden {
    cursor: none;
  }
  .overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
    transition: opacity 0.4s ease;
  }
  .overlay.faded {
    opacity: 0;
  }
  .overlay.faded * {
    pointer-events: none !important;
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
