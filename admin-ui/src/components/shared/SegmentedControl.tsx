import { useRef } from "react";
import type { KeyboardEvent } from "react";
import type { LucideIcon } from "lucide-react";

import { cn } from "@/shared/lib/cn";

export interface SegmentedItem<T extends string> {
  value: T;
  label: string;
  icon?: LucideIcon;
}

export type SegmentedMode = "tabs" | "radio";

interface SegmentedControlProps<T extends string> {
  items: SegmentedItem<T>[];
  value: T;
  onChange: (value: T) => void;
  // "tabs" → role=tablist/tab (+ aria-selected); "radio" → role=radiogroup/radio
  // (+ aria-checked). Both implement WAI-ARIA roving focus.
  mode?: SegmentedMode;
  ariaLabel?: string;
  // When set (tabs that drive panels), each tab gets a stable `id` and
  // `aria-controls` pointing at `{idBase}-panel-{value}`. Use `tabId`/`panelId`
  // to wire the matching `role="tabpanel"`.
  idBase?: string;
  className?: string;
  itemClassName?: string;
}

export function tabId(idBase: string, value: string): string {
  return `${idBase}-tab-${value}`;
}

export function panelId(idBase: string, value: string): string {
  return `${idBase}-panel-${value}`;
}

export function SegmentedControl<T extends string>({
  items,
  value,
  onChange,
  mode = "tabs",
  ariaLabel,
  idBase,
  className,
  itemClassName,
}: SegmentedControlProps<T>) {
  const buttonsRef = useRef<(HTMLButtonElement | null)[]>([]);

  // Roving focus: Arrow keys + Home/End move both focus and selection to the next
  // enabled item, so the announced tablist/radiogroup role matches behaviour.
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const count = items.length;
    if (count === 0) {
      return;
    }
    const current = items.findIndex((item) => item.value === value);
    let nextIndex: number | null = null;
    switch (event.key) {
      case "ArrowRight":
      case "ArrowDown":
        nextIndex = (current + 1) % count;
        break;
      case "ArrowLeft":
      case "ArrowUp":
        nextIndex = (current - 1 + count) % count;
        break;
      case "Home":
        nextIndex = 0;
        break;
      case "End":
        nextIndex = count - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    const target = items[nextIndex];
    onChange(target.value);
    buttonsRef.current[nextIndex]?.focus();
  }

  const isTabs = mode === "tabs";

  return (
    <div
      role={isTabs ? "tablist" : "radiogroup"}
      aria-label={ariaLabel}
      onKeyDown={onKeyDown}
      className={cn(
        "inline-flex items-center gap-1 rounded-md border border-edge bg-surface-1 p-1",
        className,
      )}
    >
      {items.map((item, index) => {
        const Icon = item.icon;
        const active = item.value === value;
        return (
          <button
            key={item.value}
            ref={(el) => {
              buttonsRef.current[index] = el;
            }}
            type="button"
            role={isTabs ? "tab" : "radio"}
            id={isTabs && idBase ? tabId(idBase, item.value) : undefined}
            aria-selected={isTabs ? active : undefined}
            aria-checked={isTabs ? undefined : active}
            aria-controls={
              isTabs && idBase ? panelId(idBase, item.value) : undefined
            }
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(item.value)}
            className={cn(
              "inline-flex items-center gap-2 rounded-sm px-3 py-1.5 text-body-med transition-colors",
              "outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
              active
                ? "bg-surface-3 text-text-hi"
                : "text-text-mute hover:text-text",
              itemClassName,
            )}
          >
            {Icon ? <Icon className="size-4" /> : null}
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
