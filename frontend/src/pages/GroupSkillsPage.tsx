import {
  Blocks,
  Download,
  Eye,
  Pencil,
  Plus,
  Undo2,
  Trash2,
} from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  AutoComplete,
  Button,
  Drawer,
  Empty,
  Form,
  Input,
  Modal,
  Popconfirm,
  Select,
  Space,
  Table,
  Tag,
  Typography,
} from "antd";
import { useEffect, useState } from "react";
import { useSearchParams } from "react-router-dom";

import { api, errorMessage } from "../api/client";
import { PageHeader } from "../components/PageHeader";
import type { Grant, Group, Page, Skill, SkillVersion } from "../types";

interface SkillFormValues {
  name: string;
  slug: string;
  description: string;
  category: string;
  tags: string[];
  instructions: string;
  status?: string;
}

const statusLabels: Record<string, string> = {
  draft: "草稿",
  published: "已发布",
  disabled: "已停用",
  archived: "已归档",
};

const statusColors: Record<string, string> = {
  published: "blue",
  draft: "default",
  disabled: "warning",
  archived: "default",
};

const groupRoleLabels: Record<string, string> = {
  owner: "群主",
  admin: "管理员",
  member: "成员",
};

export function GroupSkillsPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [params, setParams] = useSearchParams();
  const [groupId, setGroupId] = useState<string>(params.get("group") ?? "");
  const [editing, setEditing] = useState<Skill | null>(null);
  const [detail, setDetail] = useState<Skill | null>(null);
  const [form] = Form.useForm<SkillFormValues>();

  const groups = useQuery({
    queryKey: ["groups", "skill-selector"],
    queryFn: () => api.get<Group[]>("/groups/tree").then((r) => r.data),
  });
  const group = groups.data?.find((item) => item.id === groupId);
  const shared = useQuery({
    queryKey: ["group-shared-skills", groupId],
    queryFn: () =>
      api
        .get<Page<Skill>>(`/groups/${groupId}/skills/shared`, {
          params: { page_size: 100 },
        })
        .then((r) => r.data),
    enabled: Boolean(groupId),
  });
  const trash = useQuery({
    queryKey: ["group-shared-skill-trash", groupId],
    queryFn: () =>
      api
        .get<Page<Skill>>(`/groups/${groupId}/skills/shared/trash`, {
          params: { page_size: 100 },
        })
        .then((r) => r.data),
    enabled: Boolean(groupId),
  });
  const grants = useQuery({
    queryKey: ["group-grants", groupId],
    queryFn: () => api.get<Grant[]>(`/groups/${groupId}/skills`).then((r) => r.data),
    enabled: Boolean(groupId),
  });
  const versions = useQuery({
    queryKey: ["group-skill-versions", groupId, detail?.id],
    queryFn: () =>
      api
        .get<SkillVersion[]>(
          `/groups/${groupId}/skills/shared/${detail!.id}/versions`,
        )
        .then((r) => r.data),
    enabled: Boolean(groupId && detail),
  });

  useEffect(() => {
    const firstGroup = groups.data?.[0];
    if (!groupId && firstGroup) setGroupId(firstGroup.id);
  }, [groupId, groups.data]);

  useEffect(() => {
    if (groupId && params.get("group") !== groupId) {
      setParams({ group: groupId }, { replace: true });
    }
  }, [groupId, params, setParams]);

  useEffect(() => {
    setDetail(null);
    setEditing(null);
    form.resetFields();
  }, [form, groupId]);

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["group-shared-skills", groupId] });
    queryClient.invalidateQueries({ queryKey: ["group-shared-skill-trash", groupId] });
    queryClient.invalidateQueries({ queryKey: ["group-skill-versions", groupId] });
  };

  const save = useMutation({
    mutationFn: async (values: SkillFormValues) => {
      if (!groupId) throw new Error("请先选择群组");
      if (editing?.id) {
        return api.patch<Skill>(
          `/groups/${groupId}/skills/shared/${editing.id}`,
          {
            name: values.name,
            description: values.description,
            category: values.category,
            tags: values.tags,
            status: values.status,
            ...(values.instructions !==
            (editing.current_version?.content.instructions ??
              editing.current_version?.content.skill_markdown ??
              "")
              ? { content: { instructions: values.instructions } }
              : {}),
          },
        );
      }
      return api.post<Skill>(`/groups/${groupId}/skills/shared`, {
        ...values,
        content: { instructions: values.instructions },
      });
    },
    onSuccess: () => {
      message.success(editing?.id ? "群组 Skill 已更新" : "群组 Skill 已创建");
      setEditing(null);
      form.resetFields();
      refresh();
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const openEdit = async (skill: Skill) => {
    if (!groupId) return;
    try {
      const { data } = await api.get<Skill>(
        `/groups/${groupId}/skills/shared/${skill.id}`,
      );
      setEditing(data);
      form.setFieldsValue({
        name: data.name,
        slug: data.slug,
        description: data.description,
        category: data.category,
        tags: data.tags,
        status: data.status,
        instructions:
          data.current_version?.content.instructions ??
          data.current_version?.content.skill_markdown ??
          "",
      });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  const openDetail = async (skill: Skill) => {
    if (!groupId) return;
    try {
      const { data } = await api.get<Skill>(
        `/groups/${groupId}/skills/shared/${skill.id}`,
      );
      setDetail(data);
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  const remove = async (skill: Skill) => {
    if (!groupId) return;
    try {
      await api.delete(`/groups/${groupId}/skills/shared/${skill.id}`);
      message.success("群组 Skill 已移入回收站");
      if (detail?.id === skill.id) setDetail(null);
      refresh();
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  const restore = async (skill: Skill) => {
    if (!groupId) return;
    try {
      await api.post(`/groups/${groupId}/skills/shared/${skill.id}/restore`);
      message.success("群组 Skill 已恢复");
      refresh();
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  const purge = async (skill: Skill) => {
    if (!groupId) return;
    try {
      await api.delete(`/groups/${groupId}/skills/shared/${skill.id}/purge`);
      message.success("群组 Skill 已彻底删除");
      refresh();
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  const startCreate = () => {
    form.resetFields();
    form.setFieldsValue({ status: "published", tags: [] });
    setEditing({
      id: "",
      name: "",
      slug: "",
      description: "",
      skill_type: "group",
      owner_user_id: null,
      group_id: groupId ?? null,
      category: "",
      tags: [],
      status: "published",
      current_version_id: null,
      current_version: null,
      created_by: "",
      created_at: "",
      updated_at: "",
      deleted_at: null,
    });
  };

  return (
    <>
      <PageHeader
        title="群组 Skills"
        description="群组成员可查看共享 Skill；群组管理员和作者本人可以继续维护内容与版本。"
        actions={
          <Button
            type="primary"
            icon={<Plus size={16} aria-hidden="true" />}
            disabled={!groupId}
            onClick={startCreate}
          >
            创建群组 Skill
          </Button>
        }
      />
      <Select
        className="group-selector"
        placeholder="选择群组"
        value={groupId}
        onChange={setGroupId}
        options={groups.data?.map((item) => ({
          value: item.id,
          label: `${item.name} · ${groupRoleLabels[item.current_user_role ?? ""] ?? "成员"}`,
        }))}
      />
      {groupId ? (
        <>
          <section className="group-skill-section">
            <div className="section-heading">
              <div>
                <Typography.Title level={4}>群组共享 Skill</Typography.Title>
                <Typography.Text type="secondary">
                  {group?.name} 内的自有 Skill，不需要额外启用授权。
                </Typography.Text>
              </div>
              <Tag color="blue">{shared.data?.total ?? 0} 个</Tag>
            </div>
            <Table
              rowKey="id"
              loading={shared.isLoading}
              dataSource={shared.data?.items}
              scroll={{ x: 760 }}
              locale={{
                emptyText: (
                  <Empty
                    image={<Blocks className="empty-icon" aria-hidden="true" />}
                    description="还没有群组共享 Skill，可从个人 Skill 发布或直接创建"
                  />
                ),
              }}
              columns={[
                {
                  title: "Skill",
                  render: (_: unknown, skill: Skill) => (
                    <button className="link-button" onClick={() => openDetail(skill)}>
                      <Typography.Text strong>{skill.name}</Typography.Text>
                      <Typography.Text type="secondary">{skill.slug}</Typography.Text>
                    </button>
                  ),
                },
                { title: "分类", dataIndex: "category", render: (v: string) => v || "—" },
                {
                  title: "状态",
                  dataIndex: "status",
                  render: (v: string) => (
                    <Tag color={statusColors[v]}>{statusLabels[v] ?? v}</Tag>
                  ),
                },
                {
                  title: "权限",
                  render: (_: unknown, skill: Skill) =>
                    skill.can_manage ? (
                      <Tag color="green">可管理</Tag>
                    ) : (
                      <Typography.Text type="secondary">仅查看</Typography.Text>
                    ),
                },
                {
                  title: "操作",
                  width: 180,
                  render: (_: unknown, skill: Skill) => (
                    <Space>
                      <Button
                        type="text"
                        aria-label={`查看 ${skill.name}`}
                        icon={<Eye size={16} aria-hidden="true" />}
                        onClick={() => openDetail(skill)}
                      />
                      {skill.can_manage && (
                        <>
                          <Button
                            type="text"
                            aria-label={`编辑 ${skill.name}`}
                            icon={<Pencil size={16} aria-hidden="true" />}
                            onClick={() => openEdit(skill)}
                          />
                          <Popconfirm
                            title="删除这个群组 Skill？"
                            description="内容会先进入群组回收站。"
                            onConfirm={() => remove(skill)}
                          >
                            <Button
                              danger
                              type="text"
                              aria-label={`删除 ${skill.name}`}
                              icon={<Trash2 size={16} aria-hidden="true" />}
                            />
                          </Popconfirm>
                        </>
                      )}
                    </Space>
                  ),
                },
              ]}
            />
          </section>

          <section className="group-skill-section">
            <div className="section-heading">
              <div>
                <Typography.Title level={4}>群组 Skill 回收站</Typography.Title>
                <Typography.Text type="secondary">
                  只有作者或群组管理员可以恢复或彻底删除。
                </Typography.Text>
              </div>
              <Tag>{trash.data?.total ?? 0} 个</Tag>
            </div>
            <Table
              rowKey="id"
              loading={trash.isLoading}
              dataSource={trash.data?.items}
              pagination={false}
              scroll={{ x: 520 }}
              locale={{ emptyText: <Empty description="回收站为空" /> }}
              columns={[
                { title: "Skill", dataIndex: "name" },
                { title: "Slug", dataIndex: "slug" },
                {
                  title: "操作",
                  render: (_: unknown, skill: Skill) =>
                    skill.can_manage ? (
                      <Space>
                        <Button type="link" onClick={() => restore(skill)}>
                          恢复
                        </Button>
                        <Popconfirm
                          title="彻底删除这个 Skill？"
                          description="此操作不可撤销。"
                          onConfirm={() => purge(skill)}
                        >
                          <Button danger type="link">
                            彻底删除
                          </Button>
                        </Popconfirm>
                      </Space>
                    ) : (
                      <Typography.Text type="secondary">仅查看</Typography.Text>
                    ),
                },
              ]}
            />
          </section>

          <section className="group-skill-section">
            <div className="section-heading">
              <div>
                <Typography.Title level={4}>已启用的全局 Skill</Typography.Title>
                <Typography.Text type="secondary">
                  由群组管理员从平台 Skill 目录授权，和群组自有 Skill 分开管理。
                </Typography.Text>
              </div>
              <Tag>{grants.data?.length ?? 0} 个</Tag>
            </div>
            <Table
              rowKey="id"
              loading={grants.isLoading}
              dataSource={grants.data}
              scroll={{ x: 640 }}
              locale={{
                emptyText: <Empty description="当前群组还没有启用全局 Skill" />,
              }}
              columns={[
                {
                  title: "Skill",
                  render: (_: unknown, grant: Grant) => (
                    <div>
                      <Typography.Text strong>{grant.skill?.name}</Typography.Text>
                      <br />
                      <Typography.Text type="secondary">
                        {grant.skill?.description}
                      </Typography.Text>
                    </div>
                  ),
                },
                {
                  title: "版本",
                  render: (_: unknown, grant: Grant) => (
                    <Tag color="blue">{grant.effective_version?.version ?? "—"}</Tag>
                  ),
                },
                {
                  title: "策略",
                  dataIndex: "version_policy",
                  render: (v: string) =>
                    v === "latest" ? "自动跟随最新" : v === "locked" ? "锁定版本" : v,
                },
              ]}
            />
          </section>
        </>
      ) : (
        <Empty
          className="page-empty"
          image={<Blocks className="empty-icon" aria-hidden="true" />}
          description="先选择一个群组"
        />
      )}

      <Modal
        open={Boolean(editing)}
        title={editing?.id ? "编辑群组 Skill" : "创建群组 Skill"}
        okText="保存"
        confirmLoading={save.isPending}
        onCancel={() => {
          setEditing(null);
          form.resetFields();
        }}
        onOk={() => form.submit()}
        width={680}
      >
        <Form form={form} layout="vertical" onFinish={(values) => save.mutate(values)}>
          <div className="form-grid">
            <Form.Item name="name" label="名称" rules={[{ required: true }]}>
              <Input />
            </Form.Item>
            <Form.Item name="slug" label="Slug" rules={[{ required: true }]}>
              <Input disabled={Boolean(editing?.id)} placeholder="my-shared-skill" />
            </Form.Item>
          </div>
          <Form.Item name="description" label="描述">
            <Input.TextArea rows={2} />
          </Form.Item>
          <div className="form-grid">
            <Form.Item name="category" label="分类">
              <AutoComplete />
            </Form.Item>
            <Form.Item name="tags" label="标签">
              <Select mode="tags" />
            </Form.Item>
          </div>
          {editing?.id && (
            <Form.Item name="status" label="状态">
              <Select
                options={Object.entries(statusLabels).map(([value, label]) => ({
                  value,
                  label,
                }))}
              />
            </Form.Item>
          )}
          <Form.Item
            name="instructions"
            label="Instructions"
            rules={[{ required: true, message: "请输入 Skill 指令" }]}
          >
            <Input.TextArea rows={10} />
          </Form.Item>
        </Form>
      </Modal>

      <Drawer
        open={Boolean(detail)}
        title={detail?.name}
        width={620}
        onClose={() => setDetail(null)}
      >
        {detail && (
          <Space direction="vertical" size="large" className="full-width">
            <div>
              <Tag color={statusColors[detail.status]}>
                {statusLabels[detail.status] ?? detail.status}
              </Tag>
              <Tag>{detail.category || "未分类"}</Tag>
              {detail.can_manage ? (
                <Tag color="green">你可以管理</Tag>
              ) : (
                <Tag>仅查看</Tag>
              )}
            </div>
            <Typography.Paragraph>{detail.description}</Typography.Paragraph>
            <div>
              <Typography.Title level={5}>当前指令</Typography.Title>
              <pre className="content-preview">
                {detail.current_version?.content.instructions ||
                  detail.current_version?.content.skill_markdown ||
                  "暂无内容"}
              </pre>
            </div>
            <div>
              <Typography.Title level={5}>版本历史</Typography.Title>
              <Table
                className="skill-version-table"
                size="small"
                rowKey="id"
                pagination={false}
                loading={versions.isLoading}
                dataSource={versions.data}
                scroll={{ x: 560 }}
                columns={[
                  { title: "版本", dataIndex: "version" },
                  {
                    title: "状态",
                    dataIndex: "status",
                    render: (v: string) => statusLabels[v] ?? v,
                  },
                  {
                    title: "标签",
                    dataIndex: "tags",
                    render: (tags: string[] | undefined, row: SkillVersion) =>
                      detail.can_manage ? (
                        <GroupVersionTagsCell
                          groupId={groupId!}
                          skillId={detail.id}
                          version={row.version}
                          tags={tags ?? []}
                        />
                      ) : tags?.length ? (
                        tags.map((tag) => <Tag key={tag}>{tag}</Tag>)
                      ) : (
                        "—"
                      ),
                  },
                  { title: "变更", dataIndex: "change_log", ellipsis: true },
                  {
                    title: "操作",
                    width: 176,
                    render: (_: unknown, row: SkillVersion) => (
                      <div className="skill-version-actions">
                        {detail.can_manage && (
                          <Popconfirm
                            title={`回滚到 ${row.version}？`}
                            description="会以该版本内容创建一个新草稿版本。"
                            onConfirm={async () => {
                              try {
                                await api.post(
                                  `/groups/${groupId}/skills/shared/${detail.id}/rollback`,
                                  { version: row.version },
                                );
                                message.success("已创建回滚版本");
                                refresh();
                                const { data } = await api.get<Skill>(
                                  `/groups/${groupId}/skills/shared/${detail.id}`,
                                );
                                setDetail(data);
                              } catch (error) {
                                message.error(errorMessage(error));
                              }
                            }}
                          >
                            <Button
                              size="small"
                              type="text"
                              icon={<Undo2 size={14} aria-hidden="true" />}
                            >
                              回滚
                            </Button>
                          </Popconfirm>
                        )}
                        <Button
                          size="small"
                          type="text"
                          icon={<Download size={14} aria-hidden="true" />}
                          onClick={async () => {
                            try {
                              const { data } = await api.get(
                                `/groups/${groupId}/skills/shared/${detail.id}/versions/${row.version}/export`,
                                { responseType: "blob" },
                              );
                              const url = URL.createObjectURL(data as Blob);
                              const anchor = document.createElement("a");
                              anchor.href = url;
                              anchor.download = `${detail.slug}_${row.version}.zip`;
                              anchor.click();
                              URL.revokeObjectURL(url);
                            } catch (error) {
                              message.error(errorMessage(error));
                            }
                          }}
                        >
                          下载
                        </Button>
                      </div>
                    ),
                  },
                ]}
              />
            </div>
          </Space>
        )}
      </Drawer>
    </>
  );
}

function GroupVersionTagsCell({
  groupId,
  skillId,
  version,
  tags,
}: {
  groupId: string;
  skillId: string;
  version: string;
  tags: string[];
}) {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState(tags);
  const save = async () => {
    try {
      await api.put(
        `/groups/${groupId}/skills/shared/${skillId}/versions/${version}/tags`,
        { tags: value },
      );
      message.success("版本标签已更新");
      setOpen(false);
      queryClient.invalidateQueries({ queryKey: ["group-skill-versions", groupId, skillId] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };
  return (
    <Space wrap>
      {tags.length ? tags.map((tag) => <Tag key={tag}>{tag}</Tag>) : <span>—</span>}
      <Button size="small" type="link" onClick={() => setOpen(true)}>
        编辑
      </Button>
      <Modal
        open={open}
        title={`编辑 ${version} 标签`}
        okText="保存"
        onCancel={() => setOpen(false)}
        onOk={save}
      >
        <Select
          mode="tags"
          style={{ width: "100%" }}
          value={value}
          onChange={setValue}
          placeholder="输入标签后回车"
        />
      </Modal>
    </Space>
  );
}
