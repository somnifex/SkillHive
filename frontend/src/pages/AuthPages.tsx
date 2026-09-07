import {
  ArrowRight,
  Eye,
  EyeOff,
  Info,
  LockKeyhole,
  Mail,
  Server,
  UserRound,
} from "lucide-react";
import { App, Button, Card, Form, Input, Space } from "antd";
import { useState } from "react";
import { Link, Navigate, useNavigate } from "react-router-dom";

import { api, currentServerUrl, errorMessage, setServerUrl } from "../api/client";
import { isDesktop, normalizeServerUrl, pingServer } from "../api/server";
import { BrandLogo } from "../components/BrandLogo";
import { useAuthStore } from "../stores/auth";
import type { TokenResponse, User } from "../types";

interface LoginValues {
  username: string;
  password: string;
}

/**
 * Server address control shown on the auth screens. The backend is an
 * independently deployed service, so the client points at it explicitly;
 * on the web the default stays same-origin, on the desktop the default is
 * the local development server.
 */
function ServerAddressField() {
  const { message } = App.useApp();
  const [value, setValue] = useState(currentServerUrl());
  const [testing, setTesting] = useState(false);

  const test = async () => {
    setTesting(true);
    try {
      setServerUrl(value);
      await pingServer(value);
      const target = normalizeServerUrl(value);
      message.success(target ? `已连接：${target}` : "已连接到同源部署");
    } catch (error) {
      message.error(error instanceof Error ? error.message : "连接失败");
    } finally {
      setTesting(false);
    }
  };

  return (
    <Form.Item
      label="服务端地址"
      extra="服务端与客户端分离部署时可指向其他地址；留空使用同源。"
    >
      <Space.Compact style={{ width: "100%" }}>
        <Input
          value={value}
          placeholder={isDesktop() ? "http://127.0.0.1:8000" : "同源部署（可留空）"}
          onChange={(event) => setValue(event.target.value)}
          onBlur={() => setServerUrl(value)}
          prefix={<Server size={18} strokeWidth={1.7} aria-hidden="true" />}
          autoComplete="url"
        />
        <Button onClick={test} loading={testing}>
          测试
        </Button>
      </Space.Compact>
    </Form.Item>
  );
}

function AuthArtwork({ mode }: { mode: "login" | "register" }) {
  return (
    <aside className="auth-artwork">
      <img
        src="/art/skillhive-orbit.png"
        alt=""
        width={1672}
        height={941}
        fetchPriority="high"
      />
      <div className="auth-artwork-shade" />
      <div className="auth-artwork-copy">
        <span>SKILLHIVE / ABILITY OS</span>
        <h1>{mode === "login" ? "知识不是库存，\n而是流动。" : "从一个方法，\n开始构建系统。"}</h1>
        <p>
          {mode === "login"
            ? "捕捉方法。连接团队。让每一次工作都留下可复用的能力。"
            : "创建你的私人能力空间，并在清晰的边界内与团队共同演化。"}
        </p>
      </div>
      <div className="auth-coordinates">
        <span>22.3193° N</span>
        <span>114.1694° E</span>
      </div>
    </aside>
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
      <AuthArtwork mode="login" />
      <main className="auth-panel">
        <Card className="auth-card" variant="borderless">
          <div className="auth-brand">
            <BrandLogo className="brand-logo-auth" />
            <span>WELCOME BACK / 01</span>
            <h2>登录 SkillHive</h2>
            <p>继续管理你的 Skills 与团队空间</p>
          </div>
          <Form form={form} layout="vertical" size="large" onFinish={submit}>
            <ServerAddressField />
            <Form.Item
              label="用户名或邮箱"
              name="username"
              rules={[{ required: true, message: "请输入用户名或邮箱" }]}
            >
              <Input
                prefix={<UserRound size={18} strokeWidth={1.7} aria-hidden="true" />}
                autoComplete="username"
              />
            </Form.Item>
            <Form.Item
              label="密码"
              name="password"
              rules={[{ required: true, message: "请输入密码" }]}
            >
              <Input.Password
                prefix={<LockKeyhole size={18} strokeWidth={1.7} aria-hidden="true" />}
                iconRender={(visible) =>
                  visible ? (
                    <EyeOff size={17} strokeWidth={1.7} aria-hidden="true" />
                  ) : (
                    <Eye size={17} strokeWidth={1.7} aria-hidden="true" />
                  )
                }
                autoComplete="current-password"
              />
            </Form.Item>
            <Button
              type="primary"
              htmlType="submit"
              block
              icon={<ArrowRight size={18} aria-hidden="true" />}
              iconPlacement="end"
            >
              登录
            </Button>
          </Form>
          <div className="dev-account">
            <Info size={16} aria-hidden="true" />
            <span>开发账号：admin / Admin123!，howie / User123!</span>
          </div>
          <p className="auth-switch">
            还没有账号？ <Link to="/register">创建账号</Link>
          </p>
        </Card>
      </main>
    </div>
  );
}

interface RegisterValues {
  username: string;
  display_name: string;
  email: string;
  password: string;
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
      <AuthArtwork mode="register" />
      <main className="auth-panel">
        <Card className="auth-card" variant="borderless">
          <div className="auth-brand">
            <BrandLogo className="brand-logo-auth" />
            <span>NEW IDENTITY / 02</span>
            <h2>创建账号</h2>
            <p>建立你的私人 Skill 空间</p>
          </div>
          <Form layout="vertical" size="large" onFinish={submit}>
            <ServerAddressField />
            <Form.Item
              label="用户名"
              name="username"
              rules={[{ required: true }, { min: 3 }]}
            >
              <Input prefix={<UserRound size={18} strokeWidth={1.7} aria-hidden="true" />} />
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
              <Input prefix={<Mail size={18} strokeWidth={1.7} aria-hidden="true" />} />
            </Form.Item>
            <Form.Item
              label="密码"
              name="password"
              extra="至少 8 位，包含大小写字母和数字"
              rules={[{ required: true }, { min: 8 }]}
            >
              <Input.Password
                prefix={<LockKeyhole size={18} strokeWidth={1.7} aria-hidden="true" />}
                iconRender={(visible) =>
                  visible ? (
                    <EyeOff size={17} strokeWidth={1.7} aria-hidden="true" />
                  ) : (
                    <Eye size={17} strokeWidth={1.7} aria-hidden="true" />
                  )
                }
              />
            </Form.Item>
            <div className="auth-form-actions">
              <Button
                type="primary"
                htmlType="submit"
                block
                icon={<ArrowRight size={18} aria-hidden="true" />}
                iconPlacement="end"
              >
                创建账号
              </Button>
              <Button block onClick={() => navigate("/login")}>
                返回登录
              </Button>
            </div>
          </Form>
        </Card>
      </main>
    </div>
  );
}
