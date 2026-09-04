import json
from math import ceil

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.models import Group, GroupMember, SkillTemplate, User
from app.repositories.groups import GroupRepository
from app.repositories.skills import SkillRepository
from app.schemas.common import Page
from app.schemas.skill import SkillRead, SkillVersionRead
from app.schemas.template import (
    TemplateCreate,
    TemplateInstantiate,
    TemplateRead,
    TemplateUpdate,
)
from app.services.audit import write_audit
from app.services.skill_mutations import SkillMutationService

DEFAULT_TEMPLATE_SLUG = "openai-recommended-skill"
DEFAULT_TEMPLATE_INSTRUCTIONS = """# 工作流

1. 明确用户提供的输入、目标和约束；缺少关键条件时先提问。
2. 按可验证的步骤完成任务，不臆造事实或工具结果。
3. 使用用户要求的格式输出；未指定时保持简洁、可执行。
4. 在交付前检查结果是否满足目标，并说明任何限制。

# 边界

- 只处理此 Skill 描述覆盖的任务。
- 涉及不可逆操作或外部影响时，先获得明确授权。
"""
DEFAULT_TEMPLATE_CONTENT = {
    "system_prompt": "",
    "instructions": DEFAULT_TEMPLATE_INSTRUCTIONS,
    "examples": [],
    "tools": [],
    "parameters": {},
    "skill_markdown": """---
name: {{slug}}
description: {{description}}
---

{{instructions}}
""",
}
DEFAULT_TEMPLATE_MANIFEST = {
    "format": "openai-skill",
    "entrypoint": "SKILL.md",
    "schema_version": 1,
    "required_frontmatter": ["name", "description"],
    "optional_directories": ["references", "assets", "scripts"],
}


def ensure_default_template(session: Session, user: User) -> SkillTemplate:
    template = session.scalar(
        select(SkillTemplate).where(
            SkillTemplate.owner_user_id == user.id,
            SkillTemplate.slug == DEFAULT_TEMPLATE_SLUG,
        )
    )
    if template is None:
        template = SkillTemplate(
            name="OpenAI 推荐 Skill 模板",
            slug=DEFAULT_TEMPLATE_SLUG,
            description="以 SKILL.md 为入口，包含 name、description 和清晰工作流指令。",
            scope_type="personal",
            owner_user_id=user.id,
            category="通用",
            tags=["OpenAI", "SKILL.md"],
            content=DEFAULT_TEMPLATE_CONTENT,
            manifest=DEFAULT_TEMPLATE_MANIFEST,
            status="published",
            is_default=True,
            created_by=user.id,
        )
        session.add(template)
        session.flush()
    elif template.deleted_at is not None:
        template.deleted_at = None
        template.status = "published"
        template.is_default = True
    return template


