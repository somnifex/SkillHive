import {
  ArrowRightOutlined,
  BookOutlined,
  PlusOutlined,
  TeamOutlined,
} from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { Button, Empty, Skeleton, Space, Tag, Typography } from "antd";
import { useNavigate } from "react-router-dom";

import { api } from "../api/client";
import type { Group, Page, Skill } from "../types";

export function DashboardPage() {
  const navigate = useNavigate();
  const skills = useQuery({
    queryKey: ["skills", "dashboard"],
    queryFn: () => api.get<Page<Skill>>("/skills?page_size=3").then((r) => r.data),
  });
  const groups = useQuery({
    queryKey: ["groups", "dashboard"],
    queryFn: () => api.get<Page<Group>>("/groups?page_size=3").then((r) => r.data),
  });

  return (
    <>
      <section className="hero dashboard-hero">
        <Tag color="success">工作空间已就绪</Tag>
        <Typography.Title>让团队知识成为可管理的能力</Typography.Title>
        <Typography.Paragraph>
          在清晰的权限边界内沉淀私人 Skill、协作群组与经过审核的全局能力。
        </Typography.Paragraph>
        <Space wrap>
          <Button
            type="primary"
            size="large"
            icon={<PlusOutlined />}
            onClick={() => navigate("/skills?create=1")}
          >
            创建 Skill
          </Button>
          <Button size="large" onClick={() => navigate("/groups")}>
            查看群组
          </Button>
        </Space>
      </section>
      <section className="stat-grid">
        <button className="stat" onClick={() => navigate("/skills")}>
          <BookOutlined />
          <Typography.Text type="secondary">我的 Skills</Typography.Text>
          <Typography.Title>{skills.data?.total ?? "—"}</Typography.Title>
          <Typography.Text>沉淀个人工作方法</Typography.Text>
        </button>
        <button className="stat" onClick={() => navigate("/groups")}>
          <TeamOutlined />
          <Typography.Text type="secondary">加入的群组</Typography.Text>
          <Typography.Title>{groups.data?.total ?? "—"}</Typography.Title>
          <Typography.Text>和团队共享经过审核的能力</Typography.Text>
        </button>
        <button className="stat" onClick={() => navigate("/group-skills")}>
          <ArrowRightOutlined />
          <Typography.Text type="secondary">群组 Skills</Typography.Text>
          <Typography.Title>浏览</Typography.Title>
          <Typography.Text>查看当前可用的全局 Skills</Typography.Text>
        </button>
      </section>
      <section className="dashboard-section">
        <div className="section-title">
          <Typography.Title level={4}>最近的 Skills</Typography.Title>
          <Button type="link" onClick={() => navigate("/skills")}>
            查看全部
          </Button>
        </div>
        {skills.isLoading ? (
          <Skeleton active />
        ) : skills.data?.items.length ? (
          <div className="compact-list">
            {skills.data.items.map((skill) => (
              <button
                key={skill.id}
                className="compact-row"
                onClick={() => navigate(`/skills?skill=${skill.id}`)}
              >
                <div>
                  <Typography.Text strong>{skill.name}</Typography.Text>
                  <Typography.Paragraph type="secondary">
                    {skill.description || "暂无描述"}
                  </Typography.Paragraph>
                </div>
                <Tag>{skill.status}</Tag>
              </button>
            ))}
          </div>
        ) : (
          <Empty description="还没有私人 Skill" />
        )}
      </section>
    </>
  );
}
