import {
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
  EyeOutlined,
  PlusOutlined,
} from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  App,
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

export function SkillsPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [params, setParams] = useSearchParams();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<string>();
  const [editing, setEditing] = useState<Skill | null>(null);
  const [detail, setDetail] = useState<Skill | null>(null);
  const [form] = Form.useForm<SkillFormValues>();
  const modalOpen = params.get("create") === "1" || Boolean(editing);

  const skills = useQuery({
    queryKey: ["skills", search, status],
    queryFn: () =>
      api
        .get<Page<Skill>>("/skills", { params: { query: search || undefined, status } })
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
          (editing.current_version?.content.instructions ?? "")
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
      instructions: data.current_version?.content.instructions ?? "",
    });
  };

  return (
    <>
      <PageHeader
        title="我的 Skills"
        description="这些内容仅对你可见，每次内容修改都会保留一个新版本。"
        actions={
          <Button
            type="primary"
            icon={<PlusOutlined />}
            onClick={() => {
              setEditing(null);
              form.resetFields();
              setParams({ create: "1" });
            }}
          >
            创建 Skill
          </Button>
        }
      />
      <div className="toolbar">
        <Input.Search
          allowClear
          placeholder="搜索名称或描述"
          onSearch={setSearch}
          className="search-input"
        />
        <Select
          allowClear
          placeholder="全部状态"
          value={status}
          onChange={setStatus}
          options={["draft", "published", "disabled", "archived"].map((value) => ({
            value,
            label: value,
          }))}
        />
      </div>
      <Table
        rowKey="id"
        loading={skills.isLoading}
        dataSource={skills.data?.items}
        locale={{ emptyText: <Empty description="还没有 Skill" /> }}
        pagination={{
          total: skills.data?.total,
          pageSize: skills.data?.page_size ?? 20,
        }}
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
            render: (tags: string[]) => tags.map((tag) => <Tag key={tag}>{tag}</Tag>),
          },
          {
            title: "状态",
            dataIndex: "status",
            render: (value: string) => <Tag color={value === "published" ? "green" : "default"}>{value}</Tag>,
          },
          {
            title: "操作",
            render: (_: unknown, record: Skill) => (
              <Space>
                <Button
                  type="text"
                  aria-label="查看"
                  icon={<EyeOutlined />}
                  onClick={async () => {
                    const { data } = await api.get<Skill>(`/skills/${record.id}`);
                    setDetail(data);
                  }}
                />
                <Button
                  type="text"
                  aria-label="编辑"
                  icon={<EditOutlined />}
                  onClick={() => openEdit(record)}
                />
                <Button
                  type="text"
                  aria-label="复制"
                  icon={<CopyOutlined />}
                  onClick={() => copy(record)}
                />
                <Popconfirm
                  title="删除这个 Skill？"
                  description="内容会被软删除，历史审计记录仍会保留。"
                  onConfirm={() => remove(record)}
                >
                  <Button danger type="text" icon={<DeleteOutlined />} />
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
                options={["draft", "published", "disabled", "archived"].map((value) => ({
                  value,
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
              <Tag>{detail.status}</Tag>
              <Tag>{detail.category || "未分类"}</Tag>
            </div>
            <Typography.Paragraph>{detail.description}</Typography.Paragraph>
            <div>
              <Typography.Title level={5}>当前指令</Typography.Title>
              <pre className="content-preview">
                {detail.current_version?.content.instructions || "暂无内容"}
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
                  { title: "状态", dataIndex: "status" },
                  { title: "变更", dataIndex: "change_log" },
                ]}
              />
            </div>
          </Space>
        )}
      </Drawer>
    </>
  );
}
