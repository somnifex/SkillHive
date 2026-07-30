import type { ReactNode } from "react";
import { Space, Typography } from "antd";

interface Props {
  title: string;
  description?: string;
  actions?: ReactNode;
}

export function PageHeader({ title, description, actions }: Props) {
  return (
    <div className="page-header">
      <div>
        <Typography.Title level={2}>{title}</Typography.Title>
        {description && (
          <Typography.Paragraph type="secondary">
            {description}
          </Typography.Paragraph>
        )}
      </div>
      <Space wrap>{actions}</Space>
    </div>
  );
}
