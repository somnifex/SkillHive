import { errorMessage } from "./client";
import { isDesktop } from "./server";

export { errorMessage as desktopErrorMessage, isDesktop as hasDesktopCommands };

interface ZipImportDialogResult {
  workspace: { skillId: string; path: string };
  manifestHash: string;
  sourceFileName: string;
  fileCount: number;
}

interface TauriGlobal {
  core?: {
    invoke?: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  };
}

function tauriInvoke(
  command: string,
  args?: Record<string, unknown>,
): Promise<unknown> | null {
  const global = window as unknown as { __TAURI__?: TauriGlobal };
  const invoke = global.__TAURI__?.core?.invoke;
  return invoke ? invoke(command, args) : null;
}

/**
 * Opens the native zip picker inside Rust, imports the archive through the
 * snapshot-safe path and queues the create mutation for sync. Returns null
 * when the user cancels the dialog.
 */
export async function importSkillZip(input: {
  name: string;
  slug: string;
}): Promise<ZipImportDialogResult | null> {
  const pending = tauriInvoke("import_skill_zip_from_dialog", { request: input });
  if (pending === null) throw new Error("桌面导入仅在内置客户端中可用");
  return (await pending) as ZipImportDialogResult | null;
}

/**
 * Opens the native save picker inside Rust and packs the skill's verified
 * snapshot. Returns null when the user cancels the dialog.
 */
export async function exportSkillZip(skillId: string): Promise<string | null> {
  const pending = tauriInvoke("export_skill_zip_to_dialog", { skillId });
  if (pending === null) throw new Error("桌面导出仅在内置客户端中可用");
  return (await pending) as string | null;
}

export interface SkillDeploymentRecord {
  skillId: string;
  agentProfileId: string;
  deployedBlobHash: string;
  targetPath: string;
  state:
    | "installing"
    | "installed"
    | "updating"
    | "removing"
    | "modified"
    | "missing"
    | "failed"
    | "revoked";
  lastError: string | null;
}

export interface DeploySkillResult {
  deployment: SkillDeploymentRecord;
  recoveryPending: boolean;
}

export interface DeployBatchItem {
  agentProfileId: string;
  result: DeploySkillResult | null;
  error: string | null;
}

export interface UninstallSkillResult {
  skillId: string;
  agentProfileId: string;
  targetExisted: boolean;
  recoveryPending: boolean;
}

export interface AgentProfileRecord {
  id: string;
  descriptorId: string;
  displayName: string;
  skillRoot: string;
  enabled: boolean;
  isCustom: boolean;
}

export interface AgentInstance {
  id: string;
  descriptorId: string;
  displayName: string;
  skillRoot: string;
  enabled: boolean;
  detected: boolean;
  skillRootExists: boolean;
}

export interface AgentDiscoveryResult {
  descriptor: { id: string; displayName: string; kind: string };
  instances: AgentInstance[];
  error: string | null;
}

export interface DeploymentPrefs {
  defaultTargets: string[];
  skillTargets: string[] | null;
}

export interface HydrationOutcome {
  skillId: string;
  manifestHash: string;
  blobsDownloaded: number;
  workspaceCreated: boolean;
  workspacePath: string;
}

export interface DesktopSyncState {
  protocolVersion: number;
  deviceId: string | null;
  serverUserId: string | null;
  serverUrl?: string | null;
  serverLoginIdentity?: string | null;
  serverCursor: string | null;
  lastSuccessfulPushAt: string | null;
  lastSuccessfulPullAt: string | null;
  lastServerError: string | null;
}

export interface DesktopConflict {
  skillId: string;
  localName: string;
  localSlug: string;
  localSnapshotHash: string;
  localBaseRevision: number | null;
  remoteHeadRevision: number | null;
  remotePackageManifestHash: string | null;
  mutationId: string;
  mutationOperation: string;
}

export interface ResolutionApplied {
  kept: "local" | "remote";
  appliedMutations: number;
}

/** Deploys one skill to a single agent profile (auto-hydrates first). */
export async function deploySkillToAgent(
  skillId: string,
  agentProfileId: string,
): Promise<DeploySkillResult> {
  const pending = tauriInvoke("deploy_skill_to_agent", {
    request: { skillId, agentProfileId },
  });
  if (pending === null) throw new Error("部署仅在内置客户端中可用");
  return (await pending) as DeploySkillResult;
}

/** Deploys one skill to several agent profiles; each target reports separately. */
export async function deploySkillBatch(
  skillId: string,
  agentProfileIds: string[],
): Promise<DeployBatchItem[]> {
  const pending = tauriInvoke("deploy_skill_batch", {
    request: { skillId, agentProfileIds },
  });
  if (pending === null) throw new Error("批量部署仅在内置客户端中可用");
  return (await pending) as DeployBatchItem[];
}

