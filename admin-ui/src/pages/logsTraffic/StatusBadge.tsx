import { AlertTriangle, XCircle } from "lucide-react";

import { Badge, type badgeVariants } from "@/components/ui/badge";
import type { VariantProps } from "class-variance-authority";

type BadgeVariant = VariantProps<typeof badgeVariants>["variant"];

function variantForStatus(status: number): BadgeVariant {
  if (status >= 500) return "destructive";
  if (status >= 400) return "secondary";
  return "outline";
}

// The monochrome palette differentiates severity by an icon (not hue alone): a
// solid error glyph for 5xx, a warning glyph for 4xx, none for 2xx/3xx.
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
