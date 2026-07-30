import { ArrowLeft, Blocks, Plus, Settings, Trash2, UserPlus } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Descriptions,
  Empty,
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
import { useNavigate, useParams } from "react-router-dom";

import { api, errorMessage } from "../api/client";
import { PageHeader } from "../components/PageHeader";
import { useAuthStore } from "../stores/auth";
import type { Grant, Group, Member, Skill, SkillVersion } from "../types";

export function GroupDetailPage() {
  const { groupId = "" } = useParams();
  const navigate = useNavigate();
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const currentUser = useAuthStore((state) => state.user);
  const [inviteOpen, setInviteOpen] = useState(false);
  const [skillOpen, setSkillOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [form] = Form.useForm();
  const [settingsForm] = Form.useForm();
  const [enableForm] = Form.useForm();
  const selectedSkillId = Form.useWatch("skill_id", enableForm) as string | undefined;
  const versionPolicy = Form.useWatch("version_policy", enableForm) as string | undefined;

  const group = useQuery({
    queryKey: ["group", groupId],
    queryFn: () => api.get<Group>(`/groups/${groupId}`).then((r) => r.data),
  });
  const members = useQuery({
    queryKey: ["group-members", groupId],
    queryFn: () => api.get<Member[]>(`/groups/${groupId}/members`).then((r) => r.data),
  });
  const grants = useQuery({
    queryKey: ["group-grants", groupId],
    queryFn: () =>
      api.get<Grant[]>(`/groups/${groupId}/skills`).then((r) => r.data),
  });
  const isManager = ["owner", "admin"].includes(group.data?.current_user_role ?? "");
  const catalog = useQuery({
    queryKey: ["group-skill-catalog", groupId],
    queryFn: () =>
      api.get<Skill[]>(`/groups/${groupId}/skills/catalog`).then((r) => r.data),
    enabled: isManager,
  });
  const catalogVersions = useQuery({
    queryKey: ["group-skill-catalog-versions", groupId, selectedSkillId],
    queryFn: () =>
      api
        .get<SkillVersion[]>(
          `/groups/${groupId}/skills/catalog/${selectedSkillId}/versions`,
        )
        .then((r) => r.data),
    enabled: isManager && Boolean(selectedSkillId),
  });
  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["group", groupId] });
    queryClient.invalidateQueries({ queryKey: ["group-members", groupId] });
    queryClient.invalidateQueries({ queryKey: ["group-grants", groupId] });
    queryClient.invalidateQueries({ queryKey: ["groups"] });
  };
  const invite = useMutation({
    mutationFn: (values: { identity: string }) =>
      api.post(`/groups/${groupId}/members/invite`, values),
    onSuccess: () => {
      message.success("邀请已发送");
      setInviteOpen(false);
      form.resetFields();
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const setRole = async (member: Member, role: "admin" | "member") => {
    try {
      await api.patch(`/groups/${groupId}/members/${member.user_id}`, { role });
      message.success("成员角色已更新");
      refresh();
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const removeMember = async (member: Member) => {
    try {
      await api.delete(`/groups/${groupId}/members/${member.user_id}`);
      message.success("成员已移除");
      refresh();
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const enableSkill = async (values: {
    skill_id: string;
    version_policy: string;
    locked_version_id?: string;
  }) => {
    try {
      await api.post(`/groups/${groupId}/skills/${values.skill_id}`, {
        version_policy: values.version_policy,
        locked_version_id:
          values.version_policy === "locked" ? values.locked_version_id : null,
      });
      message.success("Skill 已启用");
      setSkillOpen(false);
      enableForm.resetFields();
      refresh();
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const disableSkill = async (skillId: string) => {
    try {
      await api.delete(`/groups/${groupId}/skills/${skillId}`);
      message.success("本群组已停用该 Skill");
      refresh();
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const saveSettings = async (values: Partial<Group>) => {
    try {
      await api.patch(`/groups/${groupId}`, values);
      message.success("群组设置已更新");
      setSettingsOpen(false);
      refresh();
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const leave = async () => {
    try {
      await api.post(`/groups/${groupId}/leave`);
      message.success("已退出群组");
      navigate("/groups");
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  const dissolve = async () => {
    try {
      await api.delete(`/groups/${groupId}`);
      message.success("群组已解散");
      navigate("/groups");
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  return (
    <>
      <Button
        type="text"
        icon={<ArrowLeft size={17} aria-hidden="true" />}
        onClick={() => navigate("/groups")}
      >
        返回群组
      </Button>
      <PageHeader
        title={group.data?.name ?? "群组详情"}
        description={group.data?.description || "暂无描述"}
        actions={
          <>
            {isManager && (
              <Button
                icon={<Settings size={17} aria-hidden="true" />}
                onClick={() => {
                  settingsForm.setFieldsValue(group.data);
                  setSettingsOpen(true);
                }}
              >
                群组设置
              </Button>
            )}
            {group.data?.current_user_role !== "owner" && (
              <Popconfirm title="确定退出这个群组？" onConfirm={leave}>
                <Button>退出群组</Button>
              </Popconfirm>
            )}
          </>
        }
      />
      <Tabs
        items={[
          {
            key: "overview",
            label: "概览",
            children: (
              <Descriptions bordered column={2}>
                <Descriptions.Item label="我的角色">
                  <Tag>{group.data?.current_user_role}</Tag>
                </Descriptions.Item>
                <Descriptions.Item label="群组类型">{group.data?.group_type}</Descriptions.Item>
                <Descriptions.Item label="加入策略">{group.data?.join_policy}</Descriptions.Item>
                <Descriptions.Item label="成员邀请">
                  {group.data?.allow_member_invite ? "成员可邀请" : "仅管理员"}
                </Descriptions.Item>
              </Descriptions>
            ),
          },
          {
            key: "members",
            label: `成员 ${members.data?.length ?? 0}`,
            children: (
              <>
                {isManager && (
                  <div className="tab-actions">
                    <Button
                      type="primary"
                      icon={<UserPlus size={17} aria-hidden="true" />}
                      onClick={() => setInviteOpen(true)}
                    >
                      邀请成员
                    </Button>
                  </div>
                )}
                <Table
                  rowKey="id"
                  loading={members.isLoading}
                  dataSource={members.data}
                  columns={[
                    {
                      title: "成员",
                      render: (_: unknown, member: Member) => (
                        <div>
                          <Typography.Text strong>
                            {member.user?.display_name}
                          </Typography.Text>
                          <br />
                          <Typography.Text type="secondary">
                            @{member.user?.username}
                          </Typography.Text>
                        </div>
                      ),
                    },
                    { title: "角色", dataIndex: "role", render: (v: string) => <Tag>{v}</Tag> },
                    {
                      title: "操作",
                      render: (_: unknown, member: Member) =>
                        group.data?.current_user_role === "owner" &&
                        member.role !== "owner" ? (
                          <Space>
                            <Button
                              size="small"
                              onClick={() =>
                                setRole(member, member.role === "admin" ? "member" : "admin")
                              }
                            >
                              {member.role === "admin" ? "撤销管理员" : "设为管理员"}
                            </Button>
                            <Popconfirm
                              title="移除该成员？"
                              onConfirm={() => removeMember(member)}
                            >
                              <Button danger size="small">
                                移除
                              </Button>
                            </Popconfirm>
                          </Space>
                        ) : null,
                    },
                  ]}
                />
              </>
            ),
          },
          {
            key: "skills",
            label: `群组 Skills ${grants.data?.length ?? 0}`,
            children: (
              <>
                {isManager && (
                  <div className="tab-actions">
                    <Button
                      type="primary"
                      icon={<Plus size={17} aria-hidden="true" />}
                      onClick={() => setSkillOpen(true)}
                    >
                      启用全局 Skill
                    </Button>
                  </div>
                )}
                {grants.data?.length ? (
                  <Table
                    rowKey="id"
                    dataSource={grants.data}
                    columns={[
                      {
                        title: "Skill",
                        render: (_: unknown, grant: Grant) => grant.skill?.name,
                      },
                      { title: "版本策略", dataIndex: "version_policy" },
                      {
                        title: "当前版本",
                        render: (_: unknown, grant: Grant) =>
                          grant.effective_version?.version ?? "—",
                      },
                      {
                        title: "操作",
                        render: (_: unknown, grant: Grant) =>
                          isManager ? (
                            <Button danger type="link" onClick={() => disableSkill(grant.skill_id)}>
                              停用
                            </Button>
                          ) : null,
                      },
                    ]}
                  />
                ) : (
                  <Empty
                    image={<Blocks className="empty-icon" aria-hidden="true" />}
                    description="当前群组还没有启用 Skill"
                  />
                )}
              </>
            ),
          },
        ]}
      />
      <Modal
        title="邀请成员"
        open={inviteOpen}
        okText="发送邀请"
        confirmLoading={invite.isPending}
        onCancel={() => setInviteOpen(false)}
        onOk={() => form.submit()}
      >
        <Form form={form} layout="vertical" onFinish={(v) => invite.mutate(v)}>
          <Form.Item
            name="identity"
            label="用户名或邮箱"
            rules={[{ required: true }]}
          >
            <Input />
          </Form.Item>
        </Form>
      </Modal>
      <Modal
        title="启用全局 Skill"
        open={skillOpen}
        footer={null}
        onCancel={() => setSkillOpen(false)}
      >
        <Form
          form={enableForm}
          layout="vertical"
          initialValues={{ version_policy: "latest" }}
          onFinish={enableSkill}
        >
          <Form.Item name="skill_id" label="Skill" rules={[{ required: true }]}>
            <Select
              showSearch
              optionFilterProp="label"
              options={catalog.data?.map((skill) => ({
                value: skill.id,
                label: `${skill.name} · ${skill.current_version?.version ?? ""}`,
              }))}
            />
          </Form.Item>
          <Form.Item name="version_policy" label="版本策略">
            <Select
              options={[
                { value: "latest", label: "自动跟随最新发布版本" },
                { value: "locked", label: "锁定指定发布版本" },
              ]}
            />
          </Form.Item>
          {versionPolicy === "locked" && (
            <Form.Item
              name="locked_version_id"
              label="锁定版本"
              rules={[{ required: true, message: "请选择要锁定的版本" }]}
            >
              <Select
                loading={catalogVersions.isLoading}
                options={catalogVersions.data?.map((version) => ({
                  value: version.id,
                  label: `${version.version} · ${version.change_log || "无变更说明"}`,
                }))}
              />
            </Form.Item>
          )}
          <Button type="primary" htmlType="submit" block>
            启用
          </Button>
        </Form>
      </Modal>
      <Modal
        title="群组设置"
        open={settingsOpen}
        okText="保存"
        onCancel={() => setSettingsOpen(false)}
        onOk={() => settingsForm.submit()}
        footer={(_, { OkBtn, CancelBtn }) => (
          <div className="danger-footer">
            {group.data?.current_user_role === "owner" && (
              <Popconfirm title="确定解散群组？" onConfirm={dissolve}>
                <Button danger icon={<Trash2 size={17} aria-hidden="true" />}>
                  解散群组
                </Button>
              </Popconfirm>
            )}
            <Space>
              <CancelBtn />
              <OkBtn />
            </Space>
          </div>
        )}
      >
        <Form form={settingsForm} layout="vertical" onFinish={saveSettings}>
          <Form.Item name="name" label="名称" rules={[{ required: true }]}>
            <Input />
          </Form.Item>
          <Form.Item name="description" label="描述">
            <Input.TextArea />
          </Form.Item>
          {group.data?.current_user_role === "owner" && (
            <Form.Item name="join_policy" label="加入策略">
              <Select
                options={[
                  { value: "invite_only", label: "仅邀请" },
                  { value: "approval_required", label: "申请后审批" },
                  { value: "public", label: "公开加入" },
                ]}
              />
            </Form.Item>
          )}
        </Form>
      </Modal>
      {currentUser?.id === group.data?.owner_id && members.data && (
        <span className="sr-only">你是当前群组所有者</span>
      )}
    </>
  );
}
