<script lang="ts">
  import { Camera, Video, Mic, MicOff, Square, Circle } from "@lucide/svelte";
  import { api } from "./api";
  import { app } from "./store.svelte";

  const kind = $derived(app.captureKind);
  const micOn = $derived(!!app.prefs?.audio && app.audioAvailable);
  const micTitle = $derived(
    !app.audioAvailable ? "Camera microphone not found" : micOn ? "Record with the camera's microphone" : "Record without audio",
  );
  const shutterTitle = $derived(
    kind === "photo" ? "Take photo (Space)" : app.isRecording ? "Stop recording (Space)" : "Start recording (Space)",
  );
</script>

<div class="bar">
  <div class="slot left">
    {#if app.lastCapture}
      <button
        class="thumb"
        title="Open {app.lastCapture.kind === 'photo' ? 'photo' : 'video'}"
        onclick={() => app.lastCapture && void api.openCapture(app.lastCapture.path).catch((e) => app.toast(String(e), "error"))}
      >
        {#if app.lastCapture.thumb}
          <img src={app.lastCapture.thumb} alt="Last capture" />
        {:else if app.lastCapture.kind === "video"}
          <Video size={20} />
        {:else}
          <Camera size={20} />
        {/if}
      </button>
    {/if}
  </div>

  <div class="segmented surface" role="radiogroup" aria-label="Capture mode">
    <button
      role="radio"
      aria-checked={kind === "photo"}
      class:active={kind === "photo"}
      disabled={app.isRecording}
      onclick={() => app.setCaptureKind("photo")}
      title="Photo (P)"
    >
      <Camera size={16} /> Photo
    </button>
    <button
      role="radio"
      aria-checked={kind === "video"}
      class:active={kind === "video"}
      disabled={app.isRecording}
      onclick={() => app.setCaptureKind("video")}
      title="Video (V)"
    >
      <Video size={16} /> Video
    </button>
  </div>

  <button
    class="shutter"
    class:video={kind === "video"}
    class:recording={app.isRecording}
    disabled={!app.live}
    title={shutterTitle}
    aria-label={shutterTitle}
    onclick={() => app.shutter()}
  >
    {#if kind === "photo"}
      <Camera size={26} />
    {:else if app.isRecording}
      <Square size={22} fill="currentColor" />
    {:else}
      <Circle size={30} fill="currentColor" />
    {/if}
  </button>

  <div class="slot right">
    {#if kind === "video"}
      <button
        class="icon-btn surface mic"
        class:off={!micOn}
        disabled={!app.audioAvailable || app.isRecording}
        title={micTitle + " (M)"}
        aria-label={micTitle}
        onclick={() => app.toggleAudio()}
      >
        {#if micOn}<Mic size={18} />{:else}<MicOff size={18} />{/if}
      </button>
    {/if}
  </div>
</div>

<style>
  .bar {
    display: grid;
    grid-template-columns: 56px auto 72px 56px;
    align-items: center;
    gap: 20px;
  }
  .slot {
    display: flex;
    justify-content: center;
  }
  .thumb {
    width: 52px;
    height: 52px;
    border-radius: 10px;
    overflow: hidden;
    background: var(--surface);
    border: 1px solid rgba(255, 255, 255, 0.25);
    display: grid;
    place-items: center;
    color: #fff;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .shutter {
    width: 72px;
    height: 72px;
    border-radius: 50%;
    border: 4px solid rgba(255, 255, 255, 0.9);
    background: rgba(255, 255, 255, 0.16);
    color: #fff;
    display: grid;
    place-items: center;
    transition:
      transform 0.08s,
      background 0.15s;
    backdrop-filter: blur(16px);
  }
  .shutter:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.26);
  }
  .shutter:active:not(:disabled) {
    transform: scale(0.94);
  }
  .shutter.video {
    color: var(--red);
  }
  .shutter.recording {
    color: #fff;
    background: var(--red);
    border-color: rgba(255, 255, 255, 0.9);
  }
  .shutter:disabled {
    opacity: 0.4;
  }
  .mic {
    width: 44px;
    height: 44px;
    border-radius: 50%;
  }
  .mic.off {
    color: var(--muted);
  }
</style>
