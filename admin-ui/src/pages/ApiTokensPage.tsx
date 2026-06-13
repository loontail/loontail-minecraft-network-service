import {
  AlertTriangle,
  Check,
  Copy,
  KeyRound,
  Loader2,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react";
import type * as React from "react";
import { useState } from "react";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/shared/ConfirmDialog";
import { EmptyState } from "@/components/shared/EmptyState";
import { PageHeader } from "@/components/shared/PageHeader";
import { TableSkeleton } from "@/components/shared/TableSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useApiTokens,
  useCreateApiToken,
  useDeleteApiToken,
  useUpdateApiToken,
} from "@/features/apiTokens/api";
import { formatDateTime, formatRelative } from "@/shared/lib/format";
import type { ApiToken, CreatedApiToken } from "@/shared/types";

import { ScopeEditor, WILDCARD } from "./apiTokens/ScopeEditor";

function ScopeBadges({ scopes }: { scopes: string[] }) {
  if (scopes.length === 0) {
    return <span className="text-text-faint">Unscoped</span>;
  }
  if (scopes.includes(WILDCARD)) {
    return <Badge>Full access</Badge>;
  }
  return (
    <div className="flex flex-wrap gap-1">
      {scopes.map((scope) => (
        <Badge key={scope} variant="outline">
          {scope}
        </Badge>
      ))}
    </div>
  );
}

function RevealedToken({ token }: { token: CreatedApiToken }) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(token.token);
      setCopied(true);
      toast.success("Token copied to clipboard");
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Could not copy — select and copy the value manually");
    }
  }

  return (
    <div className="space-y-4">
      <div className="flex items-start gap-3 rounded-md border border-edge-md bg-surface-2 p-3">
        <AlertTriangle className="mt-0.5 size-4 shrink-0 text-text-hi" />
        <p className="text-body text-text">
          Save this token now. It is shown only once and cannot be recovered
          after you close this dialog.
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="raw-token">Token</Label>
        <div className="flex items-center gap-2">
          <Input
            id="raw-token"
            readOnly
            value={token.token}
            data-testid="raw-token"
            className="font-mono text-caption"
            onFocus={(event) => event.currentTarget.select()}
          />
          <Button
            type="button"
            variant="outline"
            size="icon"
            onClick={copy}
            aria-label="Copy token"
          >
            {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
          </Button>
        </div>
      </div>
    </div>
  );
}

