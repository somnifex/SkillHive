import { Blocks } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Empty, Select, Table, Tag, Typography } from "antd";
import { useState } from "react";

import { api } from "../api/client";
import { PageHeader } from "../components/PageHeader";
import type { Grant, Group, Page } from "../types";

export function GroupSkillsPage() {
  const [groupId, setGroupId] = useState<string>();
  const groups = useQuery({
    queryKey: ["groups", "skill-selector"],
    queryFn: () => api.get<Page<Group>>("/groups?page_size=100").then((r) => r.data),
  });
  const grants = useQuery({
    queryKey: ["group-grants", groupId],
    queryFn: () => api.get<Grant[]>(`/groups/${groupId}/skills`).then((r) => r.data),
    enabled: Boolean(groupId),
  });

  return (
    <>
      <PageHeader
        title="群组 Skills"
        description="查看你所在群组当前已启用、可实际使用的全局 Skills。"
      />
      <Select
        className="group-selector"
        placeholder="选择群组"
        value={groupId}
        onChange={setGroupId}
        options={groups.data?.items.map((group) => ({
          value: group.id,
          label: group.name,
        }))}
      />
      {groupId ? (
        <Table
          rowKey="id"
          loading={grants.isLoading}
          dataSource={grants.data}
          locale={{
            emptyText: (
              <Empty
                image={<Blocks className="empty-icon" aria-hidden="true" />}
                description="该群组没有可用 Skill"
              />
            ),
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
              title: "分类",
              render: (_: unknown, grant: Grant) => grant.skill?.category,
            },
            {
              title: "版本",
              render: (_: unknown, grant: Grant) => (
                <Tag>{grant.effective_version?.version}</Tag>
              ),
            },
            { title: "策略", dataIndex: "version_policy" },
          ]}
        />
      ) : (
        <Empty
          className="page-empty"
          image={<Blocks className="empty-icon" aria-hidden="true" />}
          description="先选择一个群组"
        />
      )}
    </>
  );
}
