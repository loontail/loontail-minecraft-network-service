import { SegmentedControl } from "@/components/shared/SegmentedControl";
import type { TrafficWindow } from "@/shared/types";

const TIMESERIES_WINDOWS: { value: TrafficWindow; label: string }[] = [
  { value: "24h", label: "24h" },
  { value: "7d", label: "7d" },
  { value: "30d", label: "30d" },
];

interface WindowSelectorProps {
  value: TrafficWindow;
  onChange: (next: TrafficWindow) => void;
}

export function WindowSelector({ value, onChange }: WindowSelectorProps) {
  return (
    <SegmentedControl
      mode="radio"
      ariaLabel="Chart window"
      items={TIMESERIES_WINDOWS}
      value={value}
      onChange={onChange}
      itemClassName="px-2.5 py-1 text-caption font-semibold tabular-nums"
    />
  );
}
