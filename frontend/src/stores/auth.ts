import axios from "axios";
import { create } from "zustand";

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
        "/api/v1/auth/refresh",
        undefined,
        { withCredentials: true },
      );
      set({ user: data.user, accessToken: data.access_token, loading: false });
    } catch {
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
