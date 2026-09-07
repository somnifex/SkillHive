import axios, { AxiosError, type InternalAxiosRequestConfig } from "axios";

import { useAuthStore } from "../stores/auth";
import type { TokenResponse } from "../types";
import { isDesktop, resolveApiBase, setStoredServerUrl } from "./server";

export const api = axios.create({
  baseURL: resolveApiBase(),
  withCredentials: true,
  timeout: 15000,
});

interface TauriGlobal {
  core?: {
    invoke?: (command: string, args?: unknown) => Promise<unknown>;
  };
}

/**
 * Persists the backend address and re-points every API call. On the desktop
 * the address is mirrored into the Rust-side config so the sync engine and
 * the webview talk to the same server; the web build just stores it locally.
 */
export function setServerUrl(raw: string): void {
  const normalized = raw.trim().replace(/\/+$/, "");
  setStoredServerUrl(normalized);
  api.defaults.baseURL = resolveApiBase();
  if (isDesktop() && normalized) {
    const global = window as unknown as { __TAURI__?: TauriGlobal };
    const invoke = global.__TAURI__?.core?.invoke;
    if (invoke) {
      void invoke("set_server_url", { request: { baseUrl: normalized } }).catch(
        () => undefined,
      );
    }
  }
}

export function currentServerUrl(): string {
  return resolveApiBase().replace(/\/api\/v1$/, "");
}

api.interceptors.request.use((config: InternalAxiosRequestConfig) => {
  const token = useAuthStore.getState().accessToken;
  if (token) config.headers.Authorization = `Bearer ${token}`;
  return config;
});

let refreshing: Promise<string | null> | null = null;

api.interceptors.response.use(
  (response) => response,
  async (error: AxiosError) => {
    const original = error.config as (InternalAxiosRequestConfig & {
      _retry?: boolean;
    }) | undefined;
    if (
      error.response?.status !== 401 ||
      !original ||
      original._retry ||
      original.url?.includes("/auth/")
    ) {
      throw error;
    }
    original._retry = true;
    refreshing ??= axios
      .post<TokenResponse>(`${resolveApiBase()}/auth/refresh`, undefined, {
        withCredentials: true,
      })
      .then(({ data }) => {
        useAuthStore.getState().setSession(data);
        return data.access_token;
      })
      .catch(() => {
        useAuthStore.getState().clearSession();
        return null;
      })
      .finally(() => {
        refreshing = null;
      });
    const token = await refreshing;
    if (!token) throw error;
    original.headers.Authorization = `Bearer ${token}`;
    return api(original);
  },
);

export function errorMessage(error: unknown): string {
  if (axios.isAxiosError(error)) {
    const payload = error.response?.data as
      | { error?: { message?: string } }
      | undefined;
    return payload?.error?.message ?? "请求失败，请稍后再试。";
  }
  return "发生了意外错误。";
}
