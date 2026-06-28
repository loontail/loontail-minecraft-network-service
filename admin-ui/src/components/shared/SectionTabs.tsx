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
  // When set, each tab is given a stable `id` and an `aria-controls` pointing at
  // `{idBase}-panel-{value}`, associating it with a `role="tabpanel"` the consumer
  // renders. Use `tabId`/`panelId` to wire those ids up. Omit when there are no
  // panels (e.g. a tab strip that just swaps a table query).
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
