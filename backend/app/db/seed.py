from collections.abc import Iterable
from typing import TypedDict

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.security import hash_password
from app.db.session import SessionLocal
from app.models import AuditLog, Group, GroupMember, GroupSkillGrant, Skill, SkillVersion, User
from app.services.templates import ensure_default_template

DEFAULT_CONTENT = {
    "system_prompt": "",
    "instructions": "",
    "examples": [],
    "tools": [],
    "parameters": {},
}


class SkillSeed(TypedDict):
    name: str
    slug: str
    instructions: str
    category: str
    tags: list[str]


def _user(
    session: Session,
    username: str,
    email: str,
    display_name: str,
    password: str,
    *,
    is_admin: bool = False,
) -> User:
    user = session.scalar(select(User).where(User.username == username))
    if user is None:
        user = User(
            username=username,
            email=email,
            display_name=display_name,
            password_hash=hash_password(password),
            is_global_admin=is_admin,
            status="active",
        )
        session.add(user)
        session.flush()
    else:
        user.email = email
        user.display_name = display_name
        user.is_global_admin = is_admin
        user.status = "active"
    return user


def _group(
    session: Session,
    name: str,
    owner: User,
    *,
    group_type: str = "personal",
) -> Group:
    group = session.scalar(select(Group).where(Group.name == name, Group.owner_id == owner.id))
    if group is None:
        group = Group(
            name=name,
            description=f"{name} 的示例协作空间",
            group_type=group_type,
            owner_id=owner.id,
            created_by=owner.id,
            join_policy="invite_only",
            status="active",
        )
        session.add(group)
        session.flush()
    member = session.scalar(
        select(GroupMember).where(
            GroupMember.group_id == group.id,
            GroupMember.user_id == owner.id,
        )
    )
    if member is None:
        session.add(
            GroupMember(
                group_id=group.id,
                user_id=owner.id,
                role="owner",
                status="active",
            )
        )
    return group


def _member(session: Session, group: Group, user: User, role: str = "member") -> None:
    member = session.scalar(
        select(GroupMember).where(
            GroupMember.group_id == group.id,
            GroupMember.user_id == user.id,
        )
    )
    if member is None:
        session.add(
            GroupMember(
                group_id=group.id,
                user_id=user.id,
                role=role,
                status="active",
                invited_by=group.owner_id,
            )
        )
    else:
        member.role = role
        member.status = "active"


def _skill(
    session: Session,
    *,
    name: str,
    slug: str,
    creator: User,
    skill_type: str,
    instructions: str,
    category: str,
    tags: list[str],
) -> Skill:
    owner_id = creator.id if skill_type == "private" else None
    skill = session.scalar(
        select(Skill).where(
            Skill.slug == slug,
            Skill.owner_user_id == owner_id,
            Skill.skill_type == skill_type,
        )
    )
    created_skill = skill is None
    if skill is None:
        skill = Skill(
            name=name,
            slug=slug,
            description=f"{name} 示例 Skill",
            skill_type=skill_type,
            owner_user_id=owner_id,
            category=category,
            tags=tags,
            status="published",
            sync_revision=1,
            created_by=creator.id,
        )
        session.add(skill)
        session.flush()
    version = session.scalar(
        select(SkillVersion).where(
            SkillVersion.skill_id == skill.id,
            SkillVersion.version == "1.0.0",
        )
    )
    if version is None:
        revision = 1 if created_skill else max(1, skill.sync_revision + 1)
        version = SkillVersion(
            skill_id=skill.id,
            version="1.0.0",
            revision=revision,
            content={**DEFAULT_CONTENT, "instructions": instructions},
            manifest={"name": slug, "schema_version": 1},
            package_manifest_hash=None,
            package_size_bytes=None,
            dependency_config={},
            change_log="Initial example version",
            status="published",
            created_by=creator.id,
        )
        session.add(version)
        session.flush()
        skill.sync_revision = revision
    skill.current_version_id = version.id
    skill.current_package_hash = version.package_manifest_hash
    return skill


