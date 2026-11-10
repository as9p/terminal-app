// The window's settings, kept as settings.json in the app's data folder by the Tauri side.

import { invoke } from "@tauri-apps/api/core";

export type Settings = {
  fontSize: number;
  focusFollowsMouse: boolean;
  sidebarWidth: number;
  sidebarShown: boolean;
};

export const DEFAULT_SETTINGS: Settings = {
  fontSize: 14,
  focusFollowsMouse: true,
  sidebarWidth: 240,
  sidebarShown: true,
};

const DEV_KEY = "settings";

function inTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/** A value of the wrong type, from an old or hand-edited file, falls back to its default. */
function settle(raw: unknown): Settings {
  const saved = typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : {};
  const pick = <K extends keyof Settings>(key: K): Settings[K] =>
    typeof saved[key] === typeof DEFAULT_SETTINGS[key] ? (saved[key] as Settings[K]) : DEFAULT_SETTINGS[key];
  return {
    fontSize: pick("fontSize"),
    focusFollowsMouse: pick("focusFollowsMouse"),
    sidebarWidth: pick("sidebarWidth"),
    sidebarShown: pick("sidebarShown"),
  };
}

/** Never rejects: a missing or unreadable file gives the defaults. */
export async function loadSettings(): Promise<Settings> {
  try {
    if (!inTauri()) return settle(JSON.parse(localStorage.getItem(DEV_KEY) ?? "{}"));
    return settle(await invoke<unknown>("settings_get"));
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export async function saveSettings(patch: Partial<Settings>): Promise<void> {
  if (!inTauri()) {
    const saved = await loadSettings();
    localStorage.setItem(DEV_KEY, JSON.stringify({ ...saved, ...patch }));
    return;
  }
  await invoke("settings_set", { patch });
}
