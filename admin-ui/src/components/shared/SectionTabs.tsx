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
}

export function SectionTabs<T extends string>({
  tabs,
  value,
  onChange,
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
            aria-selected={active}
            onClick={() => onChange(tab.value)}
            className={cn(
              "inline-flex items-center gap-2 rounded-sm px-3 py-1.5 text-body-med transition-colors",
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
