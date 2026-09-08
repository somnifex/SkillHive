import axios from "axios";
import { create } from "zustand";

import { resolveApiBase } from "../api/server";
import { restoreDesktopSession } from "../api/desktopSession";
import type { TokenResponse, User } from "../types";

interface AuthState {
  user: User | null;
  accessToken: string | null;
  loading: boolean;
  setSession: (payload: TokenResponse) => void;
  clearSession: () => void;
  bootstrap: () => Promise<void>;
}

export const useAuthStore = create<AuthState>((set) => ({
  user: null,
  accessToken: null,
  loading: true,
  setSession: (payload) =>
    set({
      user: payload.user,
      accessToken: payload.access_token,
      loading: false,
    }),
  clearSession: () => set({ user: null, accessToken: null, loading: false }),
  bootstrap: async () => {
    try {
      const { data } = await axios.post<TokenResponse>(
        `${resolveApiBase()}/auth/refresh`,
        undefined,
        { withCredentials: true },
      );
      set({ user: data.user, accessToken: data.access_token, loading: false });
    } catch {
      // Tauri keeps the long-lived refresh credential behind Rust's OS
      // keyring.  If the WebView cookie is lost on reload, restore only a
      // short-lived access session through the privileged command boundary.
      try {
        const restored = await restoreDesktopSession();
        if (restored) {
          set({
            user: restored.user,
            accessToken: restored.access_token,
            loading: false,
          });
          return;
        }
      } catch {
        // Fall through to the normal signed-out state.
      }
      set({ user: null, accessToken: null, loading: false });
    }
  },
}));

interface AppearanceState {
  dark: boolean;
  toggle: () => void;
}

const initialDark = window.localStorage.getItem("skillhive-theme") === "dark";

export const useAppearanceStore = create<AppearanceState>((set) => ({
  dark: initialDark,
  toggle: () =>
    set((state) => {
      const dark = !state.dark;
      window.localStorage.setItem("skillhive-theme", dark ? "dark" : "light");
      return { dark };
    }),
}));
