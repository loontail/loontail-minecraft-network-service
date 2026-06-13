import { cn } from "@/shared/lib/cn";

export type TimeseriesWindow = "24h" | "7d" | "30d";

export const TIMESERIES_WINDOWS: { value: TimeseriesWindow; label: string }[] = [
  { value: "24h", label: "24h" },
  { value: "7d", label: "7d" },
  { value: "30d", label: "30d" },
];

interface WindowSelectorProps {
  value: TimeseriesWindow;
  onChange: (next: TimeseriesWindow) => void;
}

export function WindowSelector({ value, onChange }: WindowSelectorProps) {
  return (
    <div
      role="radiogroup"
      aria-label="Chart window"
      className="inline-flex items-center gap-1 rounded-md border border-edge bg-surface-1 p-1"
    >
      {TIMESERIES_WINDOWS.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option.value)}
            className={cn(
              "rounded px-2.5 py-1 text-caption font-semibold tabular-nums transition-colors",
              "outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
              active
                ? "bg-surface-3 text-text-hi"
                : "text-text-mute hover:text-text",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