def _audit_once(
    session: Session,
    actor: User,
    action: str,
    resource_type: str,
    resource_id: str,
) -> None:
    exists = session.scalar(
        select(AuditLog.id).where(
            AuditLog.action == action,
            AuditLog.resource_type == resource_type,
            AuditLog.resource_id == resource_id,
        )
    )
    if exists is None:
        session.add(
            AuditLog(
                actor_user_id=actor.id,
                action=action,
                resource_type=resource_type,
                resource_id=resource_id,
                after_data={"seeded": True},
                result="success",
            )
        )


def seed_database(session: Session) -> None:
    admin = _user(
        session,
        "admin",
        "admin@skillhive.local",
        "平台管理员",
        "Admin123!",
        is_admin=True,
    )
    howie = _user(
        session,
        "howie",
        "howie@skillhive.local",
        "Howie",
        "User123!",
    )
    mei = _user(session, "mei", "mei@skillhive.local", "Mei", "User123!")
    for user in (admin, howie, mei):
        ensure_default_template(session, user)

    product_group = _group(session, "产品研发组", howie)
    platform_group = _group(session, "SkillHive 平台组", admin, group_type="platform")
    _member(session, product_group, mei)
    _member(session, product_group, admin, role="admin")
    _member(session, platform_group, howie)

    globals_: list[Skill] = []
    global_seeds: tuple[SkillSeed, ...] = (
        {
            "name": "需求澄清助手",
            "slug": "requirement-clarifier",
            "instructions": "将模糊需求整理为目标、范围、约束和验收标准。",
            "category": "产品",
            "tags": ["需求", "协作"],
        },
        {
            "name": "代码评审助手",
            "slug": "code-reviewer",
            "instructions": "从正确性、安全性、可维护性和测试覆盖率评审代码。",
            "category": "工程",
            "tags": ["代码", "质量"],
        },
        {
            "name": "会议纪要整理",
            "slug": "meeting-notes",
            "instructions": "提取决策、行动项、责任人和截止时间。",
            "category": "效率",
            "tags": ["会议", "总结"],
        },
    )
    for args in global_seeds:
        globals_.append(
            _skill(
                session,
                creator=admin,
                skill_type="global",
                **args,
            )
        )

    private_seeds: tuple[SkillSeed, ...] = (
        {
            "name": "我的周报",
            "slug": "my-weekly-report",
            "instructions": "根据工作记录生成简洁周报。",
            "category": "个人效率",
            "tags": ["周报"],
        },
        {
            "name": "研究摘要",
            "slug": "research-summary",
            "instructions": "归纳研究材料的背景、方法、结论和局限。",
            "category": "研究",
            "tags": ["研究", "摘要"],
        },
    )
    for args in private_seeds:
        _skill(session, creator=howie, skill_type="private", **args)

    grant = session.scalar(
        select(GroupSkillGrant).where(
            GroupSkillGrant.group_id == product_group.id,
            GroupSkillGrant.skill_id == globals_[0].id,
        )
    )
    if grant is None:
        grant = GroupSkillGrant(
            group_id=product_group.id,
            skill_id=globals_[0].id,
            version_policy="latest",
            status="active",
            granted_by=admin.id,
        )
        session.add(grant)
    session.flush()

    resources: Iterable[tuple[User, str, str, str]] = (
        (admin, "user.seeded", "user", admin.id),
        (howie, "group.created", "group", product_group.id),
        (admin, "group.created", "group", platform_group.id),
        (admin, "global_skill.published", "skill", globals_[0].id),
        (admin, "group_skill.enabled", "group_skill_grant", grant.id),
    )
    for entry in resources:
        _audit_once(session, *entry)
    session.commit()


def main() -> None:
    with SessionLocal() as session:
        seed_database(session)
    print("SkillHive development data is ready.")
    print("WARNING: Change the default development passwords before production use.")
    print("Admin: admin / Admin123!")
    print("User:  howie / User123!")


if __name__ == "__main__":
    main()
