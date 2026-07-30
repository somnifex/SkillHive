import { LockOutlined, MailOutlined, UserOutlined } from "@ant-design/icons";
import { Alert, App, Button, Card, Form, Input, Space, Typography } from "antd";
import { Link, Navigate, useNavigate } from "react-router-dom";

import { api, errorMessage } from "../api/client";
import { BrandLogo } from "../components/BrandLogo";
import { useAuthStore } from "../stores/auth";
import type { TokenResponse, User } from "../types";

interface LoginValues {
  username: string;
  password: string;
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
      <Card className="auth-card" variant="borderless">
        <div className="auth-brand">
          <BrandLogo className="brand-logo-auth" />
          <Typography.Title level={2}>登录 SkillHive</Typography.Title>
          <Typography.Text type="secondary">
            继续管理你的 Skills 与团队空间
          </Typography.Text>
        </div>
        <Form form={form} layout="vertical" size="large" onFinish={submit}>
          <Form.Item
            label="用户名或邮箱"
            name="username"
            rules={[{ required: true, message: "请输入用户名或邮箱" }]}
          >
            <Input prefix={<UserOutlined />} autoComplete="username" />
          </Form.Item>
          <Form.Item
            label="密码"
            name="password"
            rules={[{ required: true, message: "请输入密码" }]}
          >
            <Input.Password
              prefix={<LockOutlined />}
              autoComplete="current-password"
            />
          </Form.Item>
          <Button type="primary" htmlType="submit" block>
            登录
          </Button>
        </Form>
        <Alert
          className="dev-account"
          type="info"
          showIcon
          title="开发账号：admin / Admin123!，howie / User123!"
        />
        <Typography.Text type="secondary">
          还没有账号？ <Link to="/register">创建账号</Link>
        </Typography.Text>
      </Card>
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
      <Card className="auth-card" variant="borderless">
        <div className="auth-brand">
          <BrandLogo className="brand-logo-auth" />
          <Typography.Title level={2}>创建账号</Typography.Title>
          <Typography.Text type="secondary">
            建立你的私人 Skill 空间
          </Typography.Text>
        </div>
        <Form layout="vertical" size="large" onFinish={submit}>
          <Form.Item
            label="用户名"
            name="username"
            rules={[{ required: true }, { min: 3 }]}
          >
            <Input prefix={<UserOutlined />} />
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
            <Input prefix={<MailOutlined />} />
          </Form.Item>
          <Form.Item
            label="密码"
            name="password"
            extra="至少 8 位，包含大小写字母和数字"
            rules={[{ required: true }, { min: 8 }]}
          >
            <Input.Password prefix={<LockOutlined />} />
          </Form.Item>
          <Space direction="vertical" className="full-width">
            <Button type="primary" htmlType="submit" block>
              创建账号
            </Button>
            <Button block onClick={() => navigate("/login")}>
              返回登录
            </Button>
          </Space>
        </Form>
      </Card>
    </div>
  );
}
