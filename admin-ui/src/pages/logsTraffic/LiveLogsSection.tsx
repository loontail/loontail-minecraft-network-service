import { RefreshCw, ScrollText } from "lucide-react";

import { EmptyState } from "@/components/shared/EmptyState";
import { TableSkeleton } from "@/components/shared/TableSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useLogsTail } from "@/features/requestLogs/api";
import { formatRelative } from "@/shared/lib/format";
import type { AuthKind, RequestLogEntry } from "@/shared/types";

import { formatLatency } from "./format";
import { StatusBadge } from "./StatusBadge";

const AUTH_LABEL: Record<AuthKind, string> = {
  session: "Session",
  admin: "Admin",
  anon: "Anon",
};

function AuthBadge({ kind }: { kind: AuthKind }) {
  return (
    <Badge variant={kind === "anon" ? "outline" : "secondary"}>
      {AUTH_LABEL[kind] ?? kind}
    </Badge>
  );
}

function LogRow({ entry }: { entry: RequestLogEntry }) {
  return (
    <TableRow>
      <TableCell className="font-mono text-caption font-semibold text-text">
        {entry.method}
      </TableCell>
      <TableCell className="max-w-[22rem] truncate font-mono text-caption text-text">
        {entry.path}
      </TableCell>
      <TableCell>
        <StatusBadge status={entry.status} />
      </TableCell>
      <TableCell className="text-right tabular-nums text-text-mute">
        {formatLatency(entry.latencyMs)}
      </TableCell>
      <TableCell>
        <div className="flex items-center gap-2">
          <AuthBadge kind={entry.authKind} />
          {entry.userId ? (
            <span className="font-mono text-caption text-text-mute">
              {entry.userId.slice(0, 8)}
            </span>
          ) : null}
        </div>
      </TableCell>
      <TableCell className="font-mono text-caption text-text-mute">
        {entry.ip ?? "—"}
      </TableCell>
      <TableCell className="text-text-mute">
        {formatRelative(entry.ts)}
      </TableCell>
    </TableRow>
  );
}

export function LiveLogsSection() {
  const tail = useLogsTail();
  const entries = tail.data?.entries ?? [];

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div className="flex flex-col gap-1.5">
          <CardTitle className="flex items-center gap-2 text-h2 text-text-hi">
            <ScrollText className="size-4 text-text-faint" aria-hidden />
            Backend logs
          </CardTitle>
          <CardDescription>
            Live request stream from the in-memory ring buffer · refreshes every
            5s.
          </CardDescription>
        </div>
        <div className="flex items-center gap-2">
          <span
            className="inline-flex items-center gap-1.5 text-caption text-text-faint"
            aria-live="polite"
          >
            <span className="relative flex size-2">
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-cta opacity-60" />
              <span className="relative inline-flex size-2 rounded-full bg-cta" />
            </span>
            Live
          </span>
          <Button
            variant="ghost"
            size="icon"
            aria-label="Refresh logs"
            disabled={tail.isFetching}
            onClick={() => void tail.refetch()}
          >
            <RefreshCw
              className={tail.isFetching ? "size-4 animate-spin" : "size-4"}
            />
          </Button>
        </div>
      </CardHeader>
      <CardContent>
        {tail.isLoading ? (
          <TableSkeleton rows={8} columns={6} />
        ) : tail.isError ? (
          <EmptyState
            icon={ScrollText}
            title="Could not load logs"
            description="The log tail request failed. It will retry automatically."
            action={
              <Button variant="outline" onClick={() => void tail.refetch()}>
                Retry
              </Button>
            }
          />
        ) : entries.length === 0 ? (
          <EmptyState
            icon={ScrollText}
            title="No requests captured yet"
            description="Incoming requests will stream into this table as they arrive."
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Method</TableHead>
                <TableHead>Path</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="text-right">Latency</TableHead>
                <TableHead>User</TableHead>
                <TableHead>IP</TableHead>
                <TableHead>Time</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {entries.map((entry, i) => (
                <LogRow
                  // why: ring-buffer entries have no id; ts+i is stable within a snapshot.
                  key={`${entry.ts}-${i}`}
                  entry={entry}
                />
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}
