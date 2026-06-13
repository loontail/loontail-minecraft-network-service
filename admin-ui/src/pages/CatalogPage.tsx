import {
  Hash,
  Loader2,
  MonitorSmartphone,
  Pencil,
  Plus,
  Server as ServerIcon,
  Trash2,
} from "lucide-react";
import type * as React from "react";
import { useState } from "react";

import { ConfirmDialog } from "@/components/shared/ConfirmDialog";
import { EmptyState } from "@/components/shared/EmptyState";
import { PageHeader } from "@/components/shared/PageHeader";
import {
  type SectionTab,
  SectionTabs,
} from "@/components/shared/SectionTabs";
import { TableSkeleton } from "@/components/shared/TableSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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
  useClients,
  useCreateClient,
  useCreateKeyword,
  useCreateServer,
  useDeleteClient,
  useDeleteKeyword,
  useDeleteServer,
  useKeywords,
  usePublishClient,
  usePublishKeyword,
  usePublishServer,
  useServers,
  useUpdateClient,
  useUpdateServer,
} from "@/features/catalog/api";
import type { Client, Keyword, Server } from "@/shared/types";

type Section = "clients" | "keywords" | "servers";

const TABS: SectionTab<Section>[] = [
  { value: "clients", label: "Clients", icon: MonitorSmartphone },
  { value: "keywords", label: "Keywords", icon: Hash },
  { value: "servers", label: "Servers", icon: ServerIcon },
];

function PublishBadge({ publishedAt }: { publishedAt?: string | null }) {
  return publishedAt ? (
    <Badge variant="outline">Published</Badge>
  ) : (
    <Badge variant="secondary">Draft</Badge>
  );
}

// --- Clients ---------------------------------------------------------------

interface ClientFormState {
  slug: string;
  title: string;
  shortDescription: string;
  description: string;
  available: boolean;
  minecraftVersion: string;
  fabricVersion: string;
  forgeVersion: string;
  runtimeVersion: string;
  bundleSlug: string;
}

function emptyClientForm(): ClientFormState {
  return {
    slug: "",
    title: "",
    shortDescription: "",
    description: "",
    available: false,
    minecraftVersion: "",
    fabricVersion: "",
    forgeVersion: "",
    runtimeVersion: "",
    bundleSlug: "",
  };
}

function clientToForm(client: Client): ClientFormState {
  return {
    slug: client.slug,
    title: client.title,
    shortDescription: client.shortDescription ?? "",
    description: client.description ?? "",
    available: client.available,
    minecraftVersion: client.minecraftVersion ?? "",
    fabricVersion: client.fabricVersion ?? "",
    forgeVersion: client.forgeVersion ?? "",
    runtimeVersion: client.runtimeVersion ?? "",
    bundleSlug: client.bundleSlug ?? "",
  };
}

function nullable(value: string): string | null {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}

