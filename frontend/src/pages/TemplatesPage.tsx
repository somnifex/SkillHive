import { FilePlus2, Info, Pencil, Plus, Rocket, Search, Trash2 } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  App,
  Button,
  Card,
  Empty,
  Form,
  Input,
  Modal,
  Popconfirm,
  Select,
  Space,
  Tag,
  Typography,
} from "antd";
import { useState } from "react";
import { useNavigate } from "react-router-dom";

import { api, errorMessage } from "../api/client";
import { PageHeader } from "../components/PageHeader";
import { useAuthStore } from "../stores/auth";
import type { Group, Page, Skill, SkillTemplate } from "../types";

interface TemplateFormValues {
  name: string;
  slug: string;
  description: string;
  scope_type: "personal" | "group" | "global";
  group_id?: string;
  category: string;
  tags: string[];
  instructions: string;
  status: "draft" | "published" | "disabled";
}

interface InstantiateFormValues {
  name: string;
  slug: string;
  description: string;
  category: string;
  tags: string[];
  instructions: string;
}

const scopeMeta = {
  personal: { label: "个人", color: "blue" },
  group: { label: "群组", color: "purple" },
  global: { label: "全局", color: "gold" },
} as const;

export function TemplatesPage() {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const user = useAuthStore((state) => state.user);
  const [search, setSearch] = useState("");
  const [scope, setScope] = useState<string>();
  const [editing, setEditing] = useState<SkillTemplate | null>(null);
  const [creating, setCreating] = useState(false);
  const [usingTemplate, setUsingTemplate] = useState<SkillTemplate | null>(null);
  const [templateForm] = Form.useForm<TemplateFormValues>();
  const [instantiateForm] = Form.useForm<InstantiateFormValues>();
  const selectedScope = Form.useWatch("scope_type", templateForm);

  const templates = useQuery({
    queryKey: ["templates", search, scope],
    queryFn: () =>
      api
        .get<Page<SkillTemplate>>("/templates", {
          params: { query: search || undefined, scope_type: scope },
        })
        .then((response) => response.data),
  });
  const managedGroups = useQuery({
    queryKey: ["template-managed-groups", user?.is_global_admin],
    queryFn: () =>
      api
        .get<Page<Group>>(
          user?.is_global_admin ? "/admin/groups" : "/groups",
          user?.is_global_admin
            ? { params: { page_size: 100 } }
            : { params: { page_size: 100, managed_only: true } },
        )
        .then((response) => response.data),
  });

  const saveTemplate = useMutation({
    mutationFn: (values: TemplateFormValues) => {
      const content = {
        ...(editing?.content ?? {}),
        instructions: values.instructions,
      };
      if (editing) {
        return api.patch<SkillTemplate>(`/templates/${editing.id}`, {
          name: values.name,
          description: values.description,
          category: values.category,
          tags: values.tags,
          content,
          status: values.status,
        });
      }
      return api.post<SkillTemplate>("/templates", {
        ...values,
        group_id: values.scope_type === "group" ? values.group_id : undefined,
        content,
      });
    },
    onSuccess: () => {
      message.success(editing ? "模板已更新" : "模板已创建");
      setEditing(null);
      setCreating(false);
      templateForm.resetFields();
      queryClient.invalidateQueries({ queryKey: ["templates"] });
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const instantiate = useMutation({
    mutationFn: (values: InstantiateFormValues) =>
      api.post<Skill>(`/templates/${usingTemplate!.id}/instantiate`, values),
    onSuccess: ({ data }) => {
      message.success("已根据模板创建私人 Skill");
      setUsingTemplate(null);
      instantiateForm.resetFields();
      queryClient.invalidateQueries({ queryKey: ["skills"] });
      navigate(`/skills?skill=${data.id}`);
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const openCreate = () => {
    setEditing(null);
    setCreating(true);
    templateForm.setFieldsValue({
      scope_type: "personal",
      status: "published",
      tags: [],
      instructions: "",
    });
  };

  const openEdit = (template: SkillTemplate) => {
    setCreating(false);
    setEditing(template);
    templateForm.setFieldsValue({
      name: template.name,
      slug: template.slug,
      description: template.description,
      scope_type: template.scope_type,
      group_id: template.group_id ?? undefined,
      category: template.category,
      tags: template.tags,
      instructions: template.content.instructions ?? "",
      status: template.status as TemplateFormValues["status"],
    });
  };

  const openInstantiate = (template: SkillTemplate) => {
    setUsingTemplate(template);
    instantiateForm.setFieldsValue({
      name: "",
      slug: "",
      description: template.description,
      category: template.category,
      tags: template.tags,
      instructions: template.content.instructions ?? "",
    });
  };

  const remove = async (template: SkillTemplate) => {
    try {
      await api.delete(`/templates/${template.id}`);
      message.success("模板已删除");
      queryClient.invalidateQueries({ queryKey: ["templates"] });
    } catch (error) {
      message.error(errorMessage(error));
    }
  };

  return (
    <>
      <PageHeader
        title="Skill 模板库"
        description="从个人、群组或全局模板快速创建属于你的私人 Skill。"
        actions={
          <Button
            type="primary"
            icon={<Plus size={17} aria-hidden="true" />}
            onClick={openCreate}
          >
            添加模板
          </Button>
        }
      />
      <Alert
        className="template-format-alert"
        type="info"
        showIcon
        icon={<Info size={18} strokeWidth={1.7} aria-hidden="true" />}
        message="OpenAI 推荐格式"
        description="默认模板以 SKILL.md 为入口，生成包含 name、description 前置信息和清晰工作流的 Skill。"
      />
      <div className="toolbar">
        <Input
          allowClear
          placeholder="搜索模板名称或用途"
          prefix={<Search size={17} strokeWidth={1.7} aria-hidden="true" />}
          onChange={(event) => setSearch(event.target.value)}
          className="search-input"
        />
        <Select
          allowClear
          placeholder="全部范围"
          value={scope}
          onChange={setScope}
          options={Object.entries(scopeMeta).map(([value, meta]) => ({
            value,
            label: meta.label,
          }))}
        />
      </div>
      {templates.data?.items.length ? (
        <div className="template-grid">
          {templates.data.items.map((template) => {
            const meta = scopeMeta[template.scope_type];
            return (
              <Card
                key={template.id}
                loading={templates.isLoading}
                className={template.is_default ? "template-card template-card-default" : "template-card"}
                title={
                  <Space wrap>
                    <Typography.Text strong>{template.name}</Typography.Text>
                    {template.is_default && <Tag color="geekblue">默认</Tag>}
                  </Space>
                }
                extra={
                  <Tag color={meta.color}>
                    {template.scope_type === "group" && template.group_name
                      ? template.group_name
                      : meta.label}
                  </Tag>
                }
                actions={[
                  <Button
                    key="use"
                    type="link"
                    icon={<Rocket size={17} aria-hidden="true" />}
                    onClick={() => openInstantiate(template)}
                  >
                    使用模板
                  </Button>,
                  ...(template.can_manage
                    ? [
                        <Button
                          key="edit"
                          type="text"
                          icon={<Pencil size={17} aria-hidden="true" />}
                          onClick={() => openEdit(template)}
                        >
                          编辑
                        </Button>,
                        <Popconfirm
                          key="delete"
                          title={template.is_default ? "默认模板不可删除" : "删除这个模板？"}
                          disabled={template.is_default}
                          onConfirm={() => remove(template)}
                        >
                          <Button
                            danger
                            type="text"
                            disabled={template.is_default}
                            icon={<Trash2 size={17} aria-hidden="true" />}
                          >
                            删除
                          </Button>
                        </Popconfirm>,
                      ]
                    : []),
                ]}
              >
                <Typography.Paragraph ellipsis={{ rows: 2 }}>
                  {template.description || "暂无描述"}
                </Typography.Paragraph>
                <Typography.Text type="secondary">{template.slug}</Typography.Text>
                <div className="template-tags">
                  {template.tags.map((tag) => (
                    <Tag key={tag}>{tag}</Tag>
                  ))}
                  {template.status !== "published" && <Tag>{template.status}</Tag>}
                </div>
              </Card>
            );
          })}
        </div>
      ) : (
        <Empty
          className="page-empty"
          image={<FilePlus2 className="empty-icon" aria-hidden="true" />}
          description={templates.isLoading ? "正在加载模板" : "没有匹配的模板"}
        />
      )}

      <Modal
        open={creating || Boolean(editing)}
        title={editing ? "编辑模板" : "添加模板"}
        okText="保存模板"
        width={720}
        confirmLoading={saveTemplate.isPending}
        onCancel={() => {
          setCreating(false);
          setEditing(null);
          templateForm.resetFields();
        }}
        onOk={() => templateForm.submit()}
      >
        <Form
          form={templateForm}
          layout="vertical"
          onFinish={(values) => saveTemplate.mutate(values)}
        >
          <div className="form-grid">
            <Form.Item name="name" label="模板名称" rules={[{ required: true }]}>
              <Input placeholder="例如：团队代码评审" />
            </Form.Item>
            <Form.Item name="slug" label="Slug" rules={[{ required: true }]}>
              <Input
                disabled={Boolean(editing)}
                maxLength={64}
                placeholder="team-code-review"
              />
            </Form.Item>
          </div>
          <div className="form-grid">
            <Form.Item name="scope_type" label="适用范围" rules={[{ required: true }]}>
              <Select
                disabled={Boolean(editing)}
                options={[
                  { value: "personal", label: "仅自己" },
                  { value: "group", label: "指定群组" },
                  ...(user?.is_global_admin
                    ? [{ value: "global", label: "全局所有用户" }]
                    : []),
                ]}
              />
            </Form.Item>
            {selectedScope === "group" ? (
              <Form.Item name="group_id" label="适用群组" rules={[{ required: true }]}>
                <Select
                  showSearch
                  optionFilterProp="label"
                  loading={managedGroups.isLoading}
                  options={managedGroups.data?.items.map((group) => ({
                    value: group.id,
                    label: group.name,
                  }))}
                />
              </Form.Item>
            ) : (
              <Form.Item name="status" label="状态">
                <Select
                  disabled={Boolean(editing?.is_default)}
                  options={[
                    { value: "published", label: "已发布" },
                    { value: "draft", label: "草稿" },
                    ...(editing ? [{ value: "disabled", label: "已停用" }] : []),
                  ]}
                />
              </Form.Item>
            )}
          </div>
          {selectedScope === "group" && (
            <Form.Item name="status" label="状态">
              <Select
                options={[
                  { value: "published", label: "已发布" },
                  { value: "draft", label: "草稿" },
                  ...(editing ? [{ value: "disabled", label: "已停用" }] : []),
                ]}
              />
            </Form.Item>
          )}
          <Form.Item name="description" label="触发描述">
            <Input.TextArea
              rows={2}
              placeholder="说明这个 Skill 做什么，以及何时应该使用它。"
            />
          </Form.Item>
          <div className="form-grid">
            <Form.Item name="category" label="分类">
              <Input />
            </Form.Item>
            <Form.Item name="tags" label="标签">
              <Select mode="tags" />
            </Form.Item>
          </div>
          <Form.Item
            name="instructions"
            label="工作流指令"
            rules={[{ required: true, message: "请输入工作流指令" }]}
          >
            <Input.TextArea
              rows={10}
              placeholder="写清楚输入、执行步骤、输出要求、边界和停止条件。"
            />
          </Form.Item>
        </Form>
      </Modal>

      <Modal
        open={Boolean(usingTemplate)}
        title={`使用模板：${usingTemplate?.name ?? ""}`}
        okText="创建私人 Skill"
        width={720}
        confirmLoading={instantiate.isPending}
        onCancel={() => {
          setUsingTemplate(null);
          instantiateForm.resetFields();
        }}
        onOk={() => instantiateForm.submit()}
      >
        <Form
          form={instantiateForm}
          layout="vertical"
          onFinish={(values) => instantiate.mutate(values)}
        >
          <div className="form-grid">
            <Form.Item name="name" label="Skill 名称" rules={[{ required: true }]}>
              <Input placeholder="我的定制 Skill" />
            </Form.Item>
            <Form.Item name="slug" label="Slug" rules={[{ required: true }]}>
              <Input maxLength={64} placeholder="my-custom-skill" />
            </Form.Item>
          </div>
          <Form.Item
            name="description"
            label="触发描述"
            rules={[{ required: true, message: "请描述何时使用这个 Skill" }]}
          >
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
          <Form.Item
            name="instructions"
            label="定制工作流指令"
            rules={[{ required: true }]}
          >
            <Input.TextArea rows={10} />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );
}
