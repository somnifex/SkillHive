export interface User {
  id: string;
  username: string;
  display_name: string;
  email: string;
  avatar_url: string | null;
  status: string;
  is_global_admin: boolean;
  created_at: string;
  updated_at: string;
  last_login_at: string | null;
}

export interface TokenResponse {
  access_token: string;
  token_type: string;
  expires_in: number;
  user: User;
}

export interface Page<T> {
  items: T[];
  page: number;
  page_size: number;
  total: number;
  pages: number;
}

export interface SkillVersion {
  id: string;
  skill_id: string;
  version: string;
  content: {
    system_prompt?: string;
    instructions?: string;
    examples?: unknown[];
    tools?: string[];
    parameters?: Record<string, unknown>;
    skill_markdown?: string;
  };
  manifest: Record<string, unknown>;
  dependency_config: Record<string, unknown>;
  change_log: string;
  status: string;
  created_by: string;
  created_at: string;
}

export interface Skill {
  id: string;
  name: string;
  slug: string;
  description: string;
  skill_type: "private" | "global";
  owner_user_id: string | null;
  category: string;
  tags: string[];
  status: string;
  current_version_id: string | null;
  current_version: SkillVersion | null;
  created_at: string;
  updated_at: string;
}

export interface SkillTemplate {
  id: string;
  name: string;
  slug: string;
  description: string;
  scope_type: "personal" | "group" | "global";
  owner_user_id: string | null;
  group_id: string | null;
  group_name: string | null;
  category: string;
  tags: string[];
  content: SkillVersion["content"];
  manifest: Record<string, unknown>;
  status: string;
  is_default: boolean;
  can_manage: boolean;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface Group {
  id: string;
  name: string;
  description: string;
  group_type: string;
  owner_id: string;
  join_policy: string;
  allow_member_invite: boolean;
  status: string;
  current_user_role: "owner" | "admin" | "member" | null;
  created_at: string;
  updated_at: string;
}

export interface Member {
  id: string;
  group_id: string;
  user_id: string;
  role: "owner" | "admin" | "member";
  status: string;
  joined_at: string;
  user: Pick<User, "id" | "username" | "display_name" | "avatar_url"> | null;
}

export interface Grant {
  id: string;
  group_id: string;
  skill_id: string;
  version_policy: "latest" | "locked";
  locked_version_id: string | null;
  status: string;
  granted_by: string;
  granted_at: string;
  skill: Skill | null;
  effective_version: SkillVersion | null;
}

export interface AuditLog {
  id: string;
  actor_user_id: string | null;
  action: string;
  resource_type: string;
  resource_id: string | null;
  result: string;
  created_at: string;
}
