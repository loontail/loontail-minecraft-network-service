import {
  Ban,
  KeyRound,
  MoreHorizontal,
  ShieldOff,
  Trash2,
  Undo2,
} from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  useBlockUser,
  useDeleteUser,
  useRevokeTokens,
  useUnblockUser,
} from "@/features/users/api";
import type { AdminUser } from "@/shared/types";

import { ConfirmDialog } from "./ConfirmDialog";

type PendingAction = "block" | "unblock" | "revoke" | "delete" | null;

export interface UserRowActionsProps {
  user: AdminUser;
  onResetPassword: (user: AdminUser) => void;
}

export function UserRowActions({ user, onResetPassword }: UserRowActionsProps) {
  const [action, setAction] = useState<PendingAction>(null);

  const blockUser = useBlockUser();
  const unblockUser = useUnblockUser();
  const revokeTokens = useRevokeTokens();
  const deleteUser = useDeleteUser();

  function close() {
    setAction(null);
  }

  async function run(mutate: () => Promise<unknown>) {
    try {
      await mutate();
      close();
    } catch {
      // The hooks surface a toast; keep the dialog open so the admin can retry.
    }
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Actions for ${user.username}`}
          >
            <MoreHorizontal className="size-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-48">
          <DropdownMenuLabel>Actions</DropdownMenuLabel>
          <DropdownMenuSeparator />
          {user.blocked ? (
            <DropdownMenuItem onSelect={() => setAction("unblock")}>
              <Undo2 className="size-4" />
              Unblock
            </DropdownMenuItem>
          ) : (
            <DropdownMenuItem onSelect={() => setAction("block")}>
              <Ban className="size-4" />
              Block
            </DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={() => onResetPassword(user)}>
            <KeyRound className="size-4" />
            Reset password
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => setAction("revoke")}>
            <ShieldOff className="size-4" />
            Revoke tokens
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            variant="destructive"
            onSelect={() => setAction("delete")}
          >
            <Trash2 className="size-4" />
            Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <ConfirmDialog
        open={action === "block"}
        onOpenChange={(next) => !next && close()}
        title="Block user"
        description={
          <>
            Block "{user.username}"? They will be unable to authenticate until
            unblocked.
          </>
        }
        confirmLabel="Block"
        destructive
        pending={blockUser.isPending}
        onConfirm={() => run(() => blockUser.mutateAsync(user.id))}
      />

      <ConfirmDialog
        open={action === "unblock"}
        onOpenChange={(next) => !next && close()}
        title="Unblock user"
        description={<>Restore access for "{user.username}"?</>}
        confirmLabel="Unblock"
        pending={unblockUser.isPending}
        onConfirm={() => run(() => unblockUser.mutateAsync(user.id))}
      />

      <ConfirmDialog
        open={action === "revoke"}
        onOpenChange={(next) => !next && close()}
        title="Revoke tokens"
        description={
          <>
            Revoke every network and admin session plus all Yggdrasil tokens for
            "{user.username}"? They will be signed out everywhere.
          </>
        }
        confirmLabel="Revoke tokens"
        destructive
        pending={revokeTokens.isPending}
        onConfirm={() => run(() => revokeTokens.mutateAsync(user.id))}
      />

      <ConfirmDialog
        open={action === "delete"}
        onOpenChange={(next) => !next && close()}
        title="Delete user"
        description={
          <>
            Permanently delete "{user.username}"? This also removes their
            sessions and tokens. This cannot be undone.
          </>
        }
        confirmLabel="Delete"
        destructive
        pending={deleteUser.isPending}
        onConfirm={() => run(() => deleteUser.mutateAsync(user.id))}
      />
    </>
  );
}
