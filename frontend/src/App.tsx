import { App as AntApp, ConfigProvider, theme } from "antd";
import zhCN from "antd/locale/zh_CN";
import { useEffect } from "react";

import { AppRouter } from "./routes/AppRouter";
import { useAppearanceStore } from "./stores/auth";

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
        token: {
          colorPrimary: "#6757f7",
          colorInfo: "#4f7cff",
          colorSuccess: "#4f7cff",
          colorWarning: "#f97316",
          colorError: "#f24b6a",
          colorLink: "#6757f7",
          colorBgLayout: dark ? "#07070b" : "#f2f0eb",
          colorBgContainer: dark ? "#111116" : "#fbfaf7",
          colorBorder: dark ? "#30303a" : "#d9d5cc",
          colorBorderSecondary: dark ? "#25252d" : "#e4e0d7",
          colorText: dark ? "#f3f1ed" : "#141318",
          colorTextSecondary: dark ? "#aaa7b2" : "#66616d",
          borderRadius: 12,
          controlHeight: 44,
          fontFamily:
            "'Aptos', 'Inter', ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
        },
        components: {
          Button: {
            borderRadius: 999,
            controlHeightLG: 52,
            fontWeight: 650,
            primaryShadow: "none",
          },
          Card: {
            borderRadiusLG: 18,
          },
          Drawer: {
            colorBgElevated: dark ? "#111116" : "#fbfaf7",
          },
          Menu: {
            itemBorderRadius: 10,
            itemHeight: 48,
          },
          Modal: {
            borderRadiusLG: 22,
          },
          Table: {
            headerBg: "transparent",
            headerColor: dark ? "#86828f" : "#746f79",
            rowHoverBg: dark ? "#181820" : "#f5f2ed",
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