function CreateTokenDialog() {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [scopes, setScopes] = useState<string[]>([]);
  const [created, setCreated] = useState<CreatedApiToken | null>(null);
  const createToken = useCreateApiToken();

  function reset() {
    setName("");
    setScopes([]);
    setCreated(null);
    createToken.reset();
  }

  function handleOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      reset();
    }
  }

  function handleSubmit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!name.trim()) {
      return;
    }
    createToken.mutate(
      { name: name.trim(), scopes },
      { onSuccess: (token) => setCreated(token) },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Button onClick={() => setOpen(true)}>
        <Plus className="size-4" />
        New token
      </Button>
      <DialogContent>
        {created ? (
          <>
            <DialogHeader>
              <DialogTitle>Token created</DialogTitle>
              <DialogDescription>
                Copy “{created.name}” somewhere safe before continuing.
              </DialogDescription>
            </DialogHeader>
            <RevealedToken token={created} />
            <DialogFooter>
              <Button onClick={() => handleOpenChange(false)}>Done</Button>
            </DialogFooter>
          </>
        ) : (
          <form onSubmit={handleSubmit} className="flex flex-col gap-5">
            <DialogHeader>
              <DialogTitle>Create API token</DialogTitle>
              <DialogDescription>
                Tokens authenticate launcher catalog and manifest reads.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-1.5">
              <Label htmlFor="token-name">Name</Label>
              <Input
                id="token-name"
                value={name}
                autoFocus
                placeholder="Launcher production"
                onChange={(event) => setName(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label>Scopes</Label>
              <ScopeEditor value={scopes} onChange={setScopes} />
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => handleOpenChange(false)}
                disabled={createToken.isPending}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={!name.trim() || createToken.isPending}
              >
                {createToken.isPending ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : null}
                Create token
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}

function EditTokenDialog({
  token,
  open,
  onOpenChange,
}: {
  token: ApiToken;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState(token.name);
  const [scopes, setScopes] = useState<string[]>(token.scopes);
  const updateToken = useUpdateApiToken();

  function handleOpenChange(next: boolean) {
    if (next) {
      setName(token.name);
      setScopes(token.scopes);
    }
    onOpenChange(next);
  }

  function handleSubmit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!name.trim()) {
      return;
    }
    updateToken.mutate(
      { id: token.id, name: name.trim(), scopes },
      { onSuccess: () => onOpenChange(false) },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <form onSubmit={handleSubmit} className="flex flex-col gap-5">
          <DialogHeader>
            <DialogTitle>Token settings</DialogTitle>
            <DialogDescription>
              Rename “{token.name}” or change which scopes it grants. The secret
              value is not affected.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="edit-token-name">Name</Label>
            <Input
              id="edit-token-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label>Scopes</Label>
            <ScopeEditor value={scopes} onChange={setScopes} />
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={updateToken.isPending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={!name.trim() || updateToken.isPending}
            >
              {updateToken.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Save changes
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function TokenRow({ token }: { token: ApiToken }) {
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const deleteToken = useDeleteApiToken();

  return (
    <TableRow>
      <TableCell className="font-medium text-text-hi">{token.name}</TableCell>
      <TableCell>
        <ScopeBadges scopes={token.scopes} />
      </TableCell>
      <TableCell className="text-text-mute">
        {token.lastUsedAt ? formatRelative(token.lastUsedAt) : "Never"}
      </TableCell>
      <TableCell className="text-text-mute">
        {formatDateTime(token.createdAt)}
      </TableCell>
      <TableCell className="text-right">
        <div className="flex items-center justify-end gap-1">
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Edit ${token.name}`}
            onClick={() => setEditOpen(true)}
          >
            <Pencil className="size-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            aria-label={`Delete ${token.name}`}
            onClick={() => setConfirmOpen(true)}
          >
            <Trash2 className="size-4" />
          </Button>
        </div>
        <EditTokenDialog
          token={token}
          open={editOpen}
          onOpenChange={setEditOpen}
        />
        <ConfirmDialog
          open={confirmOpen}
          onOpenChange={setConfirmOpen}
          title={`Delete “${token.name}”?`}
          description="Any launcher using this token will immediately lose access. This cannot be undone."
          confirmLabel="Delete token"
          destructive
          pending={deleteToken.isPending}
          onConfirm={() =>
            deleteToken.mutate(token.id, {
              onSuccess: () => setConfirmOpen(false),
            })
          }
        />
      </TableCell>
    </TableRow>
  );
}

export function ApiTokensPage() {
  const tokens = useApiTokens();
  const rows = tokens.data ?? [];

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="API tokens"
        description="Long-lived tokens that authenticate launcher catalog and manifest reads."
        actions={<CreateTokenDialog />}
      />

      <div className="rounded-lg border border-edge bg-card">
        {tokens.isLoading ? (
          <div className="p-4">
            <TableSkeleton rows={4} columns={5} />
          </div>
        ) : tokens.isError ? (
          <div className="p-6">
            <EmptyState
              icon={KeyRound}
              title="Could not load tokens"
              description="Retry or check the service is reachable."
              action={
                <Button variant="outline" onClick={() => tokens.refetch()}>
                  Retry
                </Button>
              }
            />
          </div>
        ) : rows.length === 0 ? (
          <div className="p-6">
            <EmptyState
              icon={KeyRound}
              title="No API tokens yet"
              description="Create a token to let a launcher read the catalog and bundle manifests."
            />
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow className="hover:bg-transparent">
                <TableHead>Name</TableHead>
                <TableHead>Scopes</TableHead>
                <TableHead>Last used</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((token) => (
                <TokenRow key={token.id} token={token} />
              ))}
            </TableBody>
          </Table>
        )}
      </div>
    </div>
  );
}