function ClientDialog({
  client,
  open,
  onOpenChange,
}: {
  client: Client | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [form, setForm] = useState<ClientFormState>(emptyClientForm);
  const createClient = useCreateClient();
  const updateClient = useUpdateClient();
  const pending = createClient.isPending || updateClient.isPending;
  const editing = Boolean(client);

  function handleOpenChange(next: boolean) {
    if (next) {
      setForm(client ? clientToForm(client) : emptyClientForm());
    }
    onOpenChange(next);
  }

  function field<K extends keyof ClientFormState>(
    key: K,
    value: ClientFormState[K],
  ) {
    setForm((prev) => ({ ...prev, [key]: value }));
  }

  function handleSubmit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!form.slug.trim() || !form.title.trim()) {
      return;
    }
    const payload = {
      slug: form.slug.trim(),
      available: form.available,
      minecraftVersion: nullable(form.minecraftVersion),
      fabricVersion: nullable(form.fabricVersion),
      forgeVersion: nullable(form.forgeVersion),
      runtimeVersion: nullable(form.runtimeVersion),
      bundleSlug: nullable(form.bundleSlug),
      locales: [
        {
          locale: "en",
          title: form.title.trim(),
          shortDescription: nullable(form.shortDescription),
          description: nullable(form.description),
        },
      ],
    };
    if (client) {
      updateClient.mutate(
        { id: client.documentId, ...payload },
        { onSuccess: () => onOpenChange(false) },
      );
    } else {
      createClient.mutate(payload, { onSuccess: () => onOpenChange(false) });
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>{editing ? "Edit client" : "New client"}</DialogTitle>
            <DialogDescription>
              Clients are the modpacks shown in the launcher catalog.
            </DialogDescription>
          </DialogHeader>
          <div className="mt-4 grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <Label htmlFor="client-slug">Slug</Label>
              <Input
                id="client-slug"
                value={form.slug}
                placeholder="all-the-mods-9"
                onChange={(event) => field("slug", event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="client-title">Title</Label>
              <Input
                id="client-title"
                value={form.title}
                placeholder="All the Mods 9"
                onChange={(event) => field("title", event.target.value)}
              />
            </div>
            <div className="col-span-2 space-y-1.5">
              <Label htmlFor="client-short">Short description</Label>
              <Input
                id="client-short"
                value={form.shortDescription}
                onChange={(event) =>
                  field("shortDescription", event.target.value)
                }
              />
            </div>
            <div className="col-span-2 space-y-1.5">
              <Label htmlFor="client-desc">Description</Label>
              <textarea
                id="client-desc"
                rows={3}
                value={form.description}
                onChange={(event) => field("description", event.target.value)}
                className="flex w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="client-mc">Minecraft version</Label>
              <Input
                id="client-mc"
                value={form.minecraftVersion}
                placeholder="1.21.4"
                onChange={(event) =>
                  field("minecraftVersion", event.target.value)
                }
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="client-runtime">Runtime version</Label>
              <Input
                id="client-runtime"
                value={form.runtimeVersion}
                placeholder="21"
                onChange={(event) =>
                  field("runtimeVersion", event.target.value)
                }
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="client-fabric">Fabric version</Label>
              <Input
                id="client-fabric"
                value={form.fabricVersion}
                onChange={(event) =>
                  field("fabricVersion", event.target.value)
                }
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="client-forge">Forge version</Label>
              <Input
                id="client-forge"
                value={form.forgeVersion}
                onChange={(event) => field("forgeVersion", event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="client-bundle">Bundle slug</Label>
              <Input
                id="client-bundle"
                value={form.bundleSlug}
                placeholder="atm9-overlay"
                onChange={(event) => field("bundleSlug", event.target.value)}
              />
            </div>
            <div className="flex items-end">
              <label className="flex items-center gap-2 text-body text-text">
                <input
                  type="checkbox"
                  checked={form.available}
                  onChange={(event) =>
                    field("available", event.target.checked)
                  }
                  className="size-4 accent-cta"
                />
                Available
              </label>
            </div>
          </div>
          <DialogFooter className="mt-6">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={pending || !form.slug.trim() || !form.title.trim()}
            >
              {pending ? <Loader2 className="size-4 animate-spin" /> : null}
              {editing ? "Save changes" : "Create client"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ClientsSection() {
  const clients = useClients();
  const publish = usePublishClient();
  const deleteClient = useDeleteClient();
  const [dialogClient, setDialogClient] = useState<Client | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [confirm, setConfirm] = useState<Client | null>(null);
  const rows = clients.data ?? [];

  function openCreate() {
    setDialogClient(null);
    setDialogOpen(true);
  }

  function openEdit(client: Client) {
    setDialogClient(client);
    setDialogOpen(true);
  }

  return (
    <Card>
      <CardContent className="space-y-4">
        <div className="flex justify-end">
          <Button onClick={openCreate}>
            <Plus className="size-4" />
            New client
          </Button>
        </div>
        {clients.isLoading ? (
          <TableSkeleton columns={5} />
        ) : rows.length === 0 ? (
          <EmptyState
            icon={MonitorSmartphone}
            title="No clients yet"
            description="Create a client to surface a modpack in the launcher."
            action={
              <Button variant="outline" onClick={openCreate}>
                <Plus className="size-4" />
                New client
              </Button>
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Title</TableHead>
                <TableHead>Slug</TableHead>
                <TableHead>Minecraft</TableHead>
                <TableHead>State</TableHead>
                <TableHead className="w-px text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((client) => (
                <TableRow key={client.documentId}>
                  <TableCell className="font-medium text-text-hi">
                    {client.title}
                  </TableCell>
                  <TableCell className="text-text-mute">
                    {client.slug}
                  </TableCell>
                  <TableCell className="text-text-mute">
                    {client.minecraftVersion ?? "—"}
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      <PublishBadge publishedAt={client.publishedAt} />
                      {client.available ? null : (
                        <Badge variant="secondary">Hidden</Badge>
                      )}
                    </div>
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={publish.isPending}
                        onClick={() =>
                          publish.mutate({
                            id: client.documentId,
                            publish: !client.publishedAt,
                          })
                        }
                      >
                        {client.publishedAt ? "Unpublish" : "Publish"}
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`Edit ${client.title}`}
                        onClick={() => openEdit(client)}
                      >
                        <Pencil className="size-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`Delete ${client.title}`}
                        onClick={() => setConfirm(client)}
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>

      <ClientDialog
        client={dialogClient}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(next) => !next && setConfirm(null)}
        title={`Delete “${confirm?.title}”?`}
        description="This removes the client and its localized text. This cannot be undone."
        confirmLabel="Delete client"
        destructive
        pending={deleteClient.isPending}
        onConfirm={() => {
          if (!confirm) {
            return;
          }
          deleteClient.mutate(confirm.documentId, {
            onSuccess: () => setConfirm(null),
          });
        }}
      />
    </Card>
  );
}

// --- Keywords --------------------------------------------------------------

function KeywordDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [slug, setSlug] = useState("");
  const [title, setTitle] = useState("");
  const createKeyword = useCreateKeyword();

  function handleOpenChange(next: boolean) {
    if (next) {
      setSlug("");
      setTitle("");
    }
    onOpenChange(next);
  }

  function handleSubmit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!slug.trim() || !title.trim()) {
      return;
    }
    createKeyword.mutate(
      {
        slug: slug.trim(),
        locales: [{ locale: "en", title: title.trim() }],
      },
      { onSuccess: () => onOpenChange(false) },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>New keyword</DialogTitle>
            <DialogDescription>
              Keywords tag clients for filtering in the launcher.
            </DialogDescription>
          </DialogHeader>
          <div className="mt-4 space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="keyword-slug">Slug</Label>
              <Input
                id="keyword-slug"
                value={slug}
                placeholder="tech"
                onChange={(event) => setSlug(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="keyword-title">Title</Label>
              <Input
                id="keyword-title"
                value={title}
                placeholder="Tech"
                onChange={(event) => setTitle(event.target.value)}
              />
            </div>
          </div>
          <DialogFooter className="mt-6">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={createKeyword.isPending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                createKeyword.isPending || !slug.trim() || !title.trim()
              }
            >
              {createKeyword.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Create keyword
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function KeywordsSection() {
  const keywords = useKeywords();
  const publish = usePublishKeyword();
  const deleteKeyword = useDeleteKeyword();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [confirm, setConfirm] = useState<Keyword | null>(null);
  const rows = keywords.data ?? [];

  return (
    <Card>
      <CardContent className="space-y-4">
        <div className="flex justify-end">
          <Button onClick={() => setDialogOpen(true)}>
            <Plus className="size-4" />
            New keyword
          </Button>
        </div>
        {keywords.isLoading ? (
          <TableSkeleton columns={3} />
        ) : rows.length === 0 ? (
          <EmptyState
            icon={Hash}
            title="No keywords yet"
            description="Add keywords to let players filter the catalog."
            action={
              <Button variant="outline" onClick={() => setDialogOpen(true)}>
                <Plus className="size-4" />
                New keyword
              </Button>
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Title</TableHead>
                <TableHead>State</TableHead>
                <TableHead className="w-px text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((keyword) => (
                <TableRow key={keyword.documentId}>
                  <TableCell className="font-medium text-text-hi">
                    {keyword.title}
                  </TableCell>
                  <TableCell>
                    <PublishBadge publishedAt={keyword.publishedAt} />
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={publish.isPending}
                        onClick={() =>
                          publish.mutate({
                            id: keyword.documentId,
                            publish: !keyword.publishedAt,
                          })
                        }
                      >
                        {keyword.publishedAt ? "Unpublish" : "Publish"}
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`Delete ${keyword.title}`}
                        onClick={() => setConfirm(keyword)}
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>

      <KeywordDialog open={dialogOpen} onOpenChange={setDialogOpen} />
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(next) => !next && setConfirm(null)}
        title={`Delete “${confirm?.title}”?`}
        confirmLabel="Delete keyword"
        destructive
        pending={deleteKeyword.isPending}
        onConfirm={() => {
          if (!confirm) {
            return;
          }
          deleteKeyword.mutate(confirm.documentId, {
            onSuccess: () => setConfirm(null),
          });
        }}
      />
    </Card>
  );
}

// --- Servers ---------------------------------------------------------------

function ServerDialog({
  server,
  open,
  onOpenChange,
}: {
  server: Server | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [slug, setSlug] = useState("");
  const [name, setName] = useState("");
  const [address, setAddress] = useState("");
  const createServer = useCreateServer();
  const updateServer = useUpdateServer();
  const pending = createServer.isPending || updateServer.isPending;
  const editing = Boolean(server);

  function handleOpenChange(next: boolean) {
    if (next) {
      setSlug("");
      setName(server?.name ?? "");
      setAddress(server?.address ?? "");
    }
    onOpenChange(next);
  }

  function handleSubmit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!slug.trim() || !address.trim()) {
      return;
    }
    const payload = {
      slug: slug.trim(),
      name: nullable(name),
      address: address.trim(),
    };
    if (server) {
      updateServer.mutate(
        { id: server.documentId, ...payload },
        { onSuccess: () => onOpenChange(false) },
      );
    } else {
      createServer.mutate(payload, { onSuccess: () => onOpenChange(false) });
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>{editing ? "Edit server" : "New server"}</DialogTitle>
            <DialogDescription>
              Servers are the multiplayer entries attached to clients.
            </DialogDescription>
          </DialogHeader>
          <div className="mt-4 space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="server-slug">Slug</Label>
              <Input
                id="server-slug"
                value={slug}
                placeholder="survival-main"
                onChange={(event) => setSlug(event.target.value)}
              />
              {editing ? (
                <p className="text-caption text-text-mute">
                  Re-enter the slug to save changes; it is not returned by reads.
                </p>
              ) : null}
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="server-name">Name</Label>
              <Input
                id="server-name"
                value={name}
                placeholder="Survival"
                onChange={(event) => setName(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="server-address">Address</Label>
              <Input
                id="server-address"
                value={address}
                placeholder="play.example.net"
                onChange={(event) => setAddress(event.target.value)}
              />
            </div>
          </div>
          <DialogFooter className="mt-6">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={pending || !slug.trim() || !address.trim()}
            >
              {pending ? <Loader2 className="size-4 animate-spin" /> : null}
              {editing ? "Save changes" : "Create server"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ServersSection() {
  const servers = useServers();
  const publish = usePublishServer();
  const deleteServer = useDeleteServer();
  const [dialogServer, setDialogServer] = useState<Server | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [confirm, setConfirm] = useState<Server | null>(null);
  const rows = servers.data ?? [];

  function openCreate() {
    setDialogServer(null);
    setDialogOpen(true);
  }

  function openEdit(server: Server) {
    setDialogServer(server);
    setDialogOpen(true);
  }

  return (
    <Card>
      <CardContent className="space-y-4">
        <div className="flex justify-end">
          <Button onClick={openCreate}>
            <Plus className="size-4" />
            New server
          </Button>
        </div>
        {servers.isLoading ? (
          <TableSkeleton columns={4} />
        ) : rows.length === 0 ? (
          <EmptyState
            icon={ServerIcon}
            title="No servers yet"
            description="Add a server to attach it to a client."
            action={
              <Button variant="outline" onClick={openCreate}>
                <Plus className="size-4" />
                New server
              </Button>
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Address</TableHead>
                <TableHead>State</TableHead>
                <TableHead className="w-px text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((server) => (
                <TableRow key={server.documentId}>
                  <TableCell className="font-medium text-text-hi">
                    {server.name ?? "—"}
                  </TableCell>
                  <TableCell className="text-text-mute">
                    {server.address}
                  </TableCell>
                  <TableCell>
                    <PublishBadge publishedAt={server.publishedAt} />
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={publish.isPending}
                        onClick={() =>
                          publish.mutate({
                            id: server.documentId,
                            publish: !server.publishedAt,
                          })
                        }
                      >
                        {server.publishedAt ? "Unpublish" : "Publish"}
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`Edit ${server.name ?? server.address}`}
                        onClick={() => openEdit(server)}
                      >
                        <Pencil className="size-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`Delete ${server.name ?? server.address}`}
                        onClick={() => setConfirm(server)}
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>

      <ServerDialog
        server={dialogServer}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(next) => !next && setConfirm(null)}
        title={`Delete “${confirm?.name ?? confirm?.address}”?`}
        confirmLabel="Delete server"
        destructive
        pending={deleteServer.isPending}
        onConfirm={() => {
          if (!confirm) {
            return;
          }
          deleteServer.mutate(confirm.documentId, {
            onSuccess: () => setConfirm(null),
          });
        }}
      />
    </Card>
  );
}

export function CatalogPage() {
  const [section, setSection] = useState<Section>("clients");

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Catalog"
        description="Clients, keywords, and servers shown in the launcher."
      />
      <SectionTabs tabs={TABS} value={section} onChange={setSection} />
      {section === "clients" ? (
        <ClientsSection />
      ) : section === "keywords" ? (
        <KeywordsSection />
      ) : (
        <ServersSection />
      )}
    </div>
  );
}
