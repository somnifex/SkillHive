import { FolderPlus, RefreshCw, Rocket, Trash2 } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  App,
  Button,
  Checkbox,
  Empty,
  Form,
  Input,
  Modal,
  Popconfirm,
  Select,
  Table,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import { useState } from "react";

import {
  desktopErrorMessage as errorMessage,
  discoverAgents,
  getDeploymentPrefs,
  hasDesktopCommands,
  listAgentProfiles,
  listDeployments,
  saveAgentProfile,
  setDeploymentPrefs,
  uninstallSkillFromAgent,
  type AgentDiscoveryResult,
  type SkillDeploymentRecord,
} from "../api/desktop";
import { PageHeader } from "../components/PageHeader";

const deploymentStateLabels: Record<string, { label: string; color: string }> = {
  installing: { label: "安装中", color: "processing" },
  installed: { label: "已安装", color: "success" },
  updating: { label: "更新中", color: "processing" },
  removing: { label: "移除中", color: "processing" },
  modified: { label: "本地已修改", color: "warning" },
  missing: { label: "目录缺失", color: "warning" },
  failed: { label: "失败", color: "error" },
  revoked: { label: "已失效", color: "default" },
};

interface CustomAgentForm {
  displayName: string;
  skillRoot: string;
}

function slugKey(raw: string): string {
  return raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);
}

interface TargetOption {
  value: string;
  label: string;
}

/** Flattens discovery results plus persisted custom profiles into deploy targets. */
function collectTargetOptions(
  discovery: AgentDiscoveryResult[],
  profiles: Awaited<ReturnType<typeof listAgentProfiles>>,
): TargetOption[] {
  const options: TargetOption[] = [];
  const seen = new Set<string>();
  for (const result of discovery) {
    for (const instance of result.instances) {
      if (seen.has(instance.id)) continue;
      seen.add(instance.id);
      options.push({ value: instance.id, label: instance.displayName });
    }
  }
  for (const profile of profiles) {
    if (seen.has(profile.id)) continue;
    seen.add(profile.id);
    options.push({ value: profile.id, label: profile.displayName });
  }
  return options;
}

