import { Plus, Users } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Empty,
  Form,
  Input,
  Modal,
  Select,
  Switch,
  Table,
  Tag,
  Typography,
} from "antd";
import { useState } from "react";
import { useNavigate } from "react-router-dom";

import { api, errorMessage } from "../api/client";
import { PageHeader } from "../components/PageHeader";
import type { Group, Page } from "../types";

interface GroupForm {
  name: string;
  description: string;
  join_policy: string;
  allow_member_invite: boolean;
}

const roleLabels: Record<string, { label: string; color: string }> = {
  owner: { label: "群主", color: "gold" },
  admin: { label: "管理员", color: "blue" },
  member: { label: "成员", color: "default" },
};

const joinPolicyLabels: Record<string, string> = {
  invite_only: "仅邀请",
  approval_required: "申请后审批",
  public: "公开加入",
};

export function GroupsPage() {
  const navigate = useNavigate();
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [managedOnly, setManagedOnly] = useState(false);
  const [form] = Form.useForm<GroupForm>();
  const groups = useQuery({
    queryKey: ["groups", managedOnly],
    queryFn: () =>
      api
        .get<Page<Group>>("/groups", { params: { managed_only: managedOnly } })
        .then((r) => r.data),
  });
  const create = useMutation({
    mutationFn: (values: GroupForm) => api.post<Group>("/groups", values),
    onSuccess: ({ data }) => {
      message.success("群组已创建");
      setOpen(false);
      form.resetFields();
      queryClient.invalidateQueries({ queryKey: ["groups"] });
      navigate(`/groups/${data.id}`);
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  return (
    <>
      <PageHeader
        title="协作群组"
        description="管理你加入和负责的协作空间。"
        actions={
          <Button
            type="primary"
            icon={<Plus size={16} aria-hidden="true" />}
            onClick={() => setOpen(true)}
          >
            创建群组
          </Button>
        }
      />
      <div className="toolbar">
        <span>
          <Switch checked={managedOnly} onChange={setManagedOnly} size="small" /> 仅看我管理的
        </span>
      </div>
      <Table
        rowKey="id"
        loading={groups.isLoading}
        dataSource={groups.data?.items}
        locale={{
          emptyText: (
            <Empty
              image={<Users className="empty-icon" aria-hidden="true" />}
              description="还没有群组，创建一个开始团队协作"
            />
          ),
        }}
        onRow={(record) => ({
          onClick: () => navigate(`/groups/${record.id}`),
          className: "clickable-row",
        })}
        columns={[
          {
            title: "群组",
            render: (_: unknown, record: Group) => (
              <div className="group-name">
                <div className="group-icon">
                  <Users size={18} strokeWidth={1.7} aria-hidden="true" />
                </div>
                <div>
                  <Typography.Text strong>{record.name}</Typography.Text>
                  <Typography.Text type="secondary">
                    {record.description || "暂无描述"}
                  </Typography.Text>
                </div>
              </div>
            ),
          },
          {
            title: "我的角色",
            dataIndex: "current_user_role",
            render: (role: string) => {
              const meta = roleLabels[role];
              return meta ? <Tag color={meta.color}>{meta.label}</Tag> : (role ?? "—");
            },
          },
          {
            title: "加入策略",
            dataIndex: "join_policy",
            render: (policy: string) => joinPolicyLabels[policy] ?? policy,
          },
          {
            title: "状态",
            dataIndex: "status",
            render: (v: string) => (v === "active" ? "正常" : v),
          },
        ]}
      />
      <Modal
        open={open}
        title="创建群组"
        okText="创建"
        confirmLoading={create.isPending}
        onCancel={() => setOpen(false)}
        onOk={() => form.submit()}
      >
        <Form
          form={form}
          layout="vertical"
          initialValues={{ join_policy: "invite_only", allow_member_invite: false }}
          onFinish={(values) => create.mutate(values)}
        >
          <Form.Item name="name" label="群组名称" rules={[{ required: true }]}>
            <Input />
          </Form.Item>
          <Form.Item name="description" label="描述">
            <Input.TextArea rows={3} />
          </Form.Item>
          <Form.Item name="join_policy" label="加入策略">
            <Select
              options={Object.entries(joinPolicyLabels).map(([value, label]) => ({
                value,
                label,
              }))}
            />
          </Form.Item>
          <Form.Item
            name="allow_member_invite"
            label="允许普通成员邀请"
            valuePropName="checked"
          >
            <Switch />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
