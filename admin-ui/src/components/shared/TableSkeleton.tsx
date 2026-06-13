import { Skeleton } from "@/components/ui/skeleton";

interface TableSkeletonProps {
  rows?: number;
  columns?: number;
}

export function TableSkeleton({ rows = 5, columns = 4 }: TableSkeletonProps) {
  const rowKeys = Array.from({ length: rows }, (_, i) => `row-${i}`);
  const colKeys = Array.from({ length: columns }, (_, i) => `col-${i}`);
  return (
    <div className="space-y-2.5" data-testid="table-skeleton">
      {rowKeys.map((rowKey) => (
        <div key={rowKey} className="flex items-center gap-3">
          {colKeys.map((colKey, colIndex) => (
            <Skeleton
              key={colKey}
              className="h-8 flex-1"
              style={{ flexGrow: colIndex === 0 ? 2 : 1 }}
            />
          ))}
        </div>
      ))}
    </div>
  );
}
