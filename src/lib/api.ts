// Typed wrappers around the backend commands and events (SPEC §8.3).

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type CamState = "noCamera" | "connecting" | "streaming" | "recording" | "paused" | "busy" | "error";
export type Group = "framing" | "exposure" | "focus" | "color";
export type VideoQuality = "original" | "compact";
export type CaptureKind = "photo" | "video";

export interface MenuOption {
  value: number;
  label: string;
}

export interface Control {
  id: string;
  label: string;
  group: Group;
  kind: "int" | "bool" | "menu";
  min: number;
  max: number;
  step: number;
  default: number;
  value: number;
  options?: MenuOption[];
  unit?: string;
  inactive: boolean;
  readOnly: boolean;
}

export interface Mode {
  width: number;
  height: number;
  fpsN: number;
  fpsD: number;
}

export interface CameraInfo {
  sysfsPath: string;
  node: string;
  model: string;
  serial: string;
  firmware: string;
  usbSpeedMbps: number;
  alsaCard: number | null;
}

export interface StatePayload {
  state: CamState;
  message?: string;
  device: CameraInfo | null;
  audioAvailable: boolean;
  manualPause: boolean;
  /** Controls can be changed now, also while another app has the stream (busy) or when paused. */
  controlsAvailable: boolean;
  /** While busy: the other app's frame size. */
  busyFrame: { width: number; height: number } | null;
}

export interface ModesPayload {
  modes: Mode[];
  usbSpeed: number;
  mode: Mode | null;
}

export interface Prefs {
  photoDir: string;
  videoDir: string;
  photoDirIsDefault: boolean;
  videoDirIsDefault: boolean;
  videoQuality: VideoQuality;
  audio: boolean;
  captureKind: CaptureKind;
}

export interface RecordingPayload {
  elapsedMs: number;
  bytes: number;
}

export interface CaptureSaved {
  path: string;
  kind: CaptureKind;
  interrupted: boolean;
}

export interface Stats {
  fps: number;
  dropped: number;
  resolution: string;
  format: string;
  usbSpeed: number;
  bitrate: number | null;
}

/** A setting where the camera (in use by another app) differs from the app's settings. */
export interface SettingDiff {
  id: string;
  label: string;
  /** null: decided automatically (its auto mode is on). */
  camera: number | null;
  app: number | null;
}

export interface Snapshot extends StatePayload {
  modes: ModesPayload;
  controls: Control[];
  prefs: Prefs;
  compactAvailable: boolean;
  recording: RecordingPayload | null;
  /** Started with `--debug`: keep the webview's context menu (web inspector). */
  debug: boolean;
  conflict: SettingDiff[] | null;
}

/** Error the backend returns for writes to a camera that just vanished; the UI ignores it. */
export const DISCONNECTED = "disconnected";

export const api = {
  getSnapshot: () => invoke<Snapshot>("get_snapshot"),
  setMode: (mode: Mode) => invoke<void>("set_mode", { mode }),
  setControl: (id: string, value: number) => invoke<void>("set_control", { id, value }),
  resetControl: (id: string) => invoke<void>("reset_control", { id }),
  resetAll: () => invoke<void>("reset_all"),
  resolveSettingsConflict: (adopt: boolean) => invoke<void>("resolve_settings_conflict", { adopt }),
  captureStill: () => invoke<void>("capture_still"),
  startRecording: (audio: boolean, quality: VideoQuality) =>
    invoke<void>("start_recording", { options: { audio, quality } }),
  stopRecording: () => invoke<void>("stop_recording"),
  setPrefs: (prefs: Partial<Omit<Prefs, "photoDirIsDefault" | "videoDirIsDefault">>) =>
    invoke<Prefs>("set_prefs", { prefs }),
  reveal: (path: string) => invoke<void>("reveal", { path }),
  openCapture: (path: string) => invoke<void>("open_capture", { path }),
  setHidden: (hidden: boolean) => invoke<void>("set_hidden", { hidden }),
  setPaused: (paused: boolean) => invoke<void>("set_paused", { paused }),
  previewAck: () => invoke<void>("preview_ack"),
  quit: () => invoke<void>("quit"),
  openPreview: (onFrame: (frame: ArrayBuffer) => void) => {
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = onFrame;
    return invoke<void>("open_preview", { channel });
  },
};

export interface Events {
  "camera-state": StatePayload;
  "controls-changed": { controls: Control[] };
  "modes-changed": ModesPayload;
  recording: RecordingPayload;
  "capture-saved": CaptureSaved;
  stats: Stats;
  error: { message: string };
  "settings-conflict": { differences: SettingDiff[] | null };
}

export function on<K extends keyof Events>(event: K, handler: (payload: Events[K]) => void): Promise<UnlistenFn> {
  return listen<Events[K]>(event, (e) => handler(e.payload));
}
