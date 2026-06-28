import {
  panelId,
  type SegmentedItem,
  SegmentedControl,
  tabId,
} from "@/components/shared/SegmentedControl";

export type SectionTab<T extends string> = SegmentedItem<T>;

export { panelId, tabId };

interface SectionTabsProps<T extends string> {
  tabs: SectionTab<T>[];
  value: T;
  onChange: (value: T) => void;
  ariaLabel?: string;
  // Set to wire `aria-controls` to a `{idBase}-panel-{value}` tabpanel; omit for tab strips without panels.
  idBase?: string;
}

export function SectionTabs<T extends string>({
  tabs,
  value,
  onChange,
  ariaLabel,
  idBase,
}: SectionTabsProps<T>) {
  return (
    <SegmentedControl
      mode="tabs"
      items={tabs}
      value={value}
      onChange={onChange}
      ariaLabel={ariaLabel}
      idBase={idBase}
    />
  );
}
