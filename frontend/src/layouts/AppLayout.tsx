import {
  Blocks,
  BookOpen,
  ChevronDown,
  CircleGauge,
  FileText,
  Home,
  LogOut,
  Menu as MenuIcon,
  Moon,
  Settings,
  ShieldCheck,
  Sun,
  Users,
} from "lucide-react";
import { Avatar, Button, Drawer, Dropdown, Layout } from "antd";
import { useEffect, useState, type PointerEvent } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";

import { api } from "../api/client";
import { BrandLogo } from "../components/BrandLogo";
import { useAppearanceStore, useAuthStore } from "../stores/auth";

const { Header, Sider, Content } = Layout;

const coreNavigation = [
  { key: "/", icon: Home, label: "总览", code: "01" },
  { key: "/skills", icon: BookOpen, label: "我的 Skills", code: "02" },
  { key: "/templates", icon: FileText, label: "模板档案", code: "03" },
  { key: "/groups", icon: Users, label: "协作群组", code: "04" },
  { key: "/group-skills", icon: Blocks, label: "群组 Skills", code: "05" },
];

const routeTitles: Record<string, string> = {
  "/": "能力总览",
  "/skills": "私人能力库",
  "/templates": "模板档案",
  "/groups": "协作群组",
  "/group-skills": "群组能力",
  "/admin": "系统控制",
  "/settings": "个人设置",
};

export function AppLayout() {
  const navigate = useNavigate();
  const location = useLocation();
  const user = useAuthStore((state) => state.user);
  const clearSession = useAuthStore((state) => state.clearSession);
  const dark = useAppearanceStore((state) => state.dark);
  const toggleTheme = useAppearanceStore((state) => state.toggle);
  const [mobileOpen, setMobileOpen] = useState(false);

  const selected =
    ["/skills", "/templates", "/groups", "/group-skills", "/admin", "/settings"].find(
      (path) => location.pathname.startsWith(path),
    ) ?? "/";

  const navigation = [
    ...coreNavigation,
    ...(user?.is_global_admin
      ? [{ key: "/admin", icon: ShieldCheck, label: "系统控制", code: "06" }]
      : []),
  ];

  const go = (path: string) => {
    navigate(path);
    setMobileOpen(false);
  };

  const logout = async () => {
    try {
      await api.post("/auth/logout");
    } finally {
      clearSession();
      navigate("/login");
    }
  };

  const trackPointer = (event: PointerEvent<HTMLElement>) => {
    event.currentTarget.style.setProperty("--pointer-x", `${event.clientX}px`);
    event.currentTarget.style.setProperty("--pointer-y", `${event.clientY}px`);
  };

  useEffect(() => {
    document.getElementById("main-content")?.focus({ preventScroll: true });
  }, [location.pathname]);

  const nav = (
    <nav className="nav-rail" aria-label="主要导航">
      {navigation.map(({ key, icon: Icon, label, code }) => (
        <button
          key={key}
          type="button"
          className={`nav-item${selected === key ? " is-active" : ""}`}
          aria-current={selected === key ? "page" : undefined}
          onClick={() => go(key)}
        >
          <span className="nav-index">{code}</span>
          <Icon size={19} strokeWidth={1.7} aria-hidden="true" />
          <span>{label}</span>
        </button>
      ))}
    </nav>
  );

  return (
    <Layout className="app-shell" onPointerMove={trackPointer}>
      <a className="skip-link" href="#main-content">
        跳到主要内容
      </a>
      <div className="ambient-field" aria-hidden="true">
        <div className="ambient-grid" />
        <div className="ambient-glow" />
      </div>

      <Sider width={254} className="sidebar">
        <button type="button" className="brand brand-button" onClick={() => go("/")}>
          <BrandLogo />
          <span className="brand-copy">
            <strong>SkillHive</strong>
            <small>ABILITY OPERATING SYSTEM</small>
          </span>
        </button>

        <div className="rail-label">导航 / INDEX</div>
        {nav}

        <div className="rail-footer">
          <div className="system-pulse">
            <span />
            <div>
              <strong>系统在线</strong>
              <small>所有节点运行正常</small>
            </div>
          </div>
          <button type="button" className="nav-item settings-link" onClick={() => go("/settings")}>
            <span className="nav-index">OS</span>
            <Settings size={19} strokeWidth={1.7} aria-hidden="true" />
            <span>个人设置</span>
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
              <span>SKILLHIVE / {selected === "/" ? "HOME" : selected.slice(1).toUpperCase()}</span>
              <strong>{routeTitles[selected]}</strong>
            </div>
          </div>
          <div className="top-actions">
            <div className="live-indicator">
              <CircleGauge size={15} aria-hidden="true" />
              <span>LIVE</span>
            </div>
            <Button
              type="text"
              className="icon-button"
              aria-label="切换主题"
              icon={
                dark ? (
                  <Sun size={19} strokeWidth={1.7} aria-hidden="true" />
                ) : (
                  <Moon size={19} strokeWidth={1.7} aria-hidden="true" />
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
                    icon: <Settings size={17} aria-hidden="true" />,
                    label: "个人设置",
                    onClick: () => navigate("/settings"),
                  },
                  {
                    key: "logout",
                    icon: <LogOut size={17} aria-hidden="true" />,
                    label: "退出登录",
                    onClick: logout,
                  },
                ],
              }}
            >
              <Button type="text" className="account-button">
                <Avatar size={32} className="account-avatar">
                  {user?.display_name?.slice(0, 1).toUpperCase()}
                </Avatar>
                <span className="account-name">{user?.display_name}</span>
                <ChevronDown size={15} aria-hidden="true" />
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
        width="min(340px, 88vw)"
        className="mobile-drawer"
        title={
          <div className="drawer-brand">
            <BrandLogo />
            <span>SkillHive</span>
          </div>
        }
      >
        {nav}
        <button type="button" className="nav-item settings-link" onClick={() => go("/settings")}>
          <span className="nav-index">OS</span>
          <Settings size={19} aria-hidden="true" />
          <span>个人设置</span>
        </button>
      </Drawer>
    </Layout>
  );
}