export async function uninstallSkillFromAgent(
  skillId: string,
  agentProfileId: string,
): Promise<UninstallSkillResult> {
  const pending = tauriInvoke("uninstall_skill_from_agent", {
    request: { skillId, agentProfileId },
  });
  if (pending === null) throw new Error("卸载仅在内置客户端中可用");
  return (await pending) as UninstallSkillResult;
}

export async function listDeployments(): Promise<SkillDeploymentRecord[]> {
  const pending = tauriInvoke("list_deployments");
  if (pending === null) throw new Error("部署管理仅在内置客户端中可用");
  return (await pending) as SkillDeploymentRecord[];
}

export async function discoverAgents(): Promise<AgentDiscoveryResult[]> {
  const pending = tauriInvoke("discover_agents");
  if (pending === null) throw new Error("Agent 发现仅在内置客户端中可用");
  return (await pending) as AgentDiscoveryResult[];
}

export async function listAgentProfiles(): Promise<AgentProfileRecord[]> {
  const pending = tauriInvoke("list_agent_profiles");
  if (pending === null) throw new Error("Agent 配置仅在内置客户端中可用");
  return (await pending) as AgentProfileRecord[];
}

export interface SaveAgentProfileInput {
  id: string;
  descriptorId: string;
  displayName: string;
  skillRoot: string;
  enabled: boolean;
  isCustom: boolean;
}

export async function saveAgentProfile(
  profile: SaveAgentProfileInput,
): Promise<AgentProfileRecord> {
  const pending = tauriInvoke("save_agent_profile", { request: profile });
  if (pending === null) throw new Error("Agent 配置仅在内置客户端中可用");
  return (await pending) as AgentProfileRecord;
}

export async function getDeploymentPrefs(
  skillId?: string,
): Promise<DeploymentPrefs> {
  const pending = tauriInvoke("get_deployment_prefs", { skillId: skillId ?? null });
  if (pending === null) throw new Error("部署偏好仅在内置客户端中可用");
  return (await pending) as DeploymentPrefs;
}

export async function setDeploymentPrefs(input: {
  skillId: string | null;
  profileIds: string[];
}): Promise<DeploymentPrefs> {
  const pending = tauriInvoke("set_deployment_prefs", { request: input });
  if (pending === null) throw new Error("部署偏好仅在内置客户端中可用");
  return (await pending) as DeploymentPrefs;
}

/** Downloads (hydrates) a pulled skill's files into a local workspace. */
export async function hydrateSkillWorkspace(
  skillId: string,
): Promise<HydrationOutcome> {
  const pending = tauriInvoke("hydrate_skill_workspace", {
    request: { skillId },
  });
  if (pending === null) throw new Error("下载到本地仅在内置客户端中可用");
  return (await pending) as HydrationOutcome;
}

export interface HydrationOutcome {
  skillId: string;
  manifestHash: string;
  blobsDownloaded: number;
  workspaceCreated: boolean;
  workspacePath: string;
}

export async function syncNow(): Promise<unknown> {
  const pending = tauriInvoke("sync_now", {});
  if (pending === null) throw new Error("立即同步仅在内置客户端中可用");
  return await pending;
}

export async function getSyncState(): Promise<DesktopSyncState | null> {
  const pending = tauriInvoke("sync_state", {});
  if (pending === null) return null;
  return (await pending) as DesktopSyncState;
}

export async function listConflicts(): Promise<DesktopConflict[] | null> {
  const pending = tauriInvoke("list_conflicts", {});
  if (pending === null) return null;
  return (await pending) as DesktopConflict[];
}

export async function resolveConflict(
  skillId: string,
  mode: "keep_local" | "keep_remote",
  confirmed: boolean,
): Promise<unknown> {
  const pending = tauriInvoke("resolve_conflict", {
    skillId,
    mode,
    confirmed,
  });
  if (pending === null) throw new Error("冲突解决仅在内置客户端中可用");
  return await pending;
}


export interface DesktopLoginResult {
  deviceId: string;
  clientInstanceId: string;
  expiresIn: number;
}

/**
 * Registers this desktop as a sync device and stores refresh credentials in
 * the OS keyring. Must run after the web session login so the sync worker
 * can push/pull for the logged-in user.
 */
export async function desktopLogin(input: {
  username: string;
  password: string;
  baseUrl: string;
}): Promise<DesktopLoginResult | null> {
  const pending = tauriInvoke("desktop_login", { request: input });
  if (pending === null) return null;
  return (await pending) as DesktopLoginResult;
}

/** Revokes and clears the desktop refresh session; failures are surfaced. */
export async function desktopLogout(): Promise<void> {
  const pending = tauriInvoke("desktop_logout", {});
  if (pending === null) return;
  await pending;
}
