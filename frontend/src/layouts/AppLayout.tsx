import {
  Blocks,
  BookOpen,
  ChevronDown,
  FileText,
  Home,
  LogOut,
  Menu as MenuIcon,
  Moon,
  Rocket,
  Settings,
  ShieldCheck,
  Sun,
  Users,
} from "lucide-react";
import { App, Avatar, Button, Drawer, Dropdown, Layout } from "antd";
import { useState } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";

import { api, errorMessage } from "../api/client";
import { isDesktop } from "../api/server";
import { desktopLogout } from "../api/desktop";
import { BrandLogo } from "../components/BrandLogo";
import { SyncStatus } from "../components/SyncStatus";
import { useAppearanceStore, useAuthStore } from "../stores/auth";

const { Header, Sider, Content } = Layout;

const workNavigation = [
  { key: "/", icon: Home, label: "工作台" },
  { key: "/skills", icon: BookOpen, label: "我的 Skills" },
  { key: "/agents", icon: Rocket, label: "Agent 部署" },
  { key: "/templates", icon: FileText, label: "模板库" },
];

const teamNavigation = [
  { key: "/groups", icon: Users, label: "协作群组" },
  { key: "/group-skills", icon: Blocks, label: "群组 Skills" },
];

const routeTitles: Record<string, string> = {
  "/": "工作台",
  "/skills": "我的 Skills",
  "/agents": "Agent 部署",
  "/templates": "模板库",
  "/groups": "协作群组",
  "/group-skills": "群组 Skills",
  "/admin": "管理后台",
  "/settings": "个人设置",
};

export function AppLayout() {
  const navigate = useNavigate();
  const location = useLocation();
  const { message } = App.useApp();
  const user = useAuthStore((state) => state.user);
  const clearSession = useAuthStore((state) => state.clearSession);
  const dark = useAppearanceStore((state) => state.dark);
  const toggleTheme = useAppearanceStore((state) => state.toggle);
  const [mobileOpen, setMobileOpen] = useState(false);

  const selected =
    ["/skills", "/agents", "/templates", "/groups", "/group-skills", "/admin", "/settings"].find(
      (path) => location.pathname.startsWith(path),
    ) ?? "/";

  const go = (path: string) => {
    navigate(path);
    setMobileOpen(false);
  };

  const logout = async () => {
    let warning: string | null = null;
    try {
      await api.post("/auth/logout");
    } catch (error) {
      warning = errorMessage(error);
    }
    if (isDesktop()) {
      try {
        await desktopLogout();
      } catch (error) {
        warning = error instanceof Error ? error.message : "本机会话撤销失败";
      }
    }
    clearSession();
    navigate("/login");
    if (warning) {
      message.warning(`已退出当前页面，但服务端会话撤销需要重试：${warning}`);
    }
  };

  const nav = (
    <nav aria-label="主导航">
      <div className="nav-group-label">工作台</div>
      <div className="nav-rail">
        {workNavigation.map(({ key, icon: Icon, label }) => (
          <button
            key={key}
            type="button"
            className={`nav-item${selected === key ? " is-active" : ""}`}
            aria-current={selected === key ? "page" : undefined}
            onClick={() => go(key)}
          >
            <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
            <span>{label}</span>
          </button>
        ))}
      </div>
      <div className="nav-group-label">协作</div>
      <div className="nav-rail">
        {teamNavigation.map(({ key, icon: Icon, label }) => (
          <button
            key={key}
            type="button"
            className={`nav-item${selected === key ? " is-active" : ""}`}
            aria-current={selected === key ? "page" : undefined}
            onClick={() => go(key)}
          >
            <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
            <span>{label}</span>
          </button>
        ))}
        {user?.is_global_admin && (
          <button
            type="button"
            className={`nav-item${selected === "/admin" ? " is-active" : ""}`}
            aria-current={selected === "/admin" ? "page" : undefined}
            onClick={() => go("/admin")}
          >
            <ShieldCheck size={18} strokeWidth={1.8} aria-hidden="true" />
            <span>管理后台</span>
          </button>
        )}
      </div>
    </nav>
  );

  return (
    <Layout className="app-shell">
      <a className="skip-link" href="#main-content">
        跳到主要内容
      </a>

      <Sider width={232} className="sidebar">
        <button type="button" className="brand brand-button" onClick={() => go("/")}>
          <BrandLogo />
          <span className="brand-copy">
            <strong>SkillHive</strong>
            <small>团队 Skills 平台</small>
          </span>
        </button>

        <div className="nav-group">{nav}</div>

        <div className="rail-footer">
          <div className="rail-user">
            <Avatar size={34}>
              {user?.display_name?.slice(0, 1).toUpperCase()}
            </Avatar>
            <div className="rail-user-meta">
              <strong>{user?.display_name}</strong>
              <small>
                {user?.is_global_admin ? "全局管理员" : "普通用户"}
              </small>
            </div>
          </div>
          <button
            type="button"
            className="nav-item"
            onClick={() => go("/settings")}
            aria-current={selected === "/settings" ? "page" : undefined}
          >
            <Settings size={18} strokeWidth={1.8} aria-hidden="true" />
            <span>个人设置</span>
          </button>
          <button type="button" className="nav-item" onClick={logout}>
            <LogOut size={18} strokeWidth={1.8} aria-hidden="true" />
            <span>退出登录</span>
          </button>
        </div>
      </Sider>

      <Layout className="app-stage">
        <Header className="topbar">
          <div className="topbar-leading">
            <Button
              type="text"
              className="mobile-menu-button"
              aria-label="打开导航"
              icon={<MenuIcon size={20} aria-hidden="true" />}
              onClick={() => setMobileOpen(true)}
            />
            <div className="route-context">
              <span>SKILLHIVE</span>
              <strong>{routeTitles[selected] ?? "工作台"}</strong>
            </div>
          </div>
          <div className="top-actions">
            <SyncStatus />
            <Button
              type="text"
              className="icon-button"
              aria-label="切换深色模式"
              icon={
                dark ? (
                  <Sun size={18} strokeWidth={1.8} aria-hidden="true" />
                ) : (
                  <Moon size={18} strokeWidth={1.8} aria-hidden="true" />
                )
              }
              onClick={toggleTheme}
            />
            <Dropdown
              trigger={["click"]}
              menu={{
                items: [
                  {
                    key: "settings",
                    icon: <Settings size={16} aria-hidden="true" />,
                    label: "个人设置",
                    onClick: () => navigate("/settings"),
                  },
                  {
                    key: "logout",
                    icon: <LogOut size={16} aria-hidden="true" />,
                    label: "退出登录",
                    onClick: logout,
                  },
                ],
              }}
            >
              <Button type="text" className="account-button">
                <Avatar size={30} className="account-avatar">
                  {user?.display_name?.slice(0, 1).toUpperCase()}
                </Avatar>
                <span className="account-name">{user?.display_name}</span>
                <ChevronDown size={14} aria-hidden="true" />
              </Button>
            </Dropdown>
          </div>
        </Header>

        <Content id="main-content" className="content" tabIndex={-1}>
          <div key={location.pathname} className="page-frame">
            <Outlet />
          </div>
        </Content>
      </Layout>

      <Drawer
        open={mobileOpen}
        onClose={() => setMobileOpen(false)}
        placement="left"
        width="min(320px, 86vw)"
        className="mobile-drawer"
        title={
          <div className="drawer-brand">
            <BrandLogo />
            <span>SkillHive</span>
          </div>
        }
      >
        {nav}
      </Drawer>
    </Layout>
  );
}
