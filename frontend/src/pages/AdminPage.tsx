import { Plus } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Form,
  Input,
  InputNumber,
  Modal,
  Popconfirm,
  Select,
  Space,
  Switch,
  Table,
  Tabs,
  Tag,
  Typography,
} from "antd";
import { useMemo, useState } from "react";

import { api, errorMessage } from "../api/client";
import { PageHeader } from "../components/PageHeader";
import type { AuditLog, Group, Page, Skill, SystemSettings, User } from "../types";

interface GlobalSkillForm {
  name: string;
  slug: string;
  description: string;
  category: string;
  tags: string[];
  version: string;
  instructions: string;
}

interface SettingsFormValues {
  blob_storage_backend: "local" | "s3";
  s3_endpoint_url?: string | null;
  s3_bucket?: string | null;
  s3_prefix?: string | null;
  s3_region?: string | null;
  allow_registration?: boolean;
  max_package_bytes?: number | null;
}

interface GroupRow extends Group {
  children?: GroupRow[];
}

function buildGroupRows(groups: Group[]): GroupRow[] {
  const byId = new Map<string, GroupRow>();
  for (const group of groups) {
    byId.set(group.id, { ...group });
  }
  const roots: GroupRow[] = [];
  for (const row of byId.values()) {
    const parent = row.parent_id ? byId.get(row.parent_id) : undefined;
    if (parent && parent !== row) {
      parent.children = parent.children ?? [];
      parent.children.push(row);
    } else {
      roots.push(row);
    }
  }
  return roots;
}

