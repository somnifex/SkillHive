import {
  Copy,
  Download,
  Eye,
  FileArchive,
  PackageOpen,
  Pencil,
  Plus,
  Rocket,
  Search,
  Trash2,
} from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
  Button,
  Checkbox,
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
import { useEffect, useState, type Key } from "react";
import { useSearchParams } from "react-router-dom";

import { api, errorMessage } from "../api/client";
import {
  deploySkillBatch,
  exportSkillZip,
  discoverAgents,
  getDeploymentPrefs,
  hasDesktopCommands,
  hydrateSkillWorkspace,
  importSkillZip,
  listAgentProfiles,
  setDeploymentPrefs,
  type AgentDiscoveryResult,
} from "../api/desktop";
import { PageHeader } from "../components/PageHeader";
import type { Page, Skill, SkillVersion } from "../types";

interface SkillFormValues {
  name: string;
  slug: string;
  description: string;
  category: string;
  tags: string[];
  instructions: string;
  status?: string;
}

interface ZipImportForm {
  name: string;
  slug: string;
}

function slugify(raw: string): string {
  return raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fa5]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 140);
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

export function SkillsPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [params, setParams] = useSearchParams();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<string>();
  const [page, setPage] = useState(1);
  const [editing, setEditing] = useState<Skill | null>(null);
  const [detail, setDetail] = useState<Skill | null>(null);
  const [zipOpen, setZipOpen] = useState(false);
  const [deploying, setDeploying] = useState<Skill | null>(null);
  const [selectedRowKeys, setSelectedRowKeys] = useState<Key[]>([]);
  const [form] = Form.useForm<SkillFormValues>();
  const [zipForm] = Form.useForm<ZipImportForm>();
  const modalOpen = params.get("create") === "1" || Boolean(editing);

  const skills = useQuery({
    queryKey: ["skills", search, status, page],
    queryFn: () =>
      api
        .get<Page<Skill>>("/skills", {
          params: { query: search || undefined, status, page, page_size: 20 },
        })
        .then((r) => r.data),
  });
  const versions = useQuery({
    queryKey: ["skill-versions", detail?.id],
    queryFn: () =>
      api.get<SkillVersion[]>(`/skills/${detail!.id}/versions`).then((r) => r.data),
    enabled: Boolean(detail),
  });

  useEffect(() => {
    const requested = params.get("skill");
    if (requested && skills.data) {
      const match = skills.data.items.find((skill) => skill.id === requested);
      if (match) {
        api.get<Skill>(`/skills/${requested}`).then(({ data }) => setDetail(data));
      }
    }
  }, [params, skills.data]);

  const save = useMutation({
    mutationFn: async (values: SkillFormValues) => {
      if (editing) {
        return api.patch<Skill>(`/skills/${editing.id}`, {
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
        });
      }
      return api.post<Skill>("/skills", {
        ...values,
        content: { instructions: values.instructions },
      });
    },
    onSuccess: () => {
      message.success(editing ? "Skill 已更新" : "Skill 已创建");
      setEditing(null);
      setParams({});
      form.resetFields();
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const remove = async (skill: Skill) => {
    try {
      await api.delete(`/skills/${skill.id}`);
      message.success("Skill 已删除");
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  const copy = async (skill: Skill) => {
    try {
      await api.post(`/skills/${skill.id}/copy`);
      message.success("已创建副本");
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  // Desktop-only zip packaging (the server never stores archives; the
  // desktop imports them into its managed workspace and syncs blobs).
  const importZip = useMutation({
    mutationFn: async (values: ZipImportForm) => importSkillZip(values),
    onSuccess: (result) => {
      if (result) {
        message.success(
          `已从 ${result.sourceFileName} 导入 ${result.fileCount} 个文件，将自动同步到服务器`,
        );
      }
      setZipOpen(false);
      zipForm.resetFields();
      queryClient.invalidateQueries({ queryKey: ["skills"] });
    },
    onError: (error) => message.error(errorMessage(error)),
  });
  const exportZip = async (skill: Skill) => {
    try {
      const destination = await exportSkillZip(skill.id);
      if (destination) {
        message.success(`已导出到 ${destination}`);
      }
    } catch (error) {
      message.error(error instanceof Error ? error.message : errorMessage(error));
    }
  };

  // Hydration downloads a pulled skill's files into the managed workspace so
  // it can be deployed; it is additive and idempotent.
  const downloadSkill = useMutation({
    mutationFn: (skill: Skill) => hydrateSkillWorkspace(skill.id),
    onSuccess: (outcome) => {
      message.success(
        outcome.workspaceCreated
          ? `已下载到本地（新增 ${outcome.blobsDownloaded} 个文件）`
          : "本地已存在，无需下载",
      );
    },
    onError: (error) => message.error(error instanceof Error ? error.message : errorMessage(error)),
  });

  const openEdit = async (skill: Skill) => {
    const { data } = await api.get<Skill>(`/skills/${skill.id}`);
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
  };

  return (
    <>
      <PageHeader
        title="我的 Skills"
        description="这些内容仅对你可见，每次内容修改都会保留一个新版本。"
        actions={
          <>
            {hasDesktopCommands() && (
              <Button
                icon={<FileArchive size={16} aria-hidden="true" />}
                onClick={() => {
                  zipForm.resetFields();
                  setZipOpen(true);
                }}
              >
                从 zip 导入
              </Button>
            )}
            <Button
              type="primary"
              icon={<Plus size={16} aria-hidden="true" />}
              onClick={() => {
                setEditing(null);
                form.resetFields();
                setParams({ create: "1" });
              }}
            >
              创建 Skill
            </Button>
          </>
        }
      />
      <div className="toolbar">
        <Input
          allowClear
          placeholder="搜索名称或描述"
          prefix={<Search size={16} strokeWidth={1.7} aria-hidden="true" />}
          onChange={(event) => setSearch(event.target.value)}
          className="search-input"
        />
        <Select
          allowClear
          placeholder="全部状态"
          value={status}
          onChange={setStatus}
          style={{ minWidth: 140 }}
          options={Object.entries(statusLabels).map(([value, label]) => ({
            value,
            label,
          }))}
        />
      </div>
      {hasDesktopCommands() && selectedRowKeys.length > 0 && (
        <Space className="batch-toolbar" style={{ marginBottom: 12 }}>
          <Typography.Text type="secondary">已选 {selectedRowKeys.length} 项</Typography.Text>
          <Button
            size="small"
            icon={<Rocket size={14} aria-hidden="true" />}
            onClick={async () => {
              const defaults = (await getDeploymentPrefs()).defaultTargets;
              if (!defaults.length) {
                message.warning("请先在「Agent 部署」页配置默认部署目标");
                return;
              }
              let failed = 0;
              for (const skillId of selectedRowKeys) {
                const results = await deploySkillBatch(String(skillId), defaults);
                if (results.some((item) => item.error)) failed += 1;
              }
              if (failed === 0) message.success(`已批量部署 ${selectedRowKeys.length} 个 Skill`);
              else message.warning(`批量部署完成，其中 ${failed} 个失败，详见 Agent 部署页`);
              setSelectedRowKeys([]);
            }}
          >
            批量部署到默认目标
          </Button>
          <Popconfirm
            title="删除选中的 Skill？"
            description="内容会先进入回收站。"
            onConfirm={async () => {
              const ids = [...selectedRowKeys];
              let failed = 0;
              for (const id of ids) {
                try {
                  await api.delete(`/skills/${id}`);
                } catch {
                  failed += 1;
                }
              }
              setSelectedRowKeys([]);
              queryClient.invalidateQueries({ queryKey: ["skills"] });
              if (failed) message.warning(`${ids.length - failed} 个已删除，${failed} 个失败`);
              else message.success(`已删除 ${ids.length} 个 Skill`);
            }}
          >
            <Button danger size="small" icon={<Trash2 size={14} aria-hidden="true" />}>
              批量删除
            </Button>
          </Popconfirm>
        </Space>
      )}
      <Table
        rowKey="id"
        loading={skills.isLoading}
        dataSource={skills.data?.items}
        locale={{
          emptyText: (
            <Empty
              image={<PackageOpen className="empty-icon" aria-hidden="true" />}
              description="还没有 Skill，点击右上角「创建 Skill」开始沉淀"
            />
          ),
        }}
        pagination={{
          current: page,
          pageSize: skills.data?.page_size ?? 20,
          total: skills.data?.total,
          showSizeChanger: false,
          onChange: (next) => setPage(next),
        }}
        rowSelection={hasDesktopCommands() ? { selectedRowKeys, onChange: setSelectedRowKeys } : undefined}
        columns={[
          {
            title: "名称",
            dataIndex: "name",
            render: (value: string, record: Skill) => (
              <button
                className="link-button"
                onClick={async () => {
                  const { data } = await api.get<Skill>(`/skills/${record.id}`);
                  setDetail(data);
                }}
              >
                <Typography.Text strong>{value}</Typography.Text>
                <Typography.Text type="secondary">{record.slug}</Typography.Text>
              </button>
            ),
          },
          { title: "分类", dataIndex: "category", render: (v: string) => v || "—" },
          {
            title: "标签",
            dataIndex: "tags",
            render: (tags: string[]) =>
              tags.length ? tags.map((tag) => <Tag key={tag}>{tag}</Tag>) : "—",
          },
          {
            title: "状态",
            dataIndex: "status",
            render: (value: string) => (
              <Tag color={statusColors[value]}>{statusLabels[value] ?? value}</Tag>
            ),
          },
          {
            title: "操作",
            width: 220,
            render: (_: unknown, record: Skill) => (
              <Space>
                <Button
                  type="text"
                  aria-label="查看"
                  icon={<Eye size={16} aria-hidden="true" />}
                  onClick={async () => {
                    const { data } = await api.get<Skill>(`/skills/${record.id}`);
                    setDetail(data);
                  }}
                />
                <Button
                  type="text"
                  aria-label="编辑"
                  icon={<Pencil size={16} aria-hidden="true" />}
                  onClick={() => openEdit(record)}
                />
                <Button
                  type="text"
                  aria-label="创建副本"
                  icon={<Copy size={16} aria-hidden="true" />}
                  onClick={() => copy(record)}
                />
                {hasDesktopCommands() && (
                  <>
                    <Button
                      type="text"
                      aria-label="部署到 Agent"
                      icon={<Rocket size={16} aria-hidden="true" />}
                      onClick={() => setDeploying(record)}
                    />
                    <Button
                      type="text"
                      aria-label="下载到本地"
                      icon={<Download size={16} aria-hidden="true" />}
                      loading={downloadSkill.isPending && downloadSkill.variables?.id === record.id}
                      onClick={() => downloadSkill.mutate(record)}
                    />
                    <Button
                      type="text"
                      aria-label="导出为 zip"
                      icon={<FileArchive size={16} aria-hidden="true" />}
                      onClick={() => exportZip(record)}
                    />
                  </>
                )}
                <Popconfirm
                  title="删除这个 Skill？"
                  description="内容会先进入回收站，可在回收站中恢复或彻底删除。"
                  onConfirm={() => remove(record)}
                >
                  <Button
                    danger
                    type="text"
                    aria-label="删除"
                    icon={<Trash2 size={16} aria-hidden="true" />}
                  />
                </Popconfirm>
              </Space>
            ),
          },
        ]}
      />
      <Modal
        open={modalOpen}
        title={editing ? "编辑 Skill" : "创建 Skill"}
        okText="保存"
        confirmLoading={save.isPending}
        onCancel={() => {
          setEditing(null);
          setParams({});
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
              <Input disabled={Boolean(editing)} placeholder="my-skill" />
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
          {editing && (
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
            <Input.TextArea rows={8} />
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
              <Tag color={statusColors[detail.status]}>{statusLabels[detail.status] ?? detail.status}</Tag>
              <Tag>{detail.category || "未分类"}</Tag>
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
                size="small"
                rowKey="id"
                pagination={false}
                loading={versions.isLoading}
                dataSource={versions.data}
                columns={[
                  { title: "版本", dataIndex: "version" },
                  {
                    title: "状态",
                    dataIndex: "status",
                    render: (v: string) => statusLabels[v] ?? v,
                  },
                  { title: "变更", dataIndex: "change_log" },
                ]}
              />
            </div>
          </Space>
        )}
      </Drawer>
      <Modal
        open={zipOpen}
        title="从 zip 导入 Skill"
        okText="导入"
        confirmLoading={importZip.isPending}
        onCancel={() => setZipOpen(false)}
        onOk={() => zipForm.submit()}
      >
        <Typography.Paragraph type="secondary">
          选择本地 zip 包后，将在内置客户端中解包校验（SKILL.md 入口、文件数与大小限制、
          防路径穿越）并进入托管工作区，随后自动同步到服务器。
        </Typography.Paragraph>
        <Form
          form={zipForm}
          layout="vertical"
          onValuesChange={(_changed, values) => {
            if (values?.name && !zipForm.getFieldValue("slug")) {
              zipForm.setFieldValue("slug", slugify(values.name));
            }
          }}
          onFinish={(values) => importZip.mutate(values)}
        >
          <Form.Item name="slug" label="Slug" rules={[{ required: true }]}>
            <Input placeholder="my-skill" />
          </Form.Item>
        </Form>
      </Modal>
      <DeployModal
        skill={deploying}
        onClose={() => {
          setDeploying(null);
          queryClient.invalidateQueries({ queryKey: ["deployments"] });
        }}
      />
    </>
  );
}

interface DeployTargetOption {
  value: string;
  label: string;
}

function collectTargetOptions(
  discovery: AgentDiscoveryResult[],
  profiles: Awaited<ReturnType<typeof listAgentProfiles>>,
): DeployTargetOption[] {
  const options: DeployTargetOption[] = [];
  const seen = new Set<string>();
  for (const result of discovery) {
    for (const instance of result.instances) {
      if (seen.has(instance.id)) continue;
      seen.add(instance.id);
      options.push({ value: instance.id, label: instance.displayName });
    }
  }
  for (const profile of profiles) {
    if (seen.has(profile.id)) continue;
    seen.add(profile.id);
    options.push({ value: profile.id, label: profile.displayName });
  }
  return options;
}

function DeployModal({ skill, onClose }: { skill: Skill | null; onClose: () => void }) {
  const { message } = App.useApp();
  const desktop = hasDesktopCommands() && Boolean(skill);
  const [targets, setTargets] = useState<string[]>([]);
  const [remember, setRemember] = useState(false);

  const prefs = useQuery({
    queryKey: ["deployment-prefs-skill", skill?.id],
    queryFn: () => getDeploymentPrefs(skill!.id),
    enabled: desktop,
  });
  const discovery = useQuery({
    queryKey: ["desktop-agents"],
    queryFn: discoverAgents,
    enabled: desktop,
  });
  const profiles = useQuery({
    queryKey: ["agent-profiles"],
    queryFn: listAgentProfiles,
    enabled: desktop,
  });

  useEffect(() => {
    if (skill && prefs.data) {
      setTargets(prefs.data.skillTargets ?? prefs.data.defaultTargets);
    }
  }, [skill, prefs.data]);

  const deploy = useMutation({
    mutationFn: async (input: { ids: string[]; remember: boolean }) => {
      const results = await deploySkillBatch(skill!.id, input.ids);
      if (input.remember) {
        await setDeploymentPrefs({ skillId: skill!.id, profileIds: input.ids });
      }
      return results;
    },
    onSuccess: (items) => {
      const ok = items.filter((item) => !item.error).length;
      const failed = items.filter((item) => item.error);
      if (ok > 0) message.success(`已部署到 ${ok} 个目标`);
      for (const item of failed) {
        message.error(`${item.agentProfileId}: ${item.error}`);
      }
      onClose();
    },
    onError: (error) => message.error(error instanceof Error ? error.message : errorMessage(error)),
  });

  const options = collectTargetOptions(discovery.data ?? [], profiles.data ?? []);

  return (
    <Modal
      open={Boolean(skill)}
      title={skill ? `部署「${skill.name}」到 Agent` : ""}
      okText="部署"
      confirmLoading={deploy.isPending}
      onCancel={onClose}
      onOk={() => {
        if (!targets.length) {
          message.warning("请至少选择一个部署目标");
          return;
        }
        deploy.mutate({ ids: targets, remember });
      }}
    >
      <Typography.Paragraph type="secondary">
        部署前会自动下载远端创建的 Skill 到本地。默认目标可在「Agent 部署」页配置。
      </Typography.Paragraph>
      {prefs.data?.defaultTargets.length === 0 && (
        <Typography.Text type="warning">尚未配置默认部署目标，请选择目标。</Typography.Text>
      )}
      <Select
        mode="multiple"
        style={{ width: "100%" }}
        placeholder="选择部署目标"
        value={targets}
        loading={discovery.isLoading || profiles.isLoading || prefs.isLoading}
        options={options}
        onChange={setTargets}
      />
      <Checkbox
        style={{ marginTop: 12 }}
        checked={remember}
        onChange={(event) => setRemember(event.target.checked)}
      >
        记住此 Skill 的部署目标（覆盖全局默认）
      </Checkbox>
    </Modal>
  );
}
