import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Eye, EyeOff } from "lucide-react";
import {
  App,
  Avatar,
  Button,
  Card,
  Descriptions,
  Form,
  Input,
  Popconfirm,
  Switch,
  Table,
  Tag,
  Typography,
} from "antd";

import { api, errorMessage } from "../api/client";
import {
  hasDesktopCommands,
  listConflicts,
  resolveConflict,
  type DesktopConflict,
} from "../api/desktop";
import { PageHeader } from "../components/PageHeader";
import { useAppearanceStore, useAuthStore } from "../stores/auth";

function ConflictCenter() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const conflicts = useQuery({
    queryKey: ["desktop-conflicts"],
    queryFn: listConflicts,
    refetchInterval: 30_000,
  });
  const resolve = useMutation({
    mutationFn: (input: { skillId: string; mode: "keep_local" | "keep_remote" }) =>
      resolveConflict(input.skillId, input.mode, true),
    onSuccess: () => {
      message.success("冲突已解决");
      void queryClient.invalidateQueries({ queryKey: ["desktop-conflicts"] });
      void queryClient.invalidateQueries({ queryKey: ["skills"] });
      void queryClient.invalidateQueries({ queryKey: ["desktop-sync-state"] });
    },
    onError: (error) => message.error(error instanceof Error ? error.message : errorMessage(error)),
  });

  const rows = conflicts.data ?? [];
  return (
    <Card title="同步冲突">
      <Typography.Paragraph type="secondary">
        当本地未同步的修改与服务器变更冲突时会记录在这里，任一方都不会被静默覆盖。
      </Typography.Paragraph>
      {rows.length === 0 ? (
        <Typography.Text type="secondary">没有待解决的冲突。</Typography.Text>
      ) : (
        <Table
          rowKey="skillId"
          size="small"
          pagination={false}
          dataSource={rows}
          columns={[
            { title: "本地名称", dataIndex: "localName", ellipsis: true },
            { title: "本地 Slug", dataIndex: "localSlug" },
            {
              title: "远端版本",
              dataIndex: "remoteHeadRevision",
              render: (value: number | null) => value ?? "未知",
            },
            {
              title: "远端包 Hash",
              dataIndex: "remotePackageManifestHash",
              ellipsis: true,
              render: (value: string | null) => value ?? "未知",
            },
            {
              title: "状态",
              render: () => <Tag color="warning">待解决</Tag>,
            },
            {
              title: "处理",
              width: 220,
              render: (_: unknown, record: DesktopConflict) => (
                <>
                  <Popconfirm
                    title="保留本地版本？"
                    description="服务器版本将被本机版本覆盖并推送。"
                    onConfirm={() => resolve.mutate({ skillId: record.skillId, mode: "keep_local" })}
                  >
                    <Tag style={{ cursor: "pointer" }}>保留本地</Tag>
                  </Popconfirm>
                  <Popconfirm
                    title="采用服务器版本？"
                    description="本机未同步的修改将被放弃。"
                    onConfirm={() => resolve.mutate({ skillId: record.skillId, mode: "keep_remote" })}
                  >
                    <Tag style={{ cursor: "pointer" }}>保留服务器</Tag>
                  </Popconfirm>
                </>
              ),
            },
          ]}
        />
      )}
    </Card>
  );
}

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
            <Avatar size={60}>{user?.display_name.slice(0, 1)}</Avatar>
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
            <div className="form-grid">
              <Form.Item
                name="current_password"
                label="当前密码"
                rules={[{ required: true }]}
              >
                <Input.Password
                  iconRender={(visible) =>
                    visible ? (
                      <EyeOff size={16} strokeWidth={1.7} aria-hidden="true" />
                    ) : (
                      <Eye size={16} strokeWidth={1.7} aria-hidden="true" />
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
                      <EyeOff size={16} strokeWidth={1.7} aria-hidden="true" />
                    ) : (
                      <Eye size={16} strokeWidth={1.7} aria-hidden="true" />
                    )
                  }
                />
              </Form.Item>
            </div>
            <Button type="primary" htmlType="submit" loading={password.isPending}>
              更新密码
            </Button>
          </Form>
        </Card>
        {hasDesktopCommands() && <ConflictCenter />}
      </div>
    </>
  );
}
