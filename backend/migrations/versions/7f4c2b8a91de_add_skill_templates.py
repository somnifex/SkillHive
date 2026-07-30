"""add skill templates

Revision ID: 7f4c2b8a91de
Revises: 2e26577093dc
Create Date: 2026-07-30 20:30:00
"""

from collections.abc import Sequence
from datetime import UTC, datetime
from uuid import uuid4

import sqlalchemy as sa
from alembic import op

revision: str = "7f4c2b8a91de"
down_revision: str | None = "2e26577093dc"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.create_table(
        "skill_templates",
        sa.Column("name", sa.String(length=120), nullable=False),
        sa.Column("slug", sa.String(length=140), nullable=False),
        sa.Column("description", sa.Text(), nullable=False),
        sa.Column("scope_type", sa.String(length=20), nullable=False),
        sa.Column("owner_user_id", sa.String(length=36), nullable=True),
        sa.Column("group_id", sa.String(length=36), nullable=True),
        sa.Column("category", sa.String(length=80), nullable=False),
        sa.Column("tags", sa.JSON(), nullable=False),
        sa.Column("content", sa.JSON(), nullable=False),
        sa.Column("manifest", sa.JSON(), nullable=False),
        sa.Column("status", sa.String(length=20), nullable=False),
        sa.Column("is_default", sa.Boolean(), nullable=False),
        sa.Column("created_by", sa.String(length=36), nullable=False),
        sa.Column("deleted_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("id", sa.String(length=36), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(["created_by"], ["users.id"]),
        sa.ForeignKeyConstraint(["group_id"], ["groups.id"], ondelete="CASCADE"),
        sa.ForeignKeyConstraint(["owner_user_id"], ["users.id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint("group_id", "slug", name="uq_group_template_slug"),
        sa.UniqueConstraint("owner_user_id", "slug", name="uq_owner_template_slug"),
    )
    op.create_index("ix_skill_templates_category", "skill_templates", ["category"])
    op.create_index("ix_skill_templates_group_id", "skill_templates", ["group_id"])
    op.create_index("ix_skill_templates_is_default", "skill_templates", ["is_default"])
    op.create_index("ix_skill_templates_name", "skill_templates", ["name"])
    op.create_index("ix_skill_templates_owner_user_id", "skill_templates", ["owner_user_id"])
    op.create_index("ix_skill_templates_scope_type", "skill_templates", ["scope_type"])
    op.create_index("ix_skill_templates_slug", "skill_templates", ["slug"])
    op.create_index("ix_skill_templates_status", "skill_templates", ["status"])
    op.create_index(
        "ix_template_scope_status",
        "skill_templates",
        ["scope_type", "status"],
    )

    users = sa.table("users", sa.column("id", sa.String(length=36)))
    templates = sa.table(
        "skill_templates",
        sa.column("id", sa.String(length=36)),
        sa.column("name", sa.String(length=120)),
        sa.column("slug", sa.String(length=140)),
        sa.column("description", sa.Text()),
        sa.column("scope_type", sa.String(length=20)),
        sa.column("owner_user_id", sa.String(length=36)),
        sa.column("group_id", sa.String(length=36)),
        sa.column("category", sa.String(length=80)),
        sa.column("tags", sa.JSON()),
        sa.column("content", sa.JSON()),
        sa.column("manifest", sa.JSON()),
        sa.column("status", sa.String(length=20)),
        sa.column("is_default", sa.Boolean()),
        sa.column("created_by", sa.String(length=36)),
        sa.column("deleted_at", sa.DateTime(timezone=True)),
        sa.column("created_at", sa.DateTime(timezone=True)),
        sa.column("updated_at", sa.DateTime(timezone=True)),
    )
    now = datetime.now(UTC)
    rows = []
    for user_id in op.get_bind().execute(sa.select(users.c.id)).scalars():
        rows.append(
            {
                "id": str(uuid4()),
                "name": "OpenAI 推荐 Skill 模板",
                "slug": "openai-recommended-skill",
                "description": "以 SKILL.md 为入口，包含 name、description 和清晰工作流指令。",
                "scope_type": "personal",
                "owner_user_id": user_id,
                "group_id": None,
                "category": "通用",
                "tags": ["OpenAI", "SKILL.md"],
                "content": {
                    "system_prompt": "",
                    "instructions": (
                        "# 工作流\n\n"
                        "1. 明确用户提供的输入、目标和约束；缺少关键条件时先提问。\n"
                        "2. 按可验证的步骤完成任务，不臆造事实或工具结果。\n"
                        "3. 使用用户要求的格式输出；未指定时保持简洁、可执行。\n"
                        "4. 在交付前检查结果是否满足目标，并说明任何限制。\n\n"
                        "# 边界\n\n"
                        "- 只处理此 Skill 描述覆盖的任务。\n"
                        "- 涉及不可逆操作或外部影响时，先获得明确授权。\n"
                    ),
                    "examples": [],
                    "tools": [],
                    "parameters": {},
                    "skill_markdown": (
                        "---\nname: {{slug}}\ndescription: {{description}}\n---\n\n"
                        "{{instructions}}\n"
                    ),
                },
                "manifest": {
                    "format": "openai-skill",
                    "entrypoint": "SKILL.md",
                    "schema_version": 1,
                    "required_frontmatter": ["name", "description"],
                    "optional_directories": ["references", "assets", "scripts"],
                },
                "status": "published",
                "is_default": True,
                "created_by": user_id,
                "deleted_at": None,
                "created_at": now,
                "updated_at": now,
            }
        )
    if rows:
        op.bulk_insert(templates, rows)


def downgrade() -> None:
    op.drop_index("ix_template_scope_status", table_name="skill_templates")
    op.drop_index("ix_skill_templates_status", table_name="skill_templates")
    op.drop_index("ix_skill_templates_slug", table_name="skill_templates")
    op.drop_index("ix_skill_templates_scope_type", table_name="skill_templates")
    op.drop_index("ix_skill_templates_owner_user_id", table_name="skill_templates")
    op.drop_index("ix_skill_templates_name", table_name="skill_templates")
    op.drop_index("ix_skill_templates_is_default", table_name="skill_templates")
    op.drop_index("ix_skill_templates_group_id", table_name="skill_templates")
    op.drop_index("ix_skill_templates_category", table_name="skill_templates")
    op.drop_table("skill_templates")
