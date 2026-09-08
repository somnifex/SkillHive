import { isDesktop } from "./server";
import type { TokenResponse } from "../types";

interface TauriGlobal {
  core?: {
    invoke?: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  };
}

/** Restores only the short-lived WebView session through Rust. */
export async function restoreDesktopSession(): Promise<TokenResponse | null> {
  if (!isDesktop()) return null;
  const global = window as unknown as { __TAURI__?: TauriGlobal };
  const invoke = global.__TAURI__?.core?.invoke;
  if (!invoke) return null;
  return (await invoke("desktop_restore_session", {})) as TokenResponse;
}
