import { LoaderCircle } from "lucide-react";
import { Result } from "antd";
import { lazy, Suspense, useEffect, type ReactNode } from "react";
import {
  BrowserRouter,
  Navigate,
  Route,
  Routes,
} from "react-router-dom";

import { AppLayout } from "../layouts/AppLayout";
import { useAuthStore } from "../stores/auth";

const LoginPage = lazy(() =>
  import("../pages/AuthPages").then((module) => ({ default: module.LoginPage })),
);
const RegisterPage = lazy(() =>
  import("../pages/AuthPages").then((module) => ({ default: module.RegisterPage })),
);
const DashboardPage = lazy(() =>
  import("../pages/DashboardPage").then((module) => ({ default: module.DashboardPage })),
);
const SkillsPage = lazy(() =>
  import("../pages/SkillsPage").then((module) => ({ default: module.SkillsPage })),
);
const TemplatesPage = lazy(() =>
  import("../pages/TemplatesPage").then((module) => ({ default: module.TemplatesPage })),
);
const GroupsPage = lazy(() =>
  import("../pages/GroupsPage").then((module) => ({ default: module.GroupsPage })),
);
const GroupDetailPage = lazy(() =>
  import("../pages/GroupDetailPage").then((module) => ({ default: module.GroupDetailPage })),
);
const GroupSkillsPage = lazy(() =>
  import("../pages/GroupSkillsPage").then((module) => ({ default: module.GroupSkillsPage })),
);
const SettingsPage = lazy(() =>
  import("../pages/SettingsPage").then((module) => ({ default: module.SettingsPage })),
);
const AdminPage = lazy(() =>
  import("../pages/AdminPage").then((module) => ({ default: module.AdminPage })),
);

function PageLoader() {
  return (
    <div className="page-spinner">
      <LoaderCircle className="loader-icon" size={30} aria-label="正在加载" />
    </div>
  );
}

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
    return <PageLoader />;
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
      <Suspense fallback={<PageLoader />}>
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
      </Suspense>
    </BrowserRouter>
  );
}
