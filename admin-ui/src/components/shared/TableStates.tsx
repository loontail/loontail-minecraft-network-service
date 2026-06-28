import type { LucideIcon } from "lucide-react";

import { Skeleton } from "@/components/ui/skeleton";
import { TableCell, TableRow } from "@/components/ui/table";

export function TableSkeletonRows({
  columns,
  rows = 8,
}: {
  columns: number;
  rows?: number;
}) {
  return (
    <>
      {Array.from({ length: rows }).map((_, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static placeholder rows
        <TableRow key={index}>
          {Array.from({ length: columns }).map((__, col) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: static placeholder cells
            <TableCell key={col}>
              <Skeleton className="h-8 w-20" />
            </TableCell>
          ))}
        </TableRow>
      ))}
    </>
  );
}

export function TableStateRow({
  columns,
  icon: Icon,
  title,
  description,
}: {
  columns: number;
  icon: LucideIcon;
  title: string;
  description: string;
}) {
  return (
    <TableRow className="hover:bg-transparent">
      <TableCell colSpan={columns} className="h-48 text-center">
        <div className="flex flex-col items-center justify-center gap-2 text-text-mute">
          <Icon className="size-8 text-text-faint" />
          <p className="text-body-med text-text-hi">{title}</p>
          <p className="text-caption">{description}</p>
        </div>
      </TableCell>
    </TableRow>
  );
}
