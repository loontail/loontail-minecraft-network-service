import { SegmentedControl } from "@/components/shared/SegmentedControl";
import type { TimeRange } from "@/shared/types";

const TIME_RANGES: { value: TimeRange; label: string }[] = [
  { value: "24h", label: "24h" },
  { value: "7d", label: "7d" },
  { value: "30d", label: "30d" },
];

interface TimeRangeSelectorProps {
  value: TimeRange;
  onChange: (next: TimeRange) => void;
}

export function TimeRangeSelector({ value, onChange }: TimeRangeSelectorProps) {
  return (
    <SegmentedControl
      mode="radio"
      ariaLabel="Time range"
      items={TIME_RANGES}
      value={value}
      onChange={onChange}
      itemClassName="px-2.5 py-1 text-caption font-semibold tabular-nums"
    />
  );
}
