import axios, { AxiosError, type InternalAxiosRequestConfig } from "axios";

import { useAuthStore } from "../stores/auth";
import type { TokenResponse } from "../types";

export const api = axios.create({
  baseURL: "/api/v1",
  withCredentials: true,
  timeout: 15000,
});

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
      .post<TokenResponse>("/api/v1/auth/refresh", undefined, {
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
