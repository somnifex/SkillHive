const STORAGE_KEY = "skillhive-server-url";
const DESKTOP_DEFAULT = "http://127.0.0.1:8000";

interface TauriGlobal {
  core?: {
    invoke?: (command: string, args?: unknown) => Promise<unknown>;
  };
}

function tauri(): TauriGlobal | null {
  const global = window as unknown as { __TAURI__?: TauriGlobal };
  return global.__TAURI__ ?? null;
}

export function isDesktop(): boolean {
  return tauri() !== null;
}

export function normalizeServerUrl(raw: string): string {
  return raw.trim().replace(/\/+$/, "");
}

export function isValidServerUrl(raw: string): boolean {
  const value = normalizeServerUrl(raw);
  return /^https?:\/\/.+/i.test(value) && !value.includes("@") && !value.includes(" ");
}

/**
 * The origin the API calls should target. An empty string means
 * same-origin (the default web deployment, where nginx proxies /api/v1).
 */
export function resolveServerOrigin(): string {
  const stored = window.localStorage.getItem(STORAGE_KEY);
  if (stored) return normalizeServerUrl(stored);
  return isDesktop() ? DESKTOP_DEFAULT : "";
}

export function resolveApiBase(): string {
  const origin = resolveServerOrigin();
  return origin ? `${origin}/api/v1` : "/api/v1";
}

/** Persists (or clears, when empty) the user-configured server address. */
export function setStoredServerUrl(url: string): void {
  if (url) {
    window.localStorage.setItem(STORAGE_KEY, url);
  } else {
    window.localStorage.removeItem(STORAGE_KEY);
  }
}

export async function pingServer(url: string): Promise<void> {
  const origin = normalizeServerUrl(url);
  if (!isValidServerUrl(origin)) {
    throw new Error("服务器地址必须是 http(s) URL");
  }
  const response = await fetch(`${origin}/api/v1/health`, { method: "GET" });
  if (!response.ok) {
    throw new Error(`服务器响应异常（HTTP ${response.status}）`);
  }
}
