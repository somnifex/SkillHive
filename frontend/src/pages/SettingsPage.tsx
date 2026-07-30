import { useMutation } from "@tanstack/react-query";
import { Eye, EyeOff } from "lucide-react";
import { App, Avatar, Button, Card, Descriptions, Form, Input, Switch, Typography } from "antd";

import { api, errorMessage } from "../api/client";
import { PageHeader } from "../components/PageHeader";
import { useAppearanceStore, useAuthStore } from "../stores/auth";

export function SettingsPage() {
  const { message } = App.useApp();
  const user = useAuthStore((state) => state.user);
  const clearSession = useAuthStore((state) => state.clearSession);
  const dark = useAppearanceStore((state) => state.dark);
  const toggle = useAppearanceStore((state) => state.toggle);
  const password = useMutation({
    mutationFn: (values: { current_password: string; new_password: string }) =>
      api.post("/auth/change-password", values),
    onSuccess: () => {
      message.success("密码已修改，请重新登录");
      clearSession();
      window.location.assign("/login");
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  return (
    <>
      <PageHeader title="个人设置" description="查看账号信息并调整安全与外观偏好。" />
      <div className="settings-grid">
        <Card title="账号资料">
          <div className="profile-heading">
            <Avatar size={64}>{user?.display_name.slice(0, 1)}</Avatar>
            <div>
              <Typography.Title level={4}>{user?.display_name}</Typography.Title>
              <Typography.Text type="secondary">@{user?.username}</Typography.Text>
            </div>
          </div>
          <Descriptions column={1}>
            <Descriptions.Item label="邮箱">{user?.email}</Descriptions.Item>
            <Descriptions.Item label="账号状态">{user?.status}</Descriptions.Item>
            <Descriptions.Item label="平台角色">
              {user?.is_global_admin ? "全局管理员" : "普通用户"}
            </Descriptions.Item>
          </Descriptions>
        </Card>
        <Card title="外观">
          <div className="setting-row">
            <div>
              <Typography.Text strong>深色模式</Typography.Text>
              <Typography.Paragraph type="secondary">
                主题偏好仅保存在当前设备。
              </Typography.Paragraph>
            </div>
            <Switch checked={dark} onChange={toggle} />
          </div>
        </Card>
        <Card title="修改密码">
          <Form layout="vertical" onFinish={(values) => password.mutate(values)}>
            <Form.Item
              name="current_password"
              label="当前密码"
              rules={[{ required: true }]}
            >
              <Input.Password
                iconRender={(visible) =>
                  visible ? (
                    <EyeOff size={17} strokeWidth={1.7} aria-hidden="true" />
                  ) : (
                    <Eye size={17} strokeWidth={1.7} aria-hidden="true" />
                  )
                }
              />
            </Form.Item>
            <Form.Item
              name="new_password"
              label="新密码"
              rules={[{ required: true }, { min: 8 }]}
            >
              <Input.Password
                iconRender={(visible) =>
                  visible ? (
                    <EyeOff size={17} strokeWidth={1.7} aria-hidden="true" />
                  ) : (
                    <Eye size={17} strokeWidth={1.7} aria-hidden="true" />
                  )
                }
              />
            </Form.Item>
            <Button type="primary" htmlType="submit" loading={password.isPending}>
              更新密码
            </Button>
          </Form>
        </Card>
      </div>
    </>
  );
}
