import { RefreshCw } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Badge, Button, Tooltip, Typography } from "antd";

import { getSyncState, hasDesktopCommands, listConflicts, syncNow } from "../api/desktop";

function relativeTime(iso: string | null): string {
  if (!iso) return "从未同步";
  const timestamp = Date.parse(iso);
  if (Number.isNaN(timestamp)) return "从未同步";
  const seconds = Math.max(0, Math.round((Date.now() - timestamp) / 1000));
  if (seconds < 60) return `${seconds} 秒前`;
  if (seconds < 3600) return `${Math.round(seconds / 60)} 分钟前`;
  if (seconds < 86400) return `${Math.round(seconds / 3600)} 小时前`;
  return `${Math.round(seconds / 86400)} 天前`;
}

/**
 * Desktop-only sync status chip for the top bar: last cycle time, queued
 * errors, conflict count and a manual "sync now" trigger.
 */
export function SyncStatus() {
  if (!hasDesktopCommands()) return null;
  return <SyncStatusInner />;
}

function SyncStatusInner() {
  const queryClient = useQueryClient();
  const state = useQuery({
    queryKey: ["desktop-sync-state"],
    queryFn: getSyncState,
    refetchInterval: 30_000,
  });
  const conflicts = useQuery({
    queryKey: ["desktop-conflicts"],
    queryFn: listConflicts,
    refetchInterval: 30_000,
  });

  const sync = useMutation({
    mutationFn: () => syncNow(),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["skills"] });
      void queryClient.invalidateQueries({ queryKey: ["desktop-sync-state"] });
      void queryClient.invalidateQueries({ queryKey: ["desktop-conflicts"] });
      void queryClient.invalidateQueries({ queryKey: ["deployments"] });
    },
  });

  const data = state.data ?? null;
  const loggedIn = Boolean(data?.deviceId);
  const detail = data ? (
    <div>
      <div>推送：{relativeTime(data.lastSuccessfulPushAt)}</div>
      <div>拉取：{relativeTime(data.lastSuccessfulPullAt)}</div>
      {data.lastServerError ? (
        <div style={{ color: "#ff4d4f" }}>最近错误：{data.lastServerError}</div>
      ) : null}
      {conflicts.data && conflicts.data.length > 0 ? (
        <div>{conflicts.data.length} 个待解决冲突</div>
      ) : null}
    </div>
  ) : (
    "同步状态加载中…"
  );

  if (!data) {
    return (
      <Button size="small" type="text" onClick={() => sync.mutate()} loading={sync.isPending}>
        检查同步
      </Button>
    );
  }

  return (
    <Tooltip title={detail} placement="bottom">
      <Badge dot={Boolean(data?.lastServerError)} status="error" offset={[-4, 4]}>
        <Button
          size="small"
          type="text"
          icon={
            sync.isPending ? (
              <Typography.Text type="secondary">同步中…</Typography.Text>
            ) : (
              <RefreshCw size={16} aria-hidden="true" />
            )
          }
          onClick={() => sync.mutate()}
          aria-label="立即同步"
        >
          <span className="sync-chip-label">
            {loggedIn ? relativeTime(data.lastSuccessfulPushAt) : "未登录同步"}
          </span>
        </Button>
      </Badge>
    </Tooltip>
  );
}
