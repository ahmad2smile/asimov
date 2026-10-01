import { ChevronLeft, ChevronRight } from 'lucide-react';

interface PagerProps {
  pageNumber: number;
  pageSize: number;
  /** Rows on this page. */
  count: number;
  /** Matching rows across all pages, when known without a scan. */
  total: number | undefined;
  hasMore: boolean;
  canGoBack: boolean;
  onPrevious: () => void;
  onNext: () => void;
}

export function Pager({ pageNumber, pageSize, count, total, hasMore, canGoBack, onPrevious, onNext }: PagerProps) {
  const first = count === 0 ? 0 : (pageNumber - 1) * pageSize + 1;
  const last = (pageNumber - 1) * pageSize + count;
  const button =
    'inline-flex size-8 items-center justify-center rounded-md border hover:bg-accent disabled:pointer-events-none disabled:opacity-40';
  return (
    <div className="flex items-center justify-end gap-3 pt-4 text-sm text-muted-foreground">
      <span className="tabular-nums">
        {first}–{last}
        {total !== undefined && ` of ${total}`}
      </span>
      <button
        className={button}
        aria-label="Previous page"
        disabled={!canGoBack}
        onClick={onPrevious}
      >
        <ChevronLeft className="size-4" />
      </button>
      <button
        className={button}
        aria-label="Next page"
        disabled={!hasMore}
        onClick={onNext}
      >
        <ChevronRight className="size-4" />
      </button>
    </div>
  );
}
