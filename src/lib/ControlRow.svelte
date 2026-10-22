<script lang="ts">
  import type { Component } from "svelte";
  import { RotateCcw } from "@lucide/svelte";
  import type { Control } from "./api";
  import { app } from "./store.svelte";
  import { formatValue } from "./format";

  interface Props {
    control: Control;
    /** Extra UI rule on top of the driver's INACTIVE flag (e.g. pan/tilt at zoom 1×). */
    blocked?: boolean;
    blockedReason?: string;
    icon?: Component<{ size?: number }>;
  }
  let { control: c, blocked = false, blockedReason, icon: Icon }: Props = $props();

  const disabled = $derived(!app.controlsAvailable || c.inactive || c.readOnly || blocked);
  const offDefault = $derived(c.value !== c.default);
  const title = $derived(blocked ? blockedReason : c.inactive ? "Controlled automatically" : undefined);

  // Auto exposure is a menu (1 = manual, 3 = aperture priority) shown as an "Auto" switch.
  const isAutoExposure = $derived(c.id === "exposure_auto");
  // Some drivers report on/off controls as 0–1 integers (BRIO backlight compensation).
  const isSwitch = $derived(c.kind === "bool" || isAutoExposure || (c.kind === "int" && c.min === 0 && c.max === 1));
  const autoValue = $derived(c.options?.find((o) => o.value !== 1)?.value ?? 3);

  const optionLabel = (label: string) => (label === "Disabled" ? "Off" : label);

  function reset() {
    if (!disabled && offDefault) app.resetControl(c.id);
  }
</script>

{#snippet resetButton()}
  <button class="icon-btn small reset" class:invisible={!offDefault} disabled={disabled || !offDefault} title="Reset to default" onclick={reset}>
    <RotateCcw size={13} />
  </button>
{/snippet}

<div class="row" class:disabled {title}>
  {#if isSwitch}
    <label class="switch-row">
      <span class="name">{#if Icon}<Icon size={15} />{/if}{c.label}</span>
      <input
        type="checkbox"
        class="switch"
        {disabled}
        checked={isAutoExposure ? c.value !== 1 : c.value !== 0}
        onchange={(e) => {
          const on = e.currentTarget.checked;
          app.setControl(c.id, isAutoExposure ? (on ? autoValue : 1) : on ? 1 : 0);
        }}
      />
    </label>
  {:else if c.kind === "menu"}
    <div class="head">
      <span class="name">{#if Icon}<Icon size={15} />{/if}{c.label}</span>
      {@render resetButton()}
    </div>
    <div class="segmented full" role="radiogroup" aria-label={c.label}>
      {#each c.options ?? [] as o (o.value)}
        <button
          role="radio"
          aria-checked={c.value === o.value}
          class:active={c.value === o.value}
          {disabled}
          onclick={() => app.setControl(c.id, o.value)}
        >
          {optionLabel(o.label)}
        </button>
      {/each}
    </div>
  {:else}
    <div class="head">
      <span class="name">{#if Icon}<Icon size={15} />{/if}{c.label}</span>
      <span class="value">{formatValue(c)}</span>
      {@render resetButton()}
    </div>
    <input
      type="range"
      min={c.min}
      max={c.max}
      step={c.step}
      value={c.value}
      {disabled}
      aria-label={c.label}
      oninput={(e) => app.setControl(c.id, Number(e.currentTarget.value))}
      ondblclick={reset}
    />
  {/if}
</div>

<style>
  .row {
    padding: 8px 0;
  }
  .row.disabled .name,
  .row.disabled .value {
    opacity: 0.45;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }
  .name {
    flex: 1;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }
  .value {
    font-size: 12px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .switch-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    cursor: pointer;
  }
  .reset.invisible {
    visibility: hidden;
  }
  input[type="range"] {
    width: 100%;
  }
</style>
