import { Plus } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Form,
  Input,
  Modal,
  Popconfirm,
  Select,
  Space,
  Table,
  Tabs,
  Tag,
  Typography,
} from "antd";
import { useState } from "react";

import { api, errorMessage } from "../api/client";
import { PageHeader } from "../components/PageHeader";
import type { AuditLog, Group, Page, Skill, User } from "../types";

interface GlobalSkillForm {
  name: string;
  slug: string;
  description: string;
  category: string;
  tags: string[];
  version: string;
  instructions: string;
}

export function AdminPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [skillOpen, setSkillOpen] = useState(false);
  const [form] = Form.useForm<GlobalSkillForm>();
  const users = useQuery({
    queryKey: ["admin-users"],
    queryFn: () => api.get<Page<User>>("/admin/users").then((r) => r.data),
  });
  const groups = useQuery({
    queryKey: ["admin-groups"],
    queryFn: () => api.get<Page<Group>>("/admin/groups").then((r) => r.data),
  });
  const skills = useQuery({
    queryKey: ["admin-skills"],
    queryFn: () => api.get<Page<Skill>>("/admin/skills").then((r) => r.data),
  });
  const audits = useQuery({
    queryKey: ["admin-audits"],
    queryFn: () => api.get<Page<AuditLog>>("/admin/audit-logs").then((r) => r.data),
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
                      user.is_global_admin ? <Tag color="gold">global_admin</Tag> : <Tag>user</Tag>,
                  },
                  {
                    title: "状态",
                    dataIndex: "status",
                    render: (status: string) => (
                      <Tag color={status === "active" ? "geekblue" : "red"}>{status}</Tag>
                    ),
                  },
                  {
                    title: "操作",
                    render: (_: unknown, user: User) =>
                      !user.is_global_admin ? (
                        <Popconfirm
                          title={user.status === "active" ? "禁用该用户？" : "重新启用该用户？"}
                          onConfirm={() => setUserStatus(user)}
                        >
                          <Button danger={user.status === "active"} size="small">
                            {user.status === "active" ? "禁用" : "启用"}
                          </Button>
                        </Popconfirm>
                      ) : null,
                  },
                ]}
              />
            ),
          },
          {
            key: "groups",
            label: `群组 ${groups.data?.total ?? ""}`,
            children: (
              <Table
                rowKey="id"
                loading={groups.isLoading}
                dataSource={groups.data?.items}
                columns={[
                  { title: "名称", dataIndex: "name" },
                  { title: "类型", dataIndex: "group_type" },
                  { title: "Owner ID", dataIndex: "owner_id" },
                  { title: "状态", dataIndex: "status" },
                ]}
              />
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
                    icon={<Plus size={17} aria-hidden="true" />}
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
                    { title: "分类", dataIndex: "category" },
                    {
                      title: "状态",
                      dataIndex: "status",
                      render: (status: string) => <Tag>{status}</Tag>,
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
                      <Tag color={result === "success" ? "blue" : "red"}>{result}</Tag>
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
    </>
  );
}
