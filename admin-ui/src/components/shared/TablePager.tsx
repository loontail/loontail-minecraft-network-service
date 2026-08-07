import { ChevronLeft, ChevronRight } from "lucide-react";

import { Button } from "@/components/ui/button";

interface TablePagerProps {
  page: number;
  pageCount: number;
  total: number;
  isLoading: boolean;
  isFetching: boolean;
  noun: [singular: string, plural: string];
  onPrev: () => void;
  onNext: () => void;
}

export function TablePager({
  page,
  pageCount,
  total,
  isLoading,
  isFetching,
  noun: [singular, plural],
  onPrev,
  onNext,
}: TablePagerProps) {
  return (
    <footer className="flex items-center justify-between gap-4">
      <p className="text-caption text-text-faint">
        {isLoading
          ? "Loading…"
          : total === 0
            ? `No ${plural}`
            : `${total.toLocaleString()} ${total === 1 ? singular : plural}`}
        {isFetching && !isLoading && " · refreshing…"}
      </p>
      <div className="flex items-center gap-2">
        <span className="text-caption text-text-mute">
          Page {page} of {Math.max(pageCount, 1)}
        </span>
        <Button
          variant="outline"
          size="icon"
          aria-label="Previous page"
          disabled={page <= 1 || isLoading}
          onClick={onPrev}
        >
          <ChevronLeft className="size-4" />
        </Button>
        <Button
          variant="outline"
          size="icon"
          aria-label="Next page"
          disabled={page >= pageCount || isLoading}
          onClick={onNext}
        >
          <ChevronRight className="size-4" />
        </Button>
      </div>
    </footer>
  );
}
