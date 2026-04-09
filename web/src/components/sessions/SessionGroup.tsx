import type { ComponentChildren } from "preact";

interface SessionGroupProps {
  title: string;
  subtitle: string;
  collapsed?: boolean;
  onToggle?: () => void;
  children: ComponentChildren;
}

export function SessionGroup({ title, subtitle, collapsed = false, onToggle, children }: SessionGroupProps) {
  return (
    <section className="sessionGroup">
      <div className="sessionGroupShell">
        <button
          type="button"
          className="sessionGroupHeader"
          aria-expanded={!collapsed}
          onClick={onToggle}
        >
          <span className="sessionGroupHeading">
            <span className="sessionGroupTitle">{title}</span>
            <span className="sessionGroupSubtitle">{subtitle}</span>
          </span>
          <span className="sessionGroupToggle" aria-hidden="true">{collapsed ? "+" : "-"}</span>
        </button>
        {collapsed ? null : <div className="sessionGroupList">{children}</div>}
      </div>
    </section>
  );
}
