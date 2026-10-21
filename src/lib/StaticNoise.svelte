<script lang="ts">
  // TV static for the camera-off screen (SPEC §7.3): 320×180 random greyscale at ~24 fps, scaled
  // up with a slight blur, kept dim; scanlines and a rolling band are CSS overlays.
  import { onMount } from "svelte";

  const W = 320;
  const H = 180;
  let canvas: HTMLCanvasElement;
  let reduced = $state(false);

  onMount(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    reduced = mq.matches;
    const onChange = () => (reduced = mq.matches);
    mq.addEventListener("change", onChange);

    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) return () => mq.removeEventListener("change", onChange);
    const img = ctx.createImageData(W, H);
    const px = new Uint32Array(img.data.buffer);
    let raf = 0;
    let last = 0;
    let seed = 0x9e3779b9;

    const draw = (t: number) => {
      raf = requestAnimationFrame(draw);
      if (reduced || t - last < 1000 / 24) return;
      last = t;
      for (let i = 0; i < px.length; i++) {
        // xorshift32: much cheaper than Math.random for 57,600 pixels per frame.
        seed ^= seed << 13;
        seed ^= seed >>> 17;
        seed ^= seed << 5;
        const v = (seed >>> 24) * 0.35;
        px[i] = 0xff000000 | (v << 16) | (v << 8) | v;
      }
      ctx.putImageData(img, 0, 0);
    };
    raf = requestAnimationFrame(draw);
    return () => {
      cancelAnimationFrame(raf);
      mq.removeEventListener("change", onChange);
    };
  });
</script>

<div class="static" class:reduced>
  <canvas bind:this={canvas} width={W} height={H}></canvas>
  <div class="scanlines"></div>
  <div class="band"></div>
</div>

<style>
  .static {
    position: absolute;
    inset: 0;
    background: #000;
    overflow: hidden;
  }
  canvas {
    width: 100%;
    height: 100%;
    filter: blur(0.6px);
    image-rendering: auto;
  }
  .scanlines {
    position: absolute;
    inset: 0;
    background: repeating-linear-gradient(to bottom, rgba(0, 0, 0, 0.28) 0 1px, transparent 1px 3px);
  }
  .band {
    position: absolute;
    left: 0;
    right: 0;
    height: 22%;
    top: -22%;
    background: linear-gradient(to bottom, transparent, rgba(255, 255, 255, 0.05) 50%, transparent);
    animation: roll 6s linear infinite;
  }
  @keyframes roll {
    to {
      transform: translateY(560%);
    }
  }
  .reduced canvas,
  .reduced .scanlines,
  .reduced .band {
    display: none;
  }
</style>
