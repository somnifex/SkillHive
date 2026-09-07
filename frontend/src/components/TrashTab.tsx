import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Search } from "lucide-react";
import { App, Button, Empty, Input, Popconfirm, Table, Typography } from "antd";
import { useState } from "react";

import { api, errorMessage } from "../api/client";
import type { Page, Skill } from "../types";

/**
 * Recycle-bin view for the current user's soft-deleted skills: restore back
 * to the active list (as draft) or purge permanently (irreversible).
 */
export function TrashTab() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");

  const trash = useQuery({
    queryKey: ["skills-trash", search],
    queryFn: () =>
      api
        .get<Page<Skill>>("/skills/trash", { params: { query: search || undefined } })
        .then((r) => r.data),
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: ["skills"] });
    void queryClient.invalidateQueries({ queryKey: ["skills-trash"] });
  };

  const restore = useMutation({
    mutationFn: (skillId: string) => api.post<Skill>(`/skills/${skillId}/restore`),
    onSuccess: () => {
      message.success("已恢复到「我的 Skills」（状态：草稿）");
      invalidate();
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const purge = useMutation({
    mutationFn: (skillId: string) => api.delete(`/skills/${skillId}/purge`),
    onSuccess: () => {
      message.success("已彻底删除");
      invalidate();
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  return (
    <>
      <div className="toolbar">
        <Input
          allowClear
          placeholder="搜索回收站"
          prefix={<Search size={16} strokeWidth={1.7} aria-hidden="true" />}
          onChange={(event) => setSearch(event.target.value)}
          className="search-input"
        />
      </div>
      <Typography.Paragraph type="secondary">
        删除的 Skill 会先进入回收站；恢复后状态为草稿。彻底删除会永久移除全部版本，不可撤销。
      </Typography.Paragraph>
      <Table
        rowKey="id"
        loading={trash.isLoading}
        dataSource={trash.data?.items}
        locale={{ emptyText: <Empty description="回收站是空的" /> }}
        pagination={{
          current: trash.data?.page ?? 1,
          pageSize: trash.data?.page_size ?? 50,
          total: trash.data?.total,
          showSizeChanger: false,
        }}
        columns={[
          {
            title: "名称",
            dataIndex: "name",
            render: (value: string) => <Typography.Text strong>{value}</Typography.Text>,
          },
          { title: "Slug", dataIndex: "slug" },
          { title: "分类", dataIndex: "category", render: (v: string) => v || "—" },
          {
            title: "删除时间",
            dataIndex: "deleted_at",
            render: (value: string | null) => (value ? new Date(value).toLocaleString() : "—"),
          },
          {
            title: "操作",
            width: 190,
            render: (_: unknown, record: Skill) => (
              <>
                <Popconfirm
                  title="恢复这个 Skill？"
                  description="恢复后进入「我的 Skills」，状态为草稿。"
                  onConfirm={() => restore.mutate(record.id)}
                >
                  <Button size="small" type="text" aria-label="恢复">
                    恢复
                  </Button>
                </Popconfirm>
                <Popconfirm
                  title="彻底删除？"
                  description="将永久删除该 Skill 的全部版本，不可恢复。"
                  okText="彻底删除"
                  okButtonProps={{ danger: true }}
                  onConfirm={() => purge.mutate(record.id)}
                >
                  <Button danger size="small" type="text" aria-label="彻底删除">
                    彻底删除
                  </Button>
                </Popconfirm>
              </>
            ),
          },
        ]}
      />
    </>
  );
}
