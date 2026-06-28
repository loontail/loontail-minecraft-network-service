import type { LucideIcon } from "lucide-react";

import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/shared/lib/cn";

interface StatCardProps {
  label: string;
  icon: LucideIcon;
  hint?: string;
  isLoading?: boolean;
  // Pass `value` (numeric, toLocaleString'd) or `display` (pre-formatted); `display` wins.
  value?: number;
  display?: string;
}

export function StatCard({
  label,
  icon: Icon,
  value,
  display,
  hint,
  isLoading,
}: StatCardProps) {
  const isDisplay = display !== undefined || value === undefined;
  const showSkeleton =
    isLoading || (display === undefined && value === undefined);

  return (
    <Card className="gap-3 py-5 transition-colors hover:border-edge-lg">
      <CardHeader className="gap-2">
        <CardDescription className="flex items-center gap-2 text-text-mute">
          <Icon className="size-4 text-text-faint" aria-hidden />
          {label}
        </CardDescription>
        <CardTitle
          className={cn(
            "text-display tabular-nums text-text-hi",
            value === 0 && !isDisplay && !showSkeleton && "text-text-mute",
          )}
        >
          {showSkeleton ? (
            <Skeleton className="h-8 w-14" />
          ) : isDisplay ? (
            display
          ) : (
            value?.toLocaleString()
          )}
        </CardTitle>
        {hint ? <p className="text-caption text-text-faint">{hint}</p> : null}
      </CardHeader>
    </Card>
  );
}
