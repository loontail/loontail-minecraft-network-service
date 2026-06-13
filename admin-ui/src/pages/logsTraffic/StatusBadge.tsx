import { Badge, type badgeVariants } from "@/components/ui/badge";
import type { VariantProps } from "class-variance-authority";

type BadgeVariant = VariantProps<typeof badgeVariants>["variant"];

function variantForStatus(status: number): BadgeVariant {
  if (status >= 500) return "destructive";
  if (status >= 400) return "secondary";
  return "outline";
}

/// HTTP status code rendered as a class-coloured badge (4xx warn, 5xx error).
export function StatusBadge({ status }: { status: number }) {
  return (
    <Badge variant={variantForStatus(status)} className="tabular-nums">
      {status}
    </Badge>
  );
}
