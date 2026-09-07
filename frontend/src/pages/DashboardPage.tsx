import { ArrowRight, Blocks, BookOpen, FileText, Plus, Users } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Button, Empty, Tag } from "antd";
import { useNavigate } from "react-router-dom";

import { api } from "../api/client";
import { useAuthStore } from "../stores/auth";
import type { Group, Page, Skill } from "../types";

function roleLabel(role: Group["current_user_role"]): string {
  switch (role) {
    case "owner":
      return "群主";
    case "admin":
      return "管理员";
    case "member":
      return "成员";
    default:
      return "—";
  }
}

export function DashboardPage() {
  const navigate = useNavigate();
  const displayName = useAuthStore((state) => state.user?.display_name);
  const skills = useQuery({
    queryKey: ["skills", "dashboard"],
    queryFn: () => api.get<Page<Skill>>("/skills?page_size=5").then((r) => r.data),
  });
  const groups = useQuery({
    queryKey: ["groups", "dashboard"],
    queryFn: () => api.get<Page<Group>>("/groups?page_size=100").then((r) => r.data),
  });

  const today = new Date();
  const dateLabel = `${today.getFullYear()} 年 ${today.getMonth() + 1} 月 ${today.getDate()} 日`;
  const weekday = ["星期日", "星期一", "星期二", "星期三", "星期四", "星期五", "星期六"][
    today.getDay()
  ];

  const recentSkills = skills.data?.items ?? [];
  const managedGroups = (groups.data?.items ?? []).filter((group) =>
    ["owner", "admin"].includes(group.current_user_role ?? ""),
  );

  return (
    <>
      <div className="welcome-band">
        <div className="welcome-copy">
          <h1>你好，{displayName || "用户"}</h1>
          <p className="welcome-date">
            {dateLabel} · {weekday}
          </p>
        </div>
        <div className="welcome-actions">
          <Button
            type="primary"
            icon={<Plus size={16} aria-hidden="true" />}
            onClick={() => navigate("/skills?create=1")}
          >
            创建 Skill
          </Button>
          <Button
            icon={<FileText size={16} aria-hidden="true" />}
            onClick={() => navigate("/templates")}
          >
            浏览模板
          </Button>
        </div>
      </div>

      <section className="stat-grid" aria-label="工作空间数据">
        <button className="stat stat-featured" onClick={() => navigate("/skills")}>
          <span className="stat-icon">
            <BookOpen size={20} aria-hidden="true" />
          </span>
          <span className="stat-body">
            <span className="stat-label">我的 Skills</span>
            <span className="stat-value">{skills.data?.total ?? "—"}</span>
            <span className="stat-desc">私人沉淀的工作方法</span>
          </span>
        </button>
        <button className="stat" onClick={() => navigate("/groups")}>
          <span className="stat-icon">
            <Users size={20} aria-hidden="true" />
          </span>
          <span className="stat-body">
            <span className="stat-label">协作群组</span>
            <span className="stat-value">{groups.data?.total ?? "—"}</span>
            <span className="stat-desc">共享、审核与演化能力</span>
          </span>
        </button>
        <button className="stat" onClick={() => navigate("/group-skills")}>
          <span className="stat-icon stat-icon-accent">
            <Blocks size={20} aria-hidden="true" />
          </span>
          <span className="stat-body">
            <span className="stat-label">我管理的群组</span>
            <span className="stat-value">{managedGroups.length}</span>
            <span className="stat-desc">查看群组已启用的 Skills</span>
          </span>
        </button>
        <button className="stat" onClick={() => navigate("/templates")}>
          <span className="stat-icon stat-icon-green">
            <FileText size={20} aria-hidden="true" />
          </span>
          <span className="stat-body">
            <span className="stat-label">模板库</span>
            <span className="stat-value">快速开始</span>
            <span className="stat-desc">从模板一键创建 Skill</span>
          </span>
        </button>
      </section>

      <div className="home-columns">
        <section className="home-card" aria-label="最近的 Skills">
          <div className="home-card-head">
            <h2>最近的 Skills</h2>
            <Button type="text" onClick={() => navigate("/skills")}>
              查看全部
            </Button>
          </div>
          <div className="compact-list">
            {recentSkills.length ? (
              recentSkills.map((skill) => (
                <button
                  key={skill.id}
                  className="compact-row"
                  onClick={() => navigate(`/skills?skill=${skill.id}`)}
                >
                  <span className="compact-main">
                    <span className="compact-title">{skill.name}</span>
                    <span className="compact-sub">
                      {skill.description || skill.slug}
                    </span>
                  </span>
                  <Tag color={skill.status === "published" ? "blue" : "default"}>
                    {skill.status === "published" ? "已发布" : "草稿"}
                  </Tag>
                  <ArrowRight className="compact-arrow" size={16} aria-hidden="true" />
                </button>
              ))
            ) : (
              <div className="compact-pad">
                <Empty
                  image={<BookOpen className="empty-icon" aria-hidden="true" />}
                  description="还没有私人 Skill，去「我的 Skills」创建第一个吧"
                />
              </div>
            )}
          </div>
        </section>

        <section className="home-card" aria-label="协作群组">
          <div className="home-card-head">
            <h2>协作群组</h2>
            <Button type="text" onClick={() => navigate("/groups")}>
              全部群组
            </Button>
          </div>
          <div className="compact-list">
            {(groups.data?.items ?? []).slice(0, 4).map((group) => (
              <button
                key={group.id}
                className="compact-row"
                onClick={() => navigate(`/groups/${group.id}`)}
              >
                <span className="compact-main">
                  <span className="compact-title">{group.name}</span>
                  <span className="compact-sub">
                    我的角色：{roleLabel(group.current_user_role)}
                  </span>
                </span>
                <ArrowRight className="compact-arrow" size={16} aria-hidden="true" />
              </button>
            ))}
            {groups.data && groups.data.items.length === 0 && (
              <div className="compact-pad">
                <Empty
                  image={<Users className="empty-icon" aria-hidden="true" />}
                  description="还没有加入任何群组"
                />
              </div>
            )}
          </div>
        </section>
      </div>
    </>
  );
}
