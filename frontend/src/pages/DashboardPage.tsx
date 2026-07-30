import {
  ArrowRight,
  ArrowUpRight,
  Blocks,
  BookOpen,
  Plus,
  Radio,
  Users,
} from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Button, Empty, Skeleton, Tag, Typography } from "antd";
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
        <div className="hero-copy">
          <div className="hero-status">
            <Radio size={15} aria-hidden="true" />
            <span>工作空间已同步</span>
            <small>SYS.26</small>
          </div>
          <h1>
            让能力
            <span>持续发生。</span>
          </h1>
          <p>
            将个人方法、团队经验与经过审核的全局能力，编织成一套可生长、可追溯、可协作的知识系统。
          </p>
          <div className="hero-actions">
            <Button
              type="primary"
              size="large"
              icon={<Plus size={18} aria-hidden="true" />}
              onClick={() => navigate("/skills?create=1")}
            >
              创建 Skill
            </Button>
            <Button
              size="large"
              icon={<ArrowRight size={18} aria-hidden="true" />}
              iconPlacement="end"
              onClick={() => navigate("/groups")}
            >
              进入协作空间
            </Button>
          </div>
          <div className="hero-footnote">
            <span>PRIVATE</span>
            <span>COLLABORATIVE</span>
            <span>VERSIONED</span>
          </div>
        </div>

        <figure className="hero-visual">
          <img
            src="/art/skillhive-orbit.png"
            alt="由钴蓝玻璃、金属节点与橙色轨道构成的抽象能力网络"
            width={1672}
            height={941}
            fetchPriority="high"
          />
          <div className="hero-scanline" aria-hidden="true" />
          <figcaption>
            <span>LIVE KNOWLEDGE TOPOLOGY</span>
            <strong>SH / 001</strong>
          </figcaption>
          <div className="orbit-note orbit-note-a" aria-hidden="true">
            01 / CAPTURE
          </div>
          <div className="orbit-note orbit-note-b" aria-hidden="true">
            02 / CONNECT
          </div>
        </figure>
      </section>

      <div className="kinetic-strip" aria-hidden="true">
        <div>
          SKILLS AS SYSTEMS <span>•</span> KNOWLEDGE IN MOTION <span>•</span> BUILT FOR
          TEAMS <span>•</span> SKILLS AS SYSTEMS <span>•</span> KNOWLEDGE IN MOTION
        </div>
      </div>

      <section className="stat-grid" aria-label="工作空间数据">
        <button className="stat stat-featured" onClick={() => navigate("/skills")}>
          <span className="stat-index">01</span>
          <BookOpen size={24} strokeWidth={1.5} aria-hidden="true" />
          <div>
            <span className="stat-label">我的 Skills</span>
            <strong>{skills.data?.total ?? "—"}</strong>
            <p>正在沉淀的私人工作方法</p>
          </div>
          <ArrowUpRight className="stat-arrow" size={20} aria-hidden="true" />
        </button>
        <button className="stat" onClick={() => navigate("/groups")}>
          <span className="stat-index">02</span>
          <Users size={24} strokeWidth={1.5} aria-hidden="true" />
          <div>
            <span className="stat-label">协作群组</span>
            <strong>{groups.data?.total ?? "—"}</strong>
            <p>共享、审核与演化团队能力</p>
          </div>
          <ArrowUpRight className="stat-arrow" size={20} aria-hidden="true" />
        </button>
        <button className="stat" onClick={() => navigate("/group-skills")}>
          <span className="stat-index">03</span>
          <Blocks size={24} strokeWidth={1.5} aria-hidden="true" />
          <div>
            <span className="stat-label">群组 Skills</span>
            <strong className="stat-word">浏览</strong>
            <p>发现当前可用的全局能力</p>
          </div>
          <ArrowUpRight className="stat-arrow" size={20} aria-hidden="true" />
        </button>
      </section>

      <section className="dashboard-section">
        <div className="section-title">
          <div>
            <span className="section-kicker">RECENT SIGNALS / 03</span>
            <h2>最近的 Skills</h2>
          </div>
          <Button
            type="text"
            icon={<ArrowRight size={17} aria-hidden="true" />}
            iconPlacement="end"
            onClick={() => navigate("/skills")}
          >
            查看全部
          </Button>
        </div>
        {skills.isLoading ? (
          <Skeleton active />
        ) : skills.data?.items.length ? (
          <div className="compact-list">
            {skills.data.items.map((skill, index) => (
              <button
                key={skill.id}
                className="compact-row"
                onClick={() => navigate(`/skills?skill=${skill.id}`)}
              >
                <span className="compact-index">0{index + 1}</span>
                <div className="compact-copy">
                  <Typography.Text strong>{skill.name}</Typography.Text>
                  <Typography.Paragraph type="secondary">
                    {skill.description || "暂无描述"}
                  </Typography.Paragraph>
                </div>
                <Tag color={skill.status === "published" ? "geekblue" : undefined}>
                  {skill.status}
                </Tag>
                <ArrowUpRight size={18} aria-hidden="true" />
              </button>
            ))}
          </div>
        ) : (
          <Empty
            image={<BookOpen className="empty-icon" aria-hidden="true" />}
            description="还没有私人 Skill"
          />
        )}
      </section>
    </>
  );
}
