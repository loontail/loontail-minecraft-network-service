import type { LucideIcon } from "lucide-react";

import { cn } from "@/shared/lib/cn";

export interface SectionTab<T extends string> {
  value: T;
  label: string;
  icon?: LucideIcon;
}

interface SectionTabsProps<T extends string> {
  tabs: SectionTab<T>[];
  value: T;
  onChange: (value: T) => void;
  // When set, each tab is given a stable `id` and an `aria-controls` pointing at
  // `{idBase}-panel-{value}`, associating it with a `role="tabpanel"` the consumer
  // renders. Use `tabId`/`panelId` to wire those ids up. Omit when there are no
  // panels (e.g. a tab strip that just swaps a table query).
  idBase?: string;
}

export function tabId(idBase: string, value: string): string {
  return `${idBase}-tab-${value}`;
}

export function panelId(idBase: string, value: string): string {
  return `${idBase}-panel-${value}`;
}

export function SectionTabs<T extends string>({
  tabs,
  value,
  onChange,
  idBase,
}: SectionTabsProps<T>) {
  return (
    <div
      role="tablist"
      className="inline-flex items-center gap-1 rounded-md border border-edge bg-surface-1 p-1"
    >
      {tabs.map((tab) => {
        const Icon = tab.icon;
        const active = tab.value === value;
        return (
          <button
            key={tab.value}
            type="button"
            role="tab"
            id={idBase ? tabId(idBase, tab.value) : undefined}
            aria-selected={active}
            aria-controls={idBase ? panelId(idBase, tab.value) : undefined}
            onClick={() => onChange(tab.value)}
            className={cn(
              "inline-flex items-center gap-2 rounded-sm px-3 py-1.5 text-body-med transition-colors",
              "outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
              active
                ? "bg-surface-3 text-text-hi"
                : "text-text-mute hover:text-text",
            )}
          >
            {Icon ? <Icon className="size-4" /> : null}
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
