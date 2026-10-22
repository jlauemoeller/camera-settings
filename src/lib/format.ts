import type { Control, Mode } from "./api";

const NAMED: Record<string, string> = {
  "3840x2160": "4K",
  "2560x1440": "1440p",
  "1920x1080": "1080p",
  "1280x720": "720p",
  "640x480": "480p",
};

export function fps(mode: Mode): number {
  return mode.fpsN / Math.max(1, mode.fpsD);
}

export function fpsLabel(mode: Mode): string {
  const f = fps(mode);
  return Number.isInteger(f) ? String(f) : f.toFixed(1);
}

/** "4K", "1080p", or "4096 × 2160" for sizes without a common name. */
export function sizeLabel(m: { width: number; height: number }): string {
  return NAMED[`${m.width}x${m.height}`] ?? `${m.width} × ${m.height}`;
}

/** "4K · 30" */
export function modeLabel(mode: Mode): string {
  return `${sizeLabel(mode)} · ${fpsLabel(mode)}`;
}

export function sameMode(a: Mode | null | undefined, b: Mode | null | undefined): boolean {
  return !!a && !!b && a.width === b.width && a.height === b.height && a.fpsN * b.fpsD === b.fpsN * a.fpsD;
}

/** Display value of a control with its unit (SPEC §4.1 "UI" column). */
export function formatValue(c: Control, value = c.value): string {
  switch (c.unit) {
    case "100us": {
      const ms = value / 10;
      return `${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`;
    }
    case "zoom":
      return `${(value / 100).toFixed(1)}×`;
    case "arcsec":
      return `${(value / 3600).toFixed(0)}°`;
    case "kelvin":
      return `${value} K`;
  }
  const opt = c.options?.find((o) => o.value === value);
  return opt ? opt.label : String(value);
}

/** Human value of a setting for comparisons: switches as On/Off, auto modes as Auto/Manual,
 * `null` (value picked automatically) as "Auto". */
export function settingText(c: Control | undefined, value: number | null): string {
  if (value === null) return "Auto";
  if (!c) return String(value);
  if (c.id === "exposure_auto") return value === 1 ? "Manual" : "Auto";
  if (c.id.endsWith("_auto")) return value ? "Auto" : "Manual";
  if (c.kind === "bool" || (c.kind === "int" && c.min === 0 && c.max === 1)) return value ? "On" : "Off";
  if (c.id === "power_line_frequency" && value === 0) return "Off";
  return formatValue(c, value);
}

export function duration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const mm = String(Math.floor((s % 3600) / 60)).padStart(2, "0");
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

export function bytes(n: number): string {
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

export function usbLabel(mbps: number): string {
  if (!mbps) return "unknown";
  if (mbps <= 480) return `${mbps} Mbit/s (USB 2)`;
  return `${mbps / 1000} Gbit/s (USB 3)`;
}

export function basename(path: string): string {
  return path.split("/").pop() ?? path;
}
