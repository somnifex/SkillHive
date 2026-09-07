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
