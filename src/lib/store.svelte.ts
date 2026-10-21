// App state, fed by `get_snapshot` and backend events.

import {
  api,
  DISCONNECTED,
  on,
  type CamState,
  type CameraInfo,
  type CaptureKind,
  type Control,
  type Mode,
  type Prefs,
  type RecordingPayload,
  type Snapshot,
  type Stats,
} from "./api";
import { basename } from "./format";

export interface Toast {
  id: number;
  kind: "info" | "error";
  text: string;
  path?: string;
}

export interface LastCapture {
  path: string;
  kind: CaptureKind;
  thumb: string | null;
}

function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

class AppStore {
  state = $state<CamState>("noCamera");
  message = $state<string | undefined>(undefined);
  device = $state<CameraInfo | null>(null);
  audioAvailable = $state(false);
  manualPause = $state(false);
  modes = $state<Mode[]>([]);
  usbSpeed = $state(0);
  mode = $state<Mode | null>(null);
  controls = $state<Control[]>([]);
  prefs = $state<Prefs | null>(null);
  compactAvailable = $state(false);
  recording = $state<RecordingPayload | null>(null);
  stats = $state<Stats | null>(null);
  toasts = $state<Toast[]>([]);
  lastCapture = $state<LastCapture | null>(null);
  /** Increments on each still; the viewport flashes on change. */
  flash = $state(0);
  /** Frames the webview actually drew in the last second. */
  displayFps = $state(0);
  ready = $state(false);
  debug = $state(false);

  /** Grabs a small thumbnail of the current preview frame (set by the viewport). */
  thumbnailer: (() => string | null) | null = null;
  private pendingThumb: string | null = null;
  private toastId = 0;
  // Control writes: one in flight per control, newer values coalesced (sliders fire a lot).
  private inflight = new Set<string>();
  private queued = new Map<string, number>();

  get live() {
    return this.state === "streaming" || this.state === "recording";
  }

  get isRecording() {
    return this.state === "recording";
  }

  get captureKind(): CaptureKind {
    return this.prefs?.captureKind ?? "photo";
  }

  async init() {
    await Promise.all([
      on("camera-state", (p) => {
        this.state = p.state;
        this.message = p.message;
        this.device = p.device;
        this.audioAvailable = p.audioAvailable;
        this.manualPause = p.manualPause;
        if (p.state !== "recording") this.recording = null;
      }),
      on("controls-changed", (p) => this.mergeControls(p.controls)),
      on("modes-changed", (p) => {
        this.modes = p.modes;
        this.usbSpeed = p.usbSpeed;
        this.mode = p.mode;
      }),
      on("recording", (p) => (this.recording = p)),
      on("stats", (p) => (this.stats = p)),
      on("capture-saved", (p) => {
        const thumb = p.kind === "video" ? this.pendingThumb : (this.pendingThumb ?? this.thumbnailer?.() ?? null);
        this.pendingThumb = null;
        this.lastCapture = { path: p.path, kind: p.kind, thumb };
        const text = p.interrupted
          ? "Recording saved (camera disconnected)"
          : `${p.kind === "photo" ? "Photo" : "Video"} saved: ${basename(p.path)}`;
        this.toast(text, "info", p.path);
      }),
      on("error", (p) => this.toast(p.message, "error")),
    ]);
    this.apply(await api.getSnapshot());
    this.ready = true;
  }

  private apply(s: Snapshot) {
    this.state = s.state;
    this.message = s.message;
    this.device = s.device;
    this.audioAvailable = s.audioAvailable;
    this.manualPause = s.manualPause;
    this.modes = s.modes.modes;
    this.usbSpeed = s.modes.usbSpeed;
    this.mode = s.modes.mode;
    this.controls = s.controls;
    this.prefs = s.prefs;
    this.compactAvailable = s.compactAvailable;
    this.recording = s.recording;
    this.debug = s.debug;
  }

  /** Device values win, except for a control whose write is still pending. */
  private mergeControls(controls: Control[]) {
    this.controls = controls.map((c) => {
      const q = this.queued.get(c.id);
      return q !== undefined ? { ...c, value: q } : c;
    });
  }

  toast(text: string, kind: Toast["kind"] = "info", path?: string) {
    const id = ++this.toastId;
    this.toasts = [...this.toasts.slice(-2), { id, kind, text, path }];
    setTimeout(() => this.dismiss(id), kind === "error" ? 7000 : 4500);
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  private async guard(p: Promise<unknown>) {
    try {
      await p;
    } catch (e) {
      const msg = errorText(e);
      if (msg !== DISCONNECTED) this.toast(msg, "error");
    }
  }

  setControl(id: string, value: number) {
    this.controls = this.controls.map((c) => (c.id === id ? { ...c, value } : c));
    if (this.inflight.has(id)) {
      this.queued.set(id, value);
      return;
    }
    this.inflight.add(id);
    void (async () => {
      let v: number | undefined = value;
      while (v !== undefined) {
        await this.guard(api.setControl(id, v));
        v = this.queued.get(id);
        this.queued.delete(id);
      }
      this.inflight.delete(id);
    })();
  }

  resetControl(id: string) {
    this.queued.delete(id);
    void this.guard(api.resetControl(id));
  }

  resetAll() {
    this.queued.clear();
    void this.guard(api.resetAll());
  }

  setMode(mode: Mode) {
    this.mode = mode;
    void this.guard(api.setMode(mode));
  }

  async setPrefs(p: Parameters<typeof api.setPrefs>[0]) {
    try {
      this.prefs = await api.setPrefs(p);
    } catch (e) {
      this.toast(errorText(e), "error");
    }
  }

  setCaptureKind(kind: CaptureKind) {
    if (this.isRecording || this.captureKind === kind) return;
    void this.setPrefs({ captureKind: kind });
  }

  toggleAudio() {
    if (!this.prefs || this.isRecording || !this.audioAvailable) return;
    void this.setPrefs({ audio: !this.prefs.audio });
  }

  shutter() {
    if (!this.live) return;
    if (this.captureKind === "photo") {
      this.pendingThumb = this.thumbnailer?.() ?? null;
      this.flash++;
      void this.guard(api.captureStill());
    } else if (this.isRecording) {
      void this.guard(api.stopRecording());
    } else {
      this.pendingThumb = this.thumbnailer?.() ?? null;
      const audio = !!this.prefs?.audio && this.audioAvailable;
      void this.guard(api.startRecording(audio, this.prefs?.videoQuality ?? "original"));
    }
  }

  setPaused(paused: boolean) {
    void this.guard(api.setPaused(paused));
  }
}

export const app = new AppStore();
