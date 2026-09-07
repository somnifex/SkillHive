import { App as AntApp, ConfigProvider, theme } from "antd";
import zhCN from "antd/locale/zh_CN";
import { useEffect } from "react";

import { AppRouter } from "./routes/AppRouter";
import { useAppearanceStore } from "./stores/auth";

const lightTokens = {
  colorPrimary: "#00489d",
  colorInfo: "#0096dc",
  colorLink: "#00489d",
  colorSuccess: "#5c9c31",
  colorWarning: "#c79000",
  colorError: "#d9363e",
  colorBgLayout: "#f2f6fb",
  colorBgContainer: "#ffffff",
  colorBorder: "#dfe6f0",
  colorBorderSecondary: "#e8eef6",
  colorText: "#262626",
  colorTextSecondary: "#5e7191",
  borderRadius: 8,
  controlHeight: 36,
  fontFamily:
    '"Segoe UI", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei UI", "Microsoft YaHei", system-ui, -apple-system, sans-serif',
};

const darkTokens = {
  colorBgLayout: "#0e1622",
  colorBgContainer: "#152130",
  colorBorder: "#26344c",
  colorBorderSecondary: "#1d2a40",
  colorText: "#e8eef7",
  colorTextSecondary: "#9db0cc",
};

export default function App() {
  const dark = useAppearanceStore((state) => state.dark);

  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);

  return (
    <ConfigProvider
      locale={zhCN}
      theme={{
        algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm,
        token: dark ? { ...lightTokens, ...darkTokens } : lightTokens,
        components: {
          Button: {
            fontWeight: 600,
            primaryShadow: "none",
          },
          Card: {
            borderRadiusLG: 12,
          },
          Menu: {
            itemBorderRadius: 8,
          },
          Table: {
            headerBg: "transparent",
            rowHoverBg: "rgba(0, 72, 157, 0.04)",
          },
        },
      }}
    >
      <AntApp>
        <AppRouter />
      </AntApp>
    </ConfigProvider>
  );
}
