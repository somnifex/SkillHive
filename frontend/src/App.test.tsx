import { App as AntApp } from "antd";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { api } from "./api/client";
import { PageHeader } from "./components/PageHeader";
import { LoginPage } from "./pages/AuthPages";
import { ProtectedRoute } from "./routes/AppRouter";
import { useAuthStore } from "./stores/auth";
import type { TokenResponse, User } from "./types";

const user: User = {
  id: "user-1",
  username: "alice",
  display_name: "Alice",
  email: "alice@example.com",
  avatar_url: null,
  status: "active",
  is_global_admin: false,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  last_login_at: null,
};

afterEach(() => {
  vi.restoreAllMocks();
  act(() => {
    useAuthStore.setState({ user: null, accessToken: null, loading: false });
  });
});

describe("SkillHive UI", () => {
  it("renders a reusable page header", () => {
    render(<PageHeader title="我的 Skills" description="管理私人能力" />);
    expect(screen.getByRole("heading", { name: "我的 Skills" })).toBeInTheDocument();
    expect(screen.getByText("管理私人能力")).toBeInTheDocument();
  });

  it("completes the login flow without persisting the access token", async () => {
    const payload: TokenResponse = {
      access_token: "short-lived-token",
      token_type: "bearer",
      expires_in: 900,
      user,
    };
    vi.spyOn(api, "post").mockResolvedValue({ data: payload } as never);
    render(
      <AntApp>
        <MemoryRouter initialEntries={["/login"]}>
          <Routes>
            <Route path="/login" element={<LoginPage />} />
            <Route path="/" element={<div>Dashboard ready</div>} />
          </Routes>
        </MemoryRouter>
      </AntApp>,
    );
    expect(screen.getByRole("img", { name: "SkillHive" })).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("用户名或邮箱"), {
      target: { value: "alice" },
    });
    fireEvent.change(screen.getByLabelText("密码"), {
      target: { value: "Strong123!" },
    });
    fireEvent.click(screen.getByRole("button", { name: /登\s*录/ }));
    await waitFor(() => expect(screen.getByText("Dashboard ready")).toBeInTheDocument());
    expect(useAuthStore.getState().accessToken).toBe("short-lived-token");
    expect(window.localStorage.getItem("access_token")).toBeNull();
  });

  it("blocks the admin route for a normal user", () => {
    act(() => {
      useAuthStore.setState({ user, accessToken: "token", loading: false });
    });
    render(
      <MemoryRouter>
        <ProtectedRoute admin>
          <div>Secret admin screen</div>
        </ProtectedRoute>
      </MemoryRouter>,
    );
    expect(screen.getByText("无权访问")).toBeInTheDocument();
    expect(screen.queryByText("Secret admin screen")).not.toBeInTheDocument();
  });
});
