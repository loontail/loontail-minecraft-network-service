import { LineChart as LineChartIcon } from "lucide-react";

export function EmptyChartState({
  title,
  detail,
}: {
  title: string;
  detail: string;
}) {
  return (
    <div className="flex aspect-video max-h-72 w-full flex-col items-center justify-center gap-2 rounded-md border border-dashed border-edge text-center">
      <LineChartIcon className="size-7 text-text-faint" aria-hidden />
      <p className="text-body-med text-text">{title}</p>
      <p className="max-w-xs text-caption text-text-faint">{detail}</p>
    </div>
  );
}
