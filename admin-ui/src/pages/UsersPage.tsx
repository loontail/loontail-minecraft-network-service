import {
  AlertCircle,
  ChevronLeft,
  ChevronRight,
  Search,
  ShieldCheck,
  UserX,
  Users,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useUsers } from "@/features/users/api";
import { errorMessage } from "@/shared/api/toast";
import type { AdminUser } from "@/shared/types";

import { CreateUserDialog } from "./users/CreateUserDialog";
import { ResetPasswordDialog } from "./users/ResetPasswordDialog";
import { UserRowActions } from "./users/UserRowActions";

const COLUMN_COUNT = 7;

function useDebounced<T>(value: T, delay = 300): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(id);
  }, [value, delay]);
  return debounced;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function shortUuid(value: string | null): string {
  if (!value) {
    return "—";
  }
  return value.length > 12 ? `${value.slice(0, 8)}…${value.slice(-4)}` : value;
}

function originVariant(origin: string): "default" | "secondary" | "outline" {
  if (origin === "admin") {
    return "default";
  }
  if (origin === "yggdrasil") {
    return "secondary";
  }
  return "outline";
}

function UserFlags({ user }: { user: AdminUser }) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      {user.isAdmin && (
        <Badge variant="default" className="gap-1">
          <ShieldCheck className="size-3" />
          Admin
        </Badge>
      )}
      {user.blocked && <Badge variant="destructive">Blocked</Badge>}
      {user.confirmed ? (
        <Badge variant="outline">Confirmed</Badge>
      ) : (
        <Badge variant="outline" className="text-text-faint">
          Unconfirmed
        </Badge>
      )}
    </div>
  );
}

function SkeletonRows() {
  return (
    <>
      {Array.from({ length: 8 }).map((_, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static placeholder rows
        <TableRow key={index}>
          <TableCell>
            <Skeleton className="h-4 w-28" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-4 w-40" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-4 w-16" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-4 w-24" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-5 w-32" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-4 w-20" />
          </TableCell>
          <TableCell>
            <Skeleton className="ml-auto size-8" />
          </TableCell>
        </TableRow>
      ))}
    </>
  );
}

function StateRow({
  icon: Icon,
  title,
  description,
}: {
  icon: typeof Users;
  title: string;
  description: string;
}) {
  return (
    <TableRow className="hover:bg-transparent">
      <TableCell colSpan={COLUMN_COUNT} className="h-48 text-center">
        <div className="flex flex-col items-center justify-center gap-2 text-text-mute">
          <Icon className="size-8 text-text-faint" />
          <p className="text-body-med text-text-hi">{title}</p>
          <p className="text-caption">{description}</p>
        </div>
      </TableCell>
    </TableRow>
  );
}

export function UsersPage() {
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const [resetTarget, setResetTarget] = useState<AdminUser | null>(null);

  const debouncedSearch = useDebounced(search.trim());

  useEffect(() => {
    setPage(1);
  }, [debouncedSearch]);

  const query = useMemo(
    () => ({ q: debouncedSearch || undefined, page }),
    [debouncedSearch, page],
  );
  const { data, isLoading, isError, error, isFetching } = useUsers(query);

  const users = data?.data ?? [];
  const meta = data?.meta;
  const pageCount = meta?.pageCount ?? 0;
  const total = meta?.total ?? 0;
  const showEmpty = !isLoading && !isError && users.length === 0;

  return (
    <div className="flex flex-col gap-6">
      <header className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-h1 text-text-hi">Users</h1>
          <p className="text-body text-text-mute">
            Accounts bound to Yggdrasil. Search, manage, and create users.
          </p>
        </div>
        <CreateUserDialog />
      </header>

      <div className="relative max-w-sm">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-text-faint" />
        <Input
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder="Search by username or email"
          aria-label="Search users"
          className="pl-9"
        />
      </div>

      <div className="rounded-lg border border-edge bg-card">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead>Username</TableHead>
              <TableHead>Email</TableHead>
              <TableHead>Origin</TableHead>
              <TableHead>Profile UUID</TableHead>
              <TableHead>Flags</TableHead>
              <TableHead>Created</TableHead>
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {isLoading && <SkeletonRows />}

            {isError && (
              <StateRow
                icon={AlertCircle}
                title="Could not load users"
                description={errorMessage(error, "Please try again.")}
              />
            )}

            {showEmpty && (
              <StateRow
                icon={debouncedSearch ? UserX : Users}
                title={debouncedSearch ? "No matches" : "No users yet"}
                description={
                  debouncedSearch
                    ? `Nothing matched "${debouncedSearch}".`
                    : "Create the first Yggdrasil-bound account to get started."
                }
              />
            )}

            {!isLoading &&
              !isError &&
              users.map((user) => (
                <TableRow key={user.id}>
                  <TableCell className="font-medium text-text-hi">
                    {user.username}
                  </TableCell>
                  <TableCell className="text-text-mute">
                    {user.email ?? "—"}
                  </TableCell>
                  <TableCell>
                    <Badge variant={originVariant(user.origin)}>
                      {user.origin}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    <code
                      className="font-mono text-caption text-text-mute"
                      title={user.profileUuid ?? user.minecraftUuid ?? undefined}
                    >
                      {shortUuid(user.profileUuid ?? user.minecraftUuid)}
                    </code>
                  </TableCell>
                  <TableCell>
                    <UserFlags user={user} />
                  </TableCell>
                  <TableCell className="text-text-mute">
                    {formatDate(user.createdAt)}
                  </TableCell>
                  <TableCell className="text-right">
                    <UserRowActions
                      user={user}
                      onResetPassword={setResetTarget}
                    />
                  </TableCell>
                </TableRow>
              ))}
          </TableBody>
        </Table>
      </div>

      <footer className="flex items-center justify-between gap-4">
        <p className="text-caption text-text-faint">
          {isLoading
            ? "Loading…"
            : total === 0
              ? "No users"
              : `${total.toLocaleString()} ${total === 1 ? "user" : "users"}`}
          {isFetching && !isLoading && " · refreshing…"}
        </p>
        <div className="flex items-center gap-2">
          <span className="text-caption text-text-mute">
            Page {meta?.page ?? page} of {Math.max(pageCount, 1)}
          </span>
          <Button
            variant="outline"
            size="icon"
            aria-label="Previous page"
            disabled={page <= 1 || isLoading}
            onClick={() => setPage((value) => Math.max(1, value - 1))}
          >
            <ChevronLeft className="size-4" />
          </Button>
          <Button
            variant="outline"
            size="icon"
            aria-label="Next page"
            disabled={page >= pageCount || isLoading}
            onClick={() => setPage((value) => value + 1)}
          >
            <ChevronRight className="size-4" />
          </Button>
        </div>
      </footer>

      <ResetPasswordDialog
        user={resetTarget}
        onOpenChange={(open) => !open && setResetTarget(null)}
      />
    </div>
  );
}