export function AdminPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [skillOpen, setSkillOpen] = useState(false);
  const [userOpen, setUserOpen] = useState(false);
  const [resetTarget, setResetTarget] = useState<User | null>(null);
  const [form] = Form.useForm<GlobalSkillForm>();
  const [userForm] = Form.useForm<{
    username: string;
    display_name: string;
    email: string;
    password: string;
    is_global_admin: boolean;
  }>();
  const [resetForm] = Form.useForm<{ new_password: string }>();
  const users = useQuery({
    queryKey: ["admin-users"],
    queryFn: () => api.get<Page<User>>("/admin/users").then((r) => r.data),
  });
  const groupTree = useQuery({
    queryKey: ["admin-group-tree"],
    queryFn: () => api.get<Group[]>("/admin/groups/tree").then((r) => r.data),
  });
  const groupRows = useMemo(() => buildGroupRows(groupTree.data ?? []), [groupTree.data]);
  const skills = useQuery({
    queryKey: ["admin-skills"],
    queryFn: () => api.get<Page<Skill>>("/admin/skills").then((r) => r.data),
  });
  const audits = useQuery({
    queryKey: ["admin-audits"],
    queryFn: () => api.get<Page<AuditLog>>("/admin/audit-logs").then((r) => r.data),
  });
  const settings = useQuery({
    queryKey: ["admin-system-settings"],
    queryFn: () => api.get<SystemSettings>("/admin/system/settings").then((r) => r.data),
  });
  const createSkill = useMutation({
    mutationFn: (values: GlobalSkillForm) =>
      api.post("/admin/skills", {
        ...values,
        content: { instructions: values.instructions },
      }),
    onSuccess: () => {
      message.success("全局 Skill 草稿已创建");
      setSkillOpen(false);
      form.resetFields();
      queryClient.invalidateQueries({ queryKey: ["admin-skills"] });
    },
    onError: (error) => message.error(errorMessage(error)),
  });
  const createUser = useMutation({
    mutationFn: (values: {
      username: string;
      display_name: string;
      email: string;
      password: string;
      is_global_admin: boolean;
    }) => api.post("/admin/users", values),
    onSuccess: () => {
      message.success("用户已创建");
      setUserOpen(false);
      userForm.resetFields();
      queryClient.invalidateQueries({ queryKey: ["admin-users"] });
    },
    onError: (error) => message.error(errorMessage(error)),
  });
  const resetPassword = useMutation({
    mutationFn: (values: { user_id: string; new_password: string }) =>
      api.post(`/admin/users/${values.user_id}/reset-password`, {
        new_password: values.new_password,
      }),
    onSuccess: () => {
      message.success("密码已重置，该用户的所有会话已注销");
      setResetTarget(null);
      resetForm.resetFields();
    },
    onError: (error) => message.error(errorMessage(error)),
  });
  const saveSettings = useMutation({
    mutationFn: (values: SettingsFormValues) =>
      api.patch<SystemSettings>("/admin/system/settings", values),
    onSuccess: () => {
      message.success("系统设置已保存");
      queryClient.invalidateQueries({ queryKey: ["admin-system-settings"] });
    },
    onError: (error) => message.error(errorMessage(error)),
  });
  const setUserStatus = async (user: User) => {
    try {
      await api.patch(`/admin/users/${user.id}/status`, {
        status: user.status === "active" ? "disabled" : "active",
      });
      message.success("用户状态已更新");
      queryClient.invalidateQueries({ queryKey: ["admin-users"] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const deleteUser = async (user: User) => {
    try {
      await api.delete(`/admin/users/${user.id}`);
      message.success("用户已删除");
      queryClient.invalidateQueries({ queryKey: ["admin-users"] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const setGroupStatus = async (group: Group, status: "active" | "archived") => {
    try {
      await api.patch(`/admin/groups/${group.id}/status`, { status });
      message.success(status === "archived" ? "群组已归档" : "群组已恢复");
      queryClient.invalidateQueries({ queryKey: ["admin-group-tree"] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const publish = async (skill: Skill) => {
    try {
      await api.post(`/admin/skills/${skill.id}/publish`, {
        version_id: skill.current_version_id,
      });
      message.success("Skill 已发布");
      queryClient.invalidateQueries({ queryKey: ["admin-skills"] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const disable = async (skill: Skill) => {
    try {
      await api.post(`/admin/skills/${skill.id}/disable`);
      message.success("全局 Skill 已停用");
      queryClient.invalidateQueries({ queryKey: ["admin-skills"] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  return (
    <>
      <PageHeader
        title="管理后台"
        description="管理平台用户、群组、全局 Skills 和审计记录。"
      />
      <Tabs
        items={[
          {
            key: "users",
            label: `用户 ${users.data?.total ?? ""}`,
            children: (
              <>
                <div className="tab-actions">
                  <Button
                    type="primary"
                    icon={<Plus size={16} aria-hidden="true" />}
                    onClick={() => setUserOpen(true)}
                  >
                    创建用户
                  </Button>
                </div>
                <Table
                  rowKey="id"
                  loading={users.isLoading}
                  dataSource={users.data?.items}
                  columns={[
                    {
                      title: "用户",
                      render: (_: unknown, user: User) => (
                        <div>
                          <Typography.Text strong>{user.display_name}</Typography.Text>
                          <br />
                          <Typography.Text type="secondary">
                            @{user.username} · {user.email}
                          </Typography.Text>
                        </div>
                      ),
                    },
                    {
                      title: "角色",
                      render: (_: unknown, user: User) =>
                        user.is_global_admin ? (
                          <Tag color="gold">全局管理员</Tag>
                        ) : (
                          <Tag>用户</Tag>
                        ),
                    },
                    {
                      title: "状态",
                      dataIndex: "status",
                      render: (status: string) => (
                        <Tag color={status === "active" ? "green" : "red"}>
                          {status === "active" ? "正常" : "已禁用"}
                        </Tag>
                      ),
                    },
                    {
                      title: "操作",
                      render: (_: unknown, user: User) => (
                        <Space>
                          <Button size="small" onClick={() => setResetTarget(user)}>
                            重置密码
                          </Button>
                          {!user.is_global_admin && (
                            <>
                              <Popconfirm
                                title={user.status === "active" ? "禁用该用户？" : "重新启用该用户？"}
                                onConfirm={() => setUserStatus(user)}
                              >
                                <Button danger={user.status === "active"} size="small">
                                  {user.status === "active" ? "禁用" : "启用"}
                                </Button>
                              </Popconfirm>
                              <Popconfirm
                                title="删除该用户？需先转移其名下群组/Skill。"
                                onConfirm={() => deleteUser(user)}
                              >
                                <Button danger size="small">
                                  删除
                                </Button>
                              </Popconfirm>
                            </>
                          )}
                        </Space>
                      ),
                    },
                  ]}
                />
              </>
            ),
          },
          {
            key: "groups",
            label: `群组树 ${groupTree.data?.length ?? ""}`,
            children: (
              <Table
                rowKey="id"
                loading={groupTree.isLoading}
                dataSource={groupRows}
                expandable={{ indentSize: 28 }}
                columns={[
                  { title: "名称", dataIndex: "name" },
                  {
                    title: "上级群组",
                    dataIndex: "parent_name",
                    render: (value: string | null) => value ?? "—",
                  },
                  { title: "类型", dataIndex: "group_type" },
                  {
                    title: "状态",
                    dataIndex: "status",
                    render: (v: string) => (
                      <Tag color={v === "active" ? "green" : "warning"}>
                        {v === "active" ? "正常" : v}
                      </Tag>
                    ),
                  },
                  {
                    title: "操作",
                    render: (_: unknown, group: Group) =>
                      group.status === "active" ? (
                        <Popconfirm
                          title="归档该群组？"
                          onConfirm={() => setGroupStatus(group, "archived")}
                        >
                          <Button size="small" danger>
                            归档
                          </Button>
                        </Popconfirm>
                      ) : (
                        <Button size="small" onClick={() => setGroupStatus(group, "active")}>
                          恢复
                        </Button>
                      ),
                  },
                ]}
              />
            ),
          },
          {
            key: "settings",
            label: "系统设置",
            children: (
              <div className="settings-panel">
                <Typography.Paragraph type="secondary">
                  存储后端切换即时对新写入生效；已有对象按写入时记录的后端继续可读。S3
                  访问密钥仅通过服务端环境变量配置（S3_ACCESS_KEY_ID /
                  S3_SECRET_ACCESS_KEY），不会存入数据库或回显。
                </Typography.Paragraph>
                {settings.data && !settings.data.s3_credentials_configured && (
                  <Typography.Paragraph type="warning">
                    未检测到 S3 环境变量凭据，切换到 S3 前需在服务端配置。
                  </Typography.Paragraph>
                )}
                <Form
                  key={settings.data ? "loaded" : "loading"}
                  layout="vertical"
                  initialValues={settings.data ?? undefined}
                  onFinish={(values: SettingsFormValues) => saveSettings.mutate(values)}
                  disabled={settings.isLoading}
                >
                  <Form.Item name="blob_storage_backend" label="Skill 包存储后端">
                    <Select
                      options={[
                        { value: "local", label: "本地文件（单机/自托管）" },
                        { value: "s3", label: "S3 兼容对象存储" },
                      ]}
                    />
                  </Form.Item>
                  <div className="form-grid">
                    <Form.Item name="s3_endpoint_url" label="S3 Endpoint URL">
                      <Input placeholder="https://s3.example.com" />
                    </Form.Item>
                    <Form.Item name="s3_region" label="S3 Region">
                      <Input placeholder="us-east-1" />
                    </Form.Item>
                  </div>
                  <div className="form-grid">
                    <Form.Item name="s3_bucket" label="S3 Bucket">
                      <Input placeholder="skillhive-blobs" />
                    </Form.Item>
                    <Form.Item name="s3_prefix" label="S3 前缀">
                      <Input placeholder="（可选）" />
                    </Form.Item>
                  </div>
                  <div className="form-grid">
                    <Form.Item
                      name="allow_registration"
                      label="开放自助注册"
                      valuePropName="checked"
                    >
                      <Switch />
                    </Form.Item>
                    <Form.Item name="max_package_bytes" label="单包大小上限（字节）">
                      <InputNumber min={1} style={{ width: "100%" }} />
                    </Form.Item>
                  </div>
                  <Button type="primary" htmlType="submit" loading={saveSettings.isPending}>
                    保存设置
                  </Button>
                </Form>
              </div>
            ),
          },
          {
            key: "skills",
            label: `全局 Skills ${skills.data?.total ?? ""}`,
            children: (
              <>
                <div className="tab-actions">
                  <Button
                    type="primary"
                    icon={<Plus size={16} aria-hidden="true" />}
                    onClick={() => setSkillOpen(true)}
                  >
                    创建全局 Skill
                  </Button>
                </div>
                <Table
                  rowKey="id"
                  loading={skills.isLoading}
                  dataSource={skills.data?.items}
                  columns={[
                    {
                      title: "Skill",
                      render: (_: unknown, skill: Skill) => (
                        <div>
                          <Typography.Text strong>{skill.name}</Typography.Text>
                          <br />
                          <Typography.Text type="secondary">{skill.slug}</Typography.Text>
                        </div>
                      ),
                    },
                    { title: "分类", dataIndex: "category", render: (v: string) => v || "—" },
                    {
                      title: "状态",
                      dataIndex: "status",
                      render: (status: string) => (
                        <Tag
                          color={
                            status === "published"
                              ? "green"
                              : status === "draft"
                                ? "default"
                                : "warning"
                          }
                        >
                          {status === "published"
                            ? "已发布"
                            : status === "draft"
                              ? "草稿"
                              : status === "disabled"
                                ? "已停用"
                                : status}
                        </Tag>
                      ),
                    },
                    {
                      title: "操作",
                      render: (_: unknown, skill: Skill) => (
                        <Space>
                          {skill.status !== "published" && (
                            <Button type="link" onClick={() => publish(skill)}>
                              发布
                            </Button>
                          )}
                          {skill.status === "published" && (
                            <Button danger type="link" onClick={() => disable(skill)}>
                              停用
                            </Button>
                          )}
                        </Space>
                      ),
                    },
                  ]}
                />
              </>
            ),
          },
          {
            key: "audit",
            label: "审计日志",
            children: (
              <Table
                rowKey="id"
                loading={audits.isLoading}
                dataSource={audits.data?.items}
                columns={[
                  { title: "操作", dataIndex: "action" },
                  { title: "资源", dataIndex: "resource_type" },
                  { title: "资源 ID", dataIndex: "resource_id" },
                  {
                    title: "结果",
                    dataIndex: "result",
                    render: (result: string) => (
                      <Tag color={result === "success" ? "green" : "red"}>
                        {result === "success" ? "成功" : result}
                      </Tag>
                    ),
                  },
                  {
                    title: "时间",
                    dataIndex: "created_at",
                    render: (value: string) => new Date(value).toLocaleString(),
                  },
                ]}
              />
            ),
          },
        ]}
      />
      <Modal
        title="创建全局 Skill"
        open={skillOpen}
        okText="创建草稿"
        confirmLoading={createSkill.isPending}
        onCancel={() => setSkillOpen(false)}
        onOk={() => form.submit()}
        width={680}
      >
        <Form
          form={form}
          layout="vertical"
          initialValues={{ version: "0.1.0" }}
          onFinish={(values) => createSkill.mutate(values)}
        >
          <div className="form-grid">
            <Form.Item name="name" label="名称" rules={[{ required: true }]}>
              <Input />
            </Form.Item>
            <Form.Item name="slug" label="Slug" rules={[{ required: true }]}>
              <Input placeholder="global-skill" />
            </Form.Item>
          </div>
          <Form.Item name="description" label="描述">
            <Input.TextArea rows={2} />
          </Form.Item>
          <div className="form-grid">
            <Form.Item name="category" label="分类">
              <Input />
            </Form.Item>
            <Form.Item name="tags" label="标签">
              <Select mode="tags" />
            </Form.Item>
          </div>
          <Form.Item name="version" label="版本" rules={[{ required: true }]}>
            <Input />
          </Form.Item>
          <Form.Item name="instructions" label="Instructions" rules={[{ required: true }]}>
            <Input.TextArea rows={7} />
          </Form.Item>
        </Form>
      </Modal>
      <Modal
        title="创建用户"
        open={userOpen}
        okText="创建"
        confirmLoading={createUser.isPending}
        onCancel={() => setUserOpen(false)}
        onOk={() => userForm.submit()}
      >
        <Form
          form={userForm}
          layout="vertical"
          initialValues={{ is_global_admin: false }}
          onFinish={(values) => createUser.mutate(values)}
        >
          <div className="form-grid">
            <Form.Item name="username" label="用户名" rules={[{ required: true }]}>
              <Input />
            </Form.Item>
            <Form.Item name="display_name" label="显示名" rules={[{ required: true }]}>
              <Input />
            </Form.Item>
          </div>
          <Form.Item name="email" label="邮箱" rules={[{ required: true, type: "email" }]}>
            <Input />
          </Form.Item>
          <Form.Item name="password" label="初始密码" rules={[{ required: true, min: 8 }]}>
            <Input.Password />
          </Form.Item>
          <Form.Item name="is_global_admin" label="设为全局管理员" valuePropName="checked">
            <Switch />
          </Form.Item>
        </Form>
      </Modal>
      <Modal
        title={resetTarget ? `重置 ${resetTarget.display_name} 的密码` : "重置密码"}
        open={resetTarget !== null}
        okText="重置"
        confirmLoading={resetPassword.isPending}
        onCancel={() => setResetTarget(null)}
        onOk={() => resetForm.submit()}
      >
        <Form
          form={resetForm}
          layout="vertical"
          onFinish={(values) => {
            if (resetTarget) {
              resetPassword.mutate({ user_id: resetTarget.id, new_password: values.new_password });
            }
          }}
        >
          <Form.Item
            name="new_password"
            label="新密码（该用户的所有会话将被注销）"
            rules={[{ required: true, min: 8 }]}
          >
            <Input.Password />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
