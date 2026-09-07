import {
  ArrowRight,
  Eye,
  EyeOff,
  Info,
  LockKeyhole,
  Mail,
  UserRound,
} from "lucide-react";
import { App, Button, Form, Input } from "antd";
import { Link, Navigate, useNavigate } from "react-router-dom";

import { api, errorMessage } from "../api/client";
import { BrandLogo } from "../components/BrandLogo";
import { useAuthStore } from "../stores/auth";
import type { TokenResponse, User } from "../types";

interface LoginValues {
  username: string;
  password: string;
}

interface RegisterValues {
  username: string;
  display_name: string;
  email: string;
  password: string;
}

function AuthSide({ mode }: { mode: "login" | "register" }) {
  return (
    <aside className="auth-side">
      <div className="auth-wordmark">
        <BrandLogo decorative />
        <div className="brand-copy">
          <strong>SkillHive</strong>
          <small>团队 Skills 平台</small>
        </div>
      </div>
      <div className="auth-side-copy">
        <h1 className="auth-headline">
          {mode === "login"
            ? "让团队经验\n沉淀为可复用的能力"
            : "从一份方法开始\n构建团队能力库"}
        </h1>
        <p className="auth-tagline">
          {mode === "login"
            ? "登录 SkillHive，继续沉淀、共享与部署团队的最佳实践。"
            : "创建账号，建立你的私人 Skill 空间，并在清晰的边界内与团队协作。"}
        </p>
        <ul className="auth-points">
          <li>版本化保存每一次能力更新</li>
          <li>群组共享与全局技能统一管理</li>
          <li>一键部署到本地 Agent 工作区</li>
        </ul>
        <p className="auth-side-meta">化学，让生活更美好 · BETTER CHEMISTRY, BETTER LIFE</p>
      </div>
    </aside>
  );
}

function PasswordInput(
  props: Parameters<typeof Input.Password>[0] & { autoComplete?: string },
) {
  return (
    <Input.Password
      {...props}
      prefix={<LockKeyhole size={17} strokeWidth={1.7} aria-hidden="true" />}
      iconRender={(visible) =>
        visible ? (
          <EyeOff size={16} strokeWidth={1.7} aria-hidden="true" />
        ) : (
          <Eye size={16} strokeWidth={1.7} aria-hidden="true" />
        )
      }
    />
  );
}

export function LoginPage() {
  const navigate = useNavigate();
  const { message } = App.useApp();
  const user = useAuthStore((state) => state.user);
  const setSession = useAuthStore((state) => state.setSession);
  const [form] = Form.useForm<LoginValues>();

  if (user) return <Navigate to="/" replace />;

  const submit = async (values: LoginValues) => {
    try {
      const { data } = await api.post<TokenResponse>("/auth/login", values);
      setSession(data);
      message.success("登录成功");
      navigate("/");
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  return (
    <div className="auth-page">
      <AuthSide mode="login" />
      <main className="auth-main">
        <section className="auth-card">
          <div className="auth-head">
            <BrandLogo />
            <h2>登录 SkillHive</h2>
            <p>继续管理你的 Skills 与团队空间</p>
          </div>
          <Form form={form} layout="vertical" onFinish={submit}>
            <Form.Item
              label="用户名或邮箱"
              name="username"
              rules={[{ required: true, message: "请输入用户名或邮箱" }]}
            >
              <Input
                prefix={<UserRound size={17} strokeWidth={1.7} aria-hidden="true" />}
                autoComplete="username"
              />
            </Form.Item>
            <Form.Item
              label="密码"
              name="password"
              rules={[{ required: true, message: "请输入密码" }]}
            >
              <PasswordInput autoComplete="current-password" />
            </Form.Item>
            <Button type="primary" htmlType="submit" block icon={<ArrowRight size={16} aria-hidden="true" />}>
              登录
            </Button>
          </Form>
          <div className="dev-account">
            <Info size={15} aria-hidden="true" />
            <span>开发账号：admin / Admin123!，howie / User123!</span>
          </div>
          <p className="auth-switch">
            还没有账号？ <Link to="/register">创建账号</Link>
          </p>
        </section>
      </main>
    </div>
  );
}

export function RegisterPage() {
  const navigate = useNavigate();
  const { message } = App.useApp();
  const user = useAuthStore((state) => state.user);

  if (user) return <Navigate to="/" replace />;

  const submit = async (values: RegisterValues) => {
    try {
      await api.post<User>("/auth/register", values);
      message.success("账号已创建，请登录");
      navigate("/login");
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  return (
    <div className="auth-page">
      <AuthSide mode="register" />
      <main className="auth-main">
        <section className="auth-card">
          <div className="auth-head">
            <BrandLogo />
            <h2>创建账号</h2>
            <p>建立你的私人 Skill 空间</p>
          </div>
          <Form layout="vertical" onFinish={submit}>
            <Form.Item
              label="用户名"
              name="username"
              rules={[{ required: true }, { min: 3 }]}
            >
              <Input prefix={<UserRound size={17} strokeWidth={1.7} aria-hidden="true" />} />
            </Form.Item>
            <Form.Item
              label="显示名称"
              name="display_name"
              rules={[{ required: true }]}
            >
              <Input />
            </Form.Item>
            <Form.Item
              label="邮箱"
              name="email"
              rules={[{ required: true }, { type: "email" }]}
            >
              <Input prefix={<Mail size={17} strokeWidth={1.7} aria-hidden="true" />} />
            </Form.Item>
            <Form.Item
              label="密码"
              name="password"
              extra="至少 8 位，包含大小写字母和数字"
              rules={[{ required: true }, { min: 8 }]}
            >
              <PasswordInput />
            </Form.Item>
            <div className="auth-form-actions">
              <Button
                type="primary"
                htmlType="submit"
                block
                icon={<ArrowRight size={16} aria-hidden="true" />}
              >
                创建账号
              </Button>
              <Button block onClick={() => navigate("/login")}>
                返回登录
              </Button>
            </div>
          </Form>
        </section>
      </main>
    </div>
  );
}
