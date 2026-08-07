import { AlertTriangle, XCircle } from "lucide-react";

import { Badge, type badgeVariants } from "@/components/ui/badge";
import type { VariantProps } from "class-variance-authority";

type BadgeVariant = VariantProps<typeof badgeVariants>["variant"];

function variantForStatus(status: number): BadgeVariant {
  if (status >= 500) return "destructive";
  if (status >= 400) return "secondary";
  return "outline";
}

// Severity is carried by the icon, not hue: the palette is monochrome.
export function StatusBadge({ status }: { status: number }) {
  return (
    <Badge variant={variantForStatus(status)} className="gap-1 tabular-nums">
      {status >= 500 ? (
        <XCircle className="size-3" aria-hidden />
      ) : status >= 400 ? (
        <AlertTriangle className="size-3" aria-hidden />
      ) : null}
      {status}
    </Badge>
  );
}
