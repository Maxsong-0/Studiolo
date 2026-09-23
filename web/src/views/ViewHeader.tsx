import type { ReactNode } from 'react';
import '../styles/workspace.css';

/** 视图标题行：左边图名（h1）与一句说明，右边可放本视图的次要动作。 */
export function ViewHeader({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children?: ReactNode;
}) {
  return (
    <header className="view-header">
      <div className="view-heading">
        <h1>{title}</h1>
        {description ? <p>{description}</p> : null}
      </div>
      {children ? <div className="view-header-actions">{children}</div> : null}
    </header>
  );
}
