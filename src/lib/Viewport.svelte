<script lang="ts">
  import { onMount } from "svelte";
  import { VideoOff, TriangleAlert, Play, LoaderCircle } from "@lucide/svelte";
  import { api } from "./api";
  import { app } from "./store.svelte";
  import StaticNoise from "./StaticNoise.svelte";

  let canvas: HTMLCanvasElement;
  let hasFrame = $state(false);
  let flashing = $state(false);

  // Latest frame only: one decoding, at most one waiting; a newer frame replaces the waiting one.
  let decoding = false;
  let waiting: ArrayBuffer | null = null;
  let frames = 0;

  async function decode(buf: ArrayBuffer) {
    decoding = true;
    try {
      const bitmap = await createImageBitmap(new Blob([buf], { type: "image/jpeg" }));
      if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
        canvas.width = bitmap.width;
        canvas.height = bitmap.height;
      }
      canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
      bitmap.close();
      if (app.live) hasFrame = true;
      frames++;
    } catch {
      // A corrupt frame (e.g. cut off by an unplug); skip it.
    } finally {
      void api.previewAck();
      decoding = false;
    }
    const next = waiting;
    waiting = null;
    if (next) void decode(next);
  }

  function onFrame(buf: ArrayBuffer) {
    if (decoding) {
      if (waiting) void api.previewAck(); // the replaced frame is done with
      waiting = buf;
    } else {
      void decode(buf);
    }
  }

  // Only live states show the picture; connecting/paused show black, the rest show static.
  $effect(() => {
    if (!app.live) hasFrame = false;
  });

  $effect(() => {
    if (app.flash === 0) return;
    flashing = true;
    const t = setTimeout(() => (flashing = false), 140);
    return () => clearTimeout(t);
  });

  onMount(() => {
    void api.openPreview(onFrame);
    app.thumbnailer = () => {
      if (!hasFrame || !canvas.width) return null;
      const t = document.createElement("canvas");
      t.height = 96;
      t.width = Math.round((canvas.width / canvas.height) * 96);
      t.getContext("2d")?.drawImage(canvas, 0, 0, t.width, t.height);
      return t.toDataURL("image/jpeg", 0.8);
    };
    const fpsTimer = setInterval(() => {
      app.displayFps = frames;
      frames = 0;
    }, 1000);
    return () => clearInterval(fpsTimer);
  });

  // The gradient stands in for the picture, so it gets the picture's shape: the other app's frame
  // size while busy, else our mode, else 16:9.
  const aspect = $derived.by(() => {
    const f = app.state === "busy" && app.busyFrame ? app.busyFrame : app.mode;
    return f && f.height > 0 ? f.width / f.height : 16 / 9;
  });

  const offline = $derived(app.state === "noCamera" || app.state === "busy" || app.state === "error");
  const offlineText = $derived(
    app.state === "busy"
      ? "Camera in use by another app"
      : app.state === "error"
        ? (app.message ?? "Camera error")
        : "Camera disconnected",
  );
</script>

<div class="viewport">
  <canvas bind:this={canvas} class:hidden={!hasFrame}></canvas>

  {#if offline}
    <div class="offline" style:--aspect={aspect}>
      <!-- Backgrounds: no camera → static (full window); camera there but not ours (busy, error,
           paused) → a gradient frame shaped like the picture; our stream → the picture. -->
      {#if app.state === "noCamera"}<StaticNoise />{:else}<div class="frame"></div>{/if}
      <div class="offline-content">
        {#if app.state === "error"}
          <TriangleAlert size={96} strokeWidth={1.5} />
        {:else}
          <VideoOff size={96} strokeWidth={1.5} />
        {/if}
        <p>{offlineText}</p>
        {#if app.state === "busy" && app.controlsAvailable}
          <p class="sub">Settings still work and apply to its picture.</p>
        {/if}
        {#if app.conflict && app.conflictDismissed}
          <button class="review" onclick={() => (app.conflictDismissed = false)}>
            Your saved settings weren't applied. Review differences
          </button>
        {/if}
      </div>
    </div>
  {:else if app.state === "paused"}
    <div class="paused" style:--aspect={aspect}>
      <div class="frame"></div>
      <button class="resume" onclick={() => app.setPaused(false)} title="Resume preview">
        <Play size={40} />
      </button>
      <p>{app.manualPause ? "Preview paused" : "Paused while the window is hidden"}</p>
      {#if app.manualPause && app.controlsAvailable}
        <p class="sub">The camera is free for other apps. Settings still work.</p>
      {/if}
    </div>
  {:else if !hasFrame}
    <!-- Connecting, or streaming before the first frame is drawn: keep the gradient up so
         resuming never blanks to black. -->
    <div class="paused" style:--aspect={aspect}>
      <div class="frame"></div>
      <span class="spinner" role="status" aria-label="Starting camera"><LoaderCircle size={44} strokeWidth={1.75} /></span>
    </div>
  {/if}

  {#if flashing}
    <div class="flash"></div>
  {/if}
</div>

<style>
  .viewport {
    position: absolute;
    inset: 0;
    background: #000;
    overflow: hidden;
  }
  canvas {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  canvas.hidden {
    visibility: hidden;
  }
  .offline,
  .paused {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    container-type: size;
  }
  .spinner {
    display: grid;
    color: rgba(255, 255, 255, 0.85);
    filter: drop-shadow(0 2px 8px rgba(0, 0, 0, 0.5));
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2.5s;
    }
  }
  /* Content sits above the absolutely positioned frame. */
  .paused > :not(.frame) {
    position: relative;
  }
  /* Letterboxed like the canvas (object-fit: contain), black around it. */
  .frame {
    position: absolute;
    width: min(100cqw, calc(100cqh * var(--aspect)));
    aspect-ratio: var(--aspect);
    background:
      radial-gradient(ellipse 75% 65% at 45% 35%, #6b4a2f 0%, transparent 70%),
      linear-gradient(160deg, #5a3e28 0%, #4a321f 55%, #3d2817 100%);
  }
  .offline-content {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    color: #fff;
    filter: drop-shadow(0 2px 12px rgba(0, 0, 0, 0.8));
  }
  .offline-content p,
  .paused p {
    margin: 0;
    font-size: 17px;
    font-weight: 500;
    text-shadow: 0 1px 8px rgba(0, 0, 0, 0.9);
    max-width: 60ch;
    text-align: center;
  }
  .offline-content p.sub,
  .paused p.sub {
    font-size: 14px;
    font-weight: 400;
    opacity: 0.8;
    margin-top: -8px;
  }
  .review {
    font-size: 13px;
    color: #fff;
    text-decoration: underline;
    text-underline-offset: 3px;
    opacity: 0.9;
  }
  .paused {
    align-content: center;
    gap: 16px;
    color: rgba(255, 255, 255, 0.75);
  }
  .resume {
    width: 80px;
    height: 80px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.1);
    color: #fff;
    display: grid;
    place-items: center;
  }
  .resume:hover {
    background: rgba(255, 255, 255, 0.18);
  }
  .flash {
    position: absolute;
    inset: 0;
    background: #fff;
    opacity: 0.85;
    animation: flash 140ms ease-out forwards;
    pointer-events: none;
  }
  @keyframes flash {
    to {
      opacity: 0;
    }
  }
</style>