export function AgentDeployPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const desktop = hasDesktopCommands();
  const [customOpen, setCustomOpen] = useState(false);
  const [defaultTargets, setDefaultTargetsState] = useState<string[] | undefined>();
  const [customForm] = Form.useForm<CustomAgentForm>();

  const discovery = useQuery({
    queryKey: ["desktop-agents"],
    queryFn: discoverAgents,
    enabled: desktop,
  });
  const profiles = useQuery({
    queryKey: ["agent-profiles"],
    queryFn: listAgentProfiles,
    enabled: desktop,
  });
  const deployments = useQuery({
    queryKey: ["deployments"],
    queryFn: listDeployments,
    enabled: desktop,
  });
  const prefs = useQuery({
    queryKey: ["deployment-prefs"],
    queryFn: () => getDeploymentPrefs(),
    enabled: desktop,
  });

  const invalidateAll = () => {
    queryClient.invalidateQueries({ queryKey: ["deployments"] });
    queryClient.invalidateQueries({ queryKey: ["agent-profiles"] });
    queryClient.invalidateQueries({ queryKey: ["desktop-agents"] });
    queryClient.invalidateQueries({ queryKey: ["deployment-prefs"] });
  };

  const toggleProfile = useMutation({
    mutationFn: (input: { profile: { id: string; descriptorId: string; displayName: string; skillRoot: string; isCustom: boolean }; enabled: boolean }) =>
      saveAgentProfile({
        id: input.profile.id,
        descriptorId: input.profile.descriptorId,
        displayName: input.profile.displayName,
        skillRoot: input.profile.skillRoot,
        enabled: input.enabled,
        isCustom: input.profile.descriptorId.startsWith("custom:"),
      }),
    onSuccess: () => {
      message.success("Agent 配置已保存");
      invalidateAll();
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const saveCustom = useMutation({
    mutationFn: async (values: CustomAgentForm) => {
      const key = slugKey(values.displayName) || `dir-${Date.now()}`;
      return saveAgentProfile({
        id: `custom:${key}:configured`,
        descriptorId: `custom:${key}`,
        displayName: values.displayName,
        skillRoot: values.skillRoot,
        enabled: true,
        isCustom: true,
      });
    },
    onSuccess: () => {
      message.success("自定义目录已添加");
      setCustomOpen(false);
      customForm.resetFields();
      invalidateAll();
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const saveDefaults = useMutation({
    mutationFn: (profileIds: string[]) =>
      setDeploymentPrefs({ skillId: null, profileIds }),
    onSuccess: (_data, profileIds) => {
      setDefaultTargetsState(profileIds);
      message.success("默认部署目标已保存");
      invalidateAll();
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  const uninstall = useMutation({
    mutationFn: (input: { skillId: string; agentProfileId: string }) =>
      uninstallSkillFromAgent(input.skillId, input.agentProfileId),
    onSuccess: () => {
      message.success("已从 Agent 目录卸载");
      invalidateAll();
    },
    onError: (error) => message.error(errorMessage(error)),
  });

  if (!desktop) {
    return (
      <>
        <PageHeader
          title="Agent 部署"
          description="一键把 Skill 安装到本机常见的 Agent 技能目录。"
        />
        <Alert
          type="info"
          showIcon
          message="此功能仅在内置桌面客户端中可用"
          description="网页版无法访问本机文件系统。请使用 SkillHive 桌面客户端打开后管理 Agent 部署。"
        />
      </>
    );
  }

  const targetOptions = collectTargetOptions(discovery.data ?? [], profiles.data ?? []);
  const profileName = (id: string) =>
    targetOptions.find((option) => option.value === id)?.label ?? id;

  return (
    <>
      <PageHeader
        title="Agent 部署"
        description="发现本机 Agent 的技能目录，配置默认部署目标，一键安装与卸载。"
        actions={
          <>
            <Button
              icon={<RefreshCw size={16} aria-hidden="true" />}
              onClick={() => {
                void discovery.refetch();
                void deployments.refetch();
              }}
            >
              重新扫描
            </Button>
            <Button
              type="primary"
              icon={<FolderPlus size={16} aria-hidden="true" />}
              onClick={() => {
                customForm.resetFields();
                setCustomOpen(true);
              }}
            >
              添加自定义目录
            </Button>
          </>
        }
      />

      <div className="agent-grid">
        {(discovery.data ?? []).map((result) => (
          <div key={result.descriptor.id} className="agent-card">
            <div className="agent-card-head">
              <Rocket size={18} aria-hidden="true" />
              <strong>{result.descriptor.displayName}</strong>
              {result.error ? (
                <Tooltip title={result.error}>
                  <Tag color="error">发现失败</Tag>
                </Tooltip>
              ) : result.instances.length === 0 ? (
                <Tag>未检测到</Tag>
              ) : (
                <Tag color="success">已检测到</Tag>
              )}
            </div>
            {result.instances.map((instance) => (
              <div key={instance.id} className="agent-card-body">
                <Typography.Text type="secondary" className="agent-root">
                  {instance.skillRoot}
                </Typography.Text>
                <Checkbox
                  checked={instance.enabled}
                  onChange={(event) =>
                    toggleProfile.mutate({
                      profile: {
                        id: instance.id,
                        descriptorId: instance.descriptorId,
                        displayName: instance.displayName,
                        skillRoot: instance.skillRoot,
                        isCustom: instance.descriptorId.startsWith("custom:"),
                      },
                      enabled: event.target.checked,
                    })
                  }
                >
                  启用部署
                </Checkbox>
              </div>
            ))}
          </div>
        ))}
        {discovery.data?.length === 0 && (
          <Empty description="未发现任何 Agent 目录" />
        )}
      </div>

      <Typography.Title level={5}>默认部署目标</Typography.Title>
      <Typography.Paragraph type="secondary">
        一键部署会安装到以下全部启用的目标；单个 Skill 可以在「我的 Skills」中单独覆盖。
      </Typography.Paragraph>
      <Select
        mode="multiple"
        allowClear
        placeholder="选择默认部署目标"
        style={{ maxWidth: 560 }}
        value={defaultTargets ?? prefs.data?.defaultTargets ?? []}
        loading={prefs.isLoading}
        options={targetOptions}
        onChange={(values: string[]) => saveDefaults.mutate(values)}
      />

      <Typography.Title level={5} style={{ marginTop: 24 }}>
        已部署 Skill
      </Typography.Title>
      <Table
        rowKey={(record: SkillDeploymentRecord) =>
          `${record.skillId}:${record.agentProfileId}`
        }
        loading={deployments.isLoading}
        dataSource={deployments.data}
        locale={{ emptyText: <Empty description="还没有部署任何 Skill" /> }}
        pagination={false}
        columns={[
          { title: "Skill", dataIndex: "skillId", ellipsis: true },
          {
            title: "目标",
            dataIndex: "agentProfileId",
            render: (value: string) => profileName(value),
          },
          {
            title: "安装路径",
            dataIndex: "targetPath",
            ellipsis: true,
            render: (value: string) => (
              <Typography.Text copyable={{ text: value }} type="secondary">
                {value}
              </Typography.Text>
            ),
          },
          {
            title: "状态",
            dataIndex: "state",
            render: (value: string, record) => {
              const meta = deploymentStateLabels[value] ?? { label: value, color: "default" };
              return record.lastError ? (
                <Tooltip title={record.lastError}>
                  <Tag color={meta.color}>{meta.label}</Tag>
                </Tooltip>
              ) : (
                <Tag color={meta.color}>{meta.label}</Tag>
              );
            },
          },
          {
            title: "操作",
            width: 100,
            render: (_: unknown, record: SkillDeploymentRecord) => (
              <Popconfirm
                title="从该 Agent 目录卸载？"
                description="SkillHive 管理的部署目录会被移除。"
                onConfirm={() =>
                  uninstall.mutate({
                    skillId: record.skillId,
                    agentProfileId: record.agentProfileId,
                  })
                }
              >
                <Button
                  danger
                  type="text"
                  aria-label="卸载"
                  icon={<Trash2 size={16} aria-hidden="true" />}
                />
              </Popconfirm>
            ),
          },
        ]}
      />

      <Modal
        open={customOpen}
        title="添加自定义部署目录"
        okText="保存"
        confirmLoading={saveCustom.isPending}
        onCancel={() => setCustomOpen(false)}
        onOk={() => customForm.submit()}
      >
        <Form form={customForm} layout="vertical" onFinish={(values) => saveCustom.mutate(values)}>
          <Form.Item name="displayName" label="显示名称" rules={[{ required: true }]}>
            <Input placeholder="例如：我的自定义 Agent" />
          </Form.Item>
          <Form.Item
            name="skillRoot"
            label="技能根目录（绝对路径）"
            rules={[{ required: true }]}
          >
            <Input placeholder="D:\\agents\\my-agent\\skills" />
          </Form.Item>
          <Typography.Text type="secondary">
            目录必须为绝对路径且不能是符号链接；SkillHive 只写入该目录下以 Skill slug
            命名的子目录。
          </Typography.Text>
        </Form>
      </Modal>
    </>
  );
}
