import {
  AlertCircle,
  Search,
  ShieldCheck,
  UserX,
  Users,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { PageHeader } from "@/components/shared/PageHeader";
import { TablePager } from "@/components/shared/TablePager";
import {
  TableSkeletonRows,
  TableStateRow,
} from "@/components/shared/TableStates";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
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
import { formatDate, shortUuid } from "@/shared/lib/format";
import { useDebounced } from "@/shared/lib/useDebounced";
import type { AdminUser } from "@/shared/types";

import { CreateUserDialog } from "./users/CreateUserDialog";
import { ResetPasswordDialog } from "./users/ResetPasswordDialog";
import { UserRowActions } from "./users/UserRowActions";

const COLUMN_COUNT = 7;

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
      <PageHeader
        title="Users"
        description="Accounts bound to Yggdrasil. Search, manage, and create users."
        actions={<CreateUserDialog />}
      />

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
            {isLoading && <TableSkeletonRows columns={COLUMN_COUNT} />}

            {isError && (
              <TableStateRow
                columns={COLUMN_COUNT}
                icon={AlertCircle}
                title="Could not load users"
                description={errorMessage(error, "Please try again.")}
              />
            )}

            {showEmpty && (
              <TableStateRow
                columns={COLUMN_COUNT}
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
                    <Badge variant="secondary">{user.origin}</Badge>
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

      <TablePager
        page={meta?.page ?? page}
        pageCount={pageCount}
        total={total}
        isLoading={isLoading}
        isFetching={isFetching}
        noun={["user", "users"]}
        onPrev={() => setPage((value) => Math.max(1, value - 1))}
        onNext={() => setPage((value) => value + 1)}
      />

      <ResetPasswordDialog
        user={resetTarget}
        onOpenChange={(open) => !open && setResetTarget(null)}
      />
    </div>
  );
}
