import { Result, Spin } from "antd";
import { useEffect, type ReactNode } from "react";
import {
  BrowserRouter,
  Navigate,
  Route,
  Routes,
} from "react-router-dom";

import { AppLayout } from "../layouts/AppLayout";
import { AdminPage } from "../pages/AdminPage";
import { LoginPage, RegisterPage } from "../pages/AuthPages";
import { DashboardPage } from "../pages/DashboardPage";
import { GroupDetailPage } from "../pages/GroupDetailPage";
import { GroupsPage } from "../pages/GroupsPage";
import { GroupSkillsPage } from "../pages/GroupSkillsPage";
import { SettingsPage } from "../pages/SettingsPage";
import { SkillsPage } from "../pages/SkillsPage";
import { TemplatesPage } from "../pages/TemplatesPage";
import { useAuthStore } from "../stores/auth";

export function ProtectedRoute({
  children,
  admin = false,
}: {
  children: ReactNode;
  admin?: boolean;
}) {
  const user = useAuthStore((state) => state.user);
  const loading = useAuthStore((state) => state.loading);
  if (loading) {
    return (
      <div className="page-spinner">
        <Spin size="large" />
      </div>
    );
  }
  if (!user) return <Navigate to="/login" replace />;
  if (admin && !user.is_global_admin) {
    return <Result status="403" title="无权访问" subTitle="此页面仅对全局管理员开放。" />;
  }
  return children;
}

export function AppRouter() {
  const bootstrap = useAuthStore((state) => state.bootstrap);
  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  return (
    <BrowserRouter>
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route path="/register" element={<RegisterPage />} />
        <Route
          element={
            <ProtectedRoute>
              <AppLayout />
            </ProtectedRoute>
          }
        >
          <Route index element={<DashboardPage />} />
          <Route path="skills" element={<SkillsPage />} />
          <Route path="templates" element={<TemplatesPage />} />
          <Route path="groups" element={<GroupsPage />} />
          <Route path="groups/:groupId" element={<GroupDetailPage />} />
          <Route path="group-skills" element={<GroupSkillsPage />} />
          <Route path="settings" element={<SettingsPage />} />
          <Route
            path="admin"
            element={
              <ProtectedRoute admin>
                <AdminPage />
              </ProtectedRoute>
            }
          />
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
