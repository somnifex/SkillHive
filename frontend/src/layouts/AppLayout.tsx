import {
  AppstoreOutlined,
  BookOutlined,
  FileTextOutlined,
  HomeOutlined,
  LogoutOutlined,
  MoonOutlined,
  SafetyCertificateOutlined,
  SettingOutlined,
  SunOutlined,
  TeamOutlined,
} from "@ant-design/icons";
import { Avatar, Button, Dropdown, Layout, Menu, Space, Typography } from "antd";
import { Outlet, useLocation, useNavigate } from "react-router-dom";

import { api } from "../api/client";
import { BrandLogo } from "../components/BrandLogo";
import { useAppearanceStore, useAuthStore } from "../stores/auth";

const { Header, Sider, Content } = Layout;

export function AppLayout() {
  const navigate = useNavigate();
  const location = useLocation();
  const user = useAuthStore((state) => state.user);
  const clearSession = useAuthStore((state) => state.clearSession);
  const dark = useAppearanceStore((state) => state.dark);
  const toggleTheme = useAppearanceStore((state) => state.toggle);

  const selected =
    ["/skills", "/templates", "/groups", "/group-skills", "/admin", "/settings"].find(
      (path) => location.pathname.startsWith(path),
    ) ?? "/";

  const logout = async () => {
    try {
      await api.post("/auth/logout");
    } finally {
      clearSession();
      navigate("/login");
    }
  };

  return (
    <Layout className="app-shell">
      <Sider theme={dark ? "dark" : "light"} width={248} className="sidebar">
        <button className="brand brand-button" onClick={() => navigate("/")}>
          <BrandLogo />
          <div>
            <Typography.Title level={4}>SkillHive</Typography.Title>
            <Typography.Text type="secondary">团队能力中心</Typography.Text>
          </div>
        </button>
        <Menu
          mode="inline"
          selectedKeys={[selected]}
          onClick={({ key }) => navigate(key)}
          items={[
            { key: "/", icon: <HomeOutlined />, label: "首页" },
            { key: "/skills", icon: <BookOutlined />, label: "我的 Skills" },
            { key: "/templates", icon: <FileTextOutlined />, label: "模板库" },
            { key: "/groups", icon: <TeamOutlined />, label: "我的群组" },
            {
              key: "/group-skills",
              icon: <AppstoreOutlined />,
              label: "群组 Skills",
            },
            ...(user?.is_global_admin
              ? [
                  {
                    key: "/admin",
                    icon: <SafetyCertificateOutlined />,
                    label: "管理后台",
                  },
                ]
              : []),
            { type: "divider" as const },
            { key: "/settings", icon: <SettingOutlined />, label: "个人设置" },
          ]}
        />
      </Sider>
      <Layout>
        <Header className="topbar">
          <div />
          <Space>
            <Button
              type="text"
              aria-label="切换主题"
              icon={dark ? <SunOutlined /> : <MoonOutlined />}
              onClick={toggleTheme}
            />
            <Dropdown
              menu={{
                items: [
                  {
                    key: "settings",
                    icon: <SettingOutlined />,
                    label: "个人设置",
                    onClick: () => navigate("/settings"),
                  },
                  {
                    key: "logout",
                    icon: <LogoutOutlined />,
                    label: "退出登录",
                    onClick: logout,
                  },
                ],
              }}
            >
              <Button type="text" className="account-button">
                <Avatar size="small">
                  {user?.display_name?.slice(0, 1).toUpperCase()}
                </Avatar>
                <span>{user?.display_name}</span>
              </Button>
            </Dropdown>
          </Space>
        </Header>
        <Content className="content">
          <Outlet />
        </Content>
      </Layout>
    </Layout>
  );
}