class TemplateService:
    def __init__(self, session: Session, user: User) -> None:
        self.session = session
        self.user = user
        self.groups = GroupRepository(session)
        self.skills = SkillRepository(session)
        self.skill_mutations = SkillMutationService(session, user.id)

    def list_page(
        self,
        *,
        page: int,
        page_size: int,
        query: str | None,
        scope_type: str | None,
        group_id: str | None,
    ) -> Page[TemplateRead]:
        ensure_default_template(self.session, self.user)
        self.session.commit()
        memberships = self._membership_roles()
        templates = list(
            self.session.scalars(
                select(SkillTemplate)
                .where(SkillTemplate.deleted_at.is_(None))
                .order_by(SkillTemplate.is_default.desc(), SkillTemplate.updated_at.desc())
            )
        )
        normalized_query = query.strip().lower() if query else None
        visible: list[SkillTemplate] = []
        for template in templates:
            if not self._can_view(template, memberships):
                continue
            if scope_type and template.scope_type != scope_type:
                continue
            if group_id and template.group_id != group_id:
                continue
            if normalized_query and normalized_query not in (
                f"{template.name} {template.slug} {template.description}".lower()
            ):
                continue
            visible.append(template)
        total = len(visible)
        start = (page - 1) * page_size
        return Page[TemplateRead](
            items=[self._read(item) for item in visible[start : start + page_size]],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def create(self, data: TemplateCreate) -> TemplateRead:
        owner_user_id: str | None = None
        group_id: str | None = None
        if data.scope_type == "personal":
            owner_user_id = self.user.id
        elif data.scope_type == "group":
            group_id = data.group_id
            self._require_group_manager(group_id)
        elif not self.user.is_global_admin:
            raise AppError(
                "PERMISSION_DENIED",
                "Global administrator permission is required.",
                403,
            )
        self._ensure_slug_available(data.scope_type, data.slug, owner_user_id, group_id)
        template = SkillTemplate(
            name=data.name.strip(),
            slug=data.slug,
            description=data.description,
            scope_type=data.scope_type,
            owner_user_id=owner_user_id,
            group_id=group_id,
            category=data.category,
            tags=data.tags,
            content=data.content.model_dump(mode="json"),
            manifest=data.manifest or DEFAULT_TEMPLATE_MANIFEST,
            status=data.status,
            is_default=False,
            created_by=self.user.id,
        )
        self.session.add(template)
        self.session.flush()
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="skill_template.created",
            resource_type="skill_template",
            resource_id=template.id,
            after_data={"name": template.name, "scope_type": template.scope_type},
        )
        self.session.commit()
        return self._read(template)

    def get(self, template_id: str) -> TemplateRead:
        return self._read(self._visible(template_id))

    def update(self, template_id: str, data: TemplateUpdate) -> TemplateRead:
        template = self._managed(template_id)
        if template.is_default and data.status == "disabled":
            raise AppError(
                "DEFAULT_TEMPLATE_REQUIRED",
                "The default template cannot be disabled.",
                409,
            )
        before = {"name": template.name, "status": template.status}
        updates = data.model_dump(exclude_unset=True)
        if "content" in updates and data.content is not None:
            updates["content"] = data.content.model_dump(mode="json")
        for key, value in updates.items():
            if value is not None:
                setattr(template, key, value)
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="skill_template.updated",
            resource_type="skill_template",
            resource_id=template.id,
            before_data=before,
            after_data={"name": template.name, "status": template.status},
        )
        self.session.commit()
        return self._read(template)

    def delete(self, template_id: str) -> None:
        template = self._managed(template_id)
        if template.is_default:
            raise AppError(
                "DEFAULT_TEMPLATE_REQUIRED",
                "The default template cannot be deleted.",
                409,
            )
        template.status = "deleted"
        template.deleted_at = utc_now()
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="skill_template.deleted",
            resource_type="skill_template",
            resource_id=template.id,
            before_data={"name": template.name, "scope_type": template.scope_type},
        )
        self.session.commit()

    def instantiate(self, template_id: str, data: TemplateInstantiate) -> SkillRead:
        template = self._visible(template_id)
        if template.status != "published" and not self._can_manage(template):
            raise AppError("TEMPLATE_UNAVAILABLE", "This template is not published.", 409)
        if self.skills.slug_exists(self.user.id, data.slug):
            raise AppError("SKILL_SLUG_TAKEN", "A skill with this slug already exists.", 409)

        description = data.description if data.description is not None else template.description
        category = data.category if data.category is not None else template.category
        tags = data.tags if data.tags is not None else list(template.tags)
        instructions = (
            data.instructions
            if data.instructions is not None
            else str(template.content.get("instructions", ""))
        )
        if not description.strip():
            raise AppError(
                "SKILL_DESCRIPTION_REQUIRED",
                "A trigger description is required for an OpenAI-format skill.",
                422,
            )
        if not instructions.strip():
            raise AppError(
                "SKILL_INSTRUCTIONS_REQUIRED",
                "Workflow instructions are required.",
                422,
            )

        content = dict(template.content)
        content["instructions"] = instructions
        content["skill_markdown"] = self._skill_markdown(data.slug, description, instructions)
        manifest = {
            **template.manifest,
            "name": data.slug,
            "description": description,
            "source_template_id": template.id,
        }
        skill, version = self.skill_mutations.create_skill(
            name=data.name,
            slug=data.slug,
            description=description,
            skill_type="private",
            owner_user_id=self.user.id,
            category=category,
            tags=tags,
            skill_status="draft",
            version=data.version,
            content=content,
            manifest=manifest,
            dependency_config={},
            change_log=f"Created from template: {template.name}",
            version_status="draft",
            audit_action="skill_template.instantiated",
            audit_after_data={"template_id": template.id},
        )
        self.session.commit()

        result = SkillRead.model_validate(skill)
        result.current_version = SkillVersionRead.model_validate(version)
        return result

    def _visible(self, template_id: str) -> SkillTemplate:
        template = self.session.scalar(
            select(SkillTemplate).where(
                SkillTemplate.id == template_id,
                SkillTemplate.deleted_at.is_(None),
            )
        )
        if template is None or not self._can_view(template):
            raise AppError("TEMPLATE_NOT_FOUND", "Template was not found.", 404)
        return template

    def _managed(self, template_id: str) -> SkillTemplate:
        template = self.session.scalar(
            select(SkillTemplate).where(
                SkillTemplate.id == template_id,
                SkillTemplate.deleted_at.is_(None),
            )
        )
        if template is None:
            raise AppError("TEMPLATE_NOT_FOUND", "Template was not found.", 404)
        if not self._can_manage(template):
            raise AppError("PERMISSION_DENIED", "You cannot manage this template.", 403)
        return template

    def _can_view(
        self,
        template: SkillTemplate,
        memberships: dict[str, str] | None = None,
    ) -> bool:
        if template.scope_type == "personal":
            return template.owner_user_id == self.user.id
        if template.scope_type == "global":
            return template.status == "published" or self.user.is_global_admin
        if self.user.is_global_admin:
            return True
        role = (memberships or self._membership_roles()).get(template.group_id or "")
        return bool(role and (template.status == "published" or role in {"owner", "admin"}))

    def _can_manage(self, template: SkillTemplate) -> bool:
        if template.scope_type == "personal":
            return template.owner_user_id == self.user.id
        if self.user.is_global_admin:
            return True
        if template.scope_type == "global":
            return False
        membership = self.groups.membership(template.group_id or "", self.user.id)
        return bool(membership and membership.role in {"owner", "admin"})

    def _require_group_manager(self, group_id: str | None) -> None:
        group = self.groups.get_active(group_id or "")
        if group is None:
            raise AppError("GROUP_NOT_FOUND", "Group was not found.", 404)
        if self.user.is_global_admin:
            return
        membership = self.groups.membership(group.id, self.user.id)
        if membership is None or membership.role not in {"owner", "admin"}:
            raise AppError("PERMISSION_DENIED", "Group administrator permission is required.", 403)

    def _membership_roles(self) -> dict[str, str]:
        return {
            member.group_id: member.role
            for member in self.session.scalars(
                select(GroupMember).where(
                    GroupMember.user_id == self.user.id,
                    GroupMember.status == "active",
                )
            )
        }

    def _ensure_slug_available(
        self,
        scope_type: str,
        slug: str,
        owner_user_id: str | None,
        group_id: str | None,
    ) -> None:
        statement = select(SkillTemplate.id).where(
            SkillTemplate.scope_type == scope_type,
            SkillTemplate.slug == slug,
        )
        if scope_type == "personal":
            statement = statement.where(SkillTemplate.owner_user_id == owner_user_id)
        elif scope_type == "group":
            statement = statement.where(SkillTemplate.group_id == group_id)
        if self.session.scalar(statement) is not None:
            raise AppError(
                "TEMPLATE_SLUG_TAKEN",
                "A template with this slug already exists in this scope.",
                409,
            )

    def _read(self, template: SkillTemplate) -> TemplateRead:
        result = TemplateRead.model_validate(template)
        result.can_manage = self._can_manage(template)
        if template.group_id:
            group = self.session.get(Group, template.group_id)
            result.group_name = group.name if group else None
        return result

    @staticmethod
    def _skill_markdown(slug: str, description: str, instructions: str) -> str:
        safe_description = " ".join(description.splitlines()).strip()
        yaml_description = json.dumps(safe_description, ensure_ascii=False)
        return (
            f"---\nname: {slug}\ndescription: {yaml_description}\n---\n\n{instructions.strip()}\n"
        )
