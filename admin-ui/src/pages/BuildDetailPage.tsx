import * as SelectPrimitive from "@radix-ui/react-select";
import {
  ArrowLeft,
  CheckIcon,
  Copy,
  FolderTree,
  ImageIcon,
  Loader2,
  Package,
  Pencil,
  Plus,
  Server as ServerIcon,
  Star,
  Tags,
  X,
} from "lucide-react";
import type * as React from "react";
import { useEffect, useRef, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { toast } from "sonner";

import { EmptyState } from "@/components/shared/EmptyState";
import {
  panelId,
  type SectionTab,
  SectionTabs,
  tabId,
} from "@/components/shared/SectionTabs";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { BuildFilesTab } from "@/features/builds/BuildFilesTab";
import {
  useBuildBySlug,
  usePublishClient,
  useUpdateClient,
} from "@/features/builds/api";
import { useVersions } from "@/features/builds/useVersions";
import { useBuild } from "@/features/bundles/api";
import {
  useAttachKeyword,
  useAttachServer,
  useCreateKeyword,
  useCreateServer,
  useDetachKeyword,
  useDetachServer,
  useKeywords,
  useServers,
} from "@/features/catalog/api";
import { ClientMediaSection } from "@/features/catalog/components/ClientMediaSection";
import { cn } from "@/shared/lib/cn";
import type { ClientAdmin } from "@/shared/types";

type Tab = "details" | "media" | "files" | "servers-tags";

const TABS: SectionTab<Tab>[] = [
  { value: "details", label: "Details", icon: Pencil },
  { value: "media", label: "Media", icon: ImageIcon },
  { value: "files", label: "Files", icon: FolderTree },
  { value: "servers-tags", label: "Servers & Tags", icon: Tags },
];

// Radix Select reserves "" for the placeholder, so a clearable select needs a sentinel value.
const NONE = "__none__";

// Picking this swaps the Java select for a free-text input (target a runtime component not yet catalogued).
const CUSTOM = "__custom__";

// The Star marker sits OUTSIDE `ItemText` so Radix never clones it into the trigger's `SelectValue`.
// `value` is the stored string (version / component); `label` is the visible text (defaults to value).
function VersionSelectItem({
  value,
  label,
  recommended,
}: {
  value: string;
  label?: string;
  recommended?: boolean;
}) {
  const text = label ?? value;
  return (
    <SelectPrimitive.Item
      value={value}
      textValue={text}
      className={cn(
        "focus:bg-accent focus:text-accent-foreground relative flex w-full cursor-default items-center gap-2 rounded-sm py-1.5 pr-8 pl-2 text-sm outline-hidden select-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
      )}
    >
      <span className="absolute right-2 flex size-3.5 items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <CheckIcon className="size-4" />
        </SelectPrimitive.ItemIndicator>
      </span>
      <span className="flex items-center gap-1.5">
        <SelectPrimitive.ItemText>{text}</SelectPrimitive.ItemText>
        {recommended ? (
          <Star className="size-3 text-primary" aria-label="recommended" />
        ) : null}
      </span>
    </SelectPrimitive.Item>
  );
}

interface BuildFormState {
  slug: string;
  title: string;
  shortDescription: string;
  description: string;
  available: boolean;
  minecraftVersion: string;
  runtimeVersion: string;
  fabricVersion: string;
  forgeVersion: string;
}

function buildToForm(build: ClientAdmin): BuildFormState {
  return {
    slug: build.slug,
    title: build.title,
    shortDescription: build.shortDescription ?? "",
    description: build.description ?? "",
    available: build.available,
    minecraftVersion: build.minecraftVersion ?? "",
    runtimeVersion: build.runtimeVersion ?? "",
    fabricVersion: build.fabricVersion ?? "",
    forgeVersion: build.forgeVersion ?? "",
  };
}

// A token over the persisted fields; a change means the stored copy diverged and the form must re-seed.
function buildFormToken(build: ClientAdmin): string {
  return JSON.stringify(buildToForm(build));
}

function nullable(value: string): string | null {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
}

// Prepend a stored value missing from the catalog (legacy builds) so editing never drops it.
function withCurrent(options: string[], current: string): string[] {
  if (current === "" || options.includes(current)) {
    return options;
  }
  return [current, ...options];
}

function ManifestPanel({ build }: { build: ClientAdmin }) {
  const bundleSlug = build.bundle?.slug ?? null;
  // Read live bundle state: the summary on `build.bundle` is not invalidated by file uploads.
  const liveBundle = useBuild(bundleSlug ?? undefined);

  if (!bundleSlug) {
    return (
      <Card>
        <CardContent className="flex items-center gap-2 py-4 text-body text-text-mute">
          <Package className="size-4" />
          No bundle linked yet.
        </CardContent>
      </Card>
    );
  }

  const status = liveBundle.data?.status ?? build.bundle?.status ?? "draft";
  const filesCount = liveBundle.data?.filesCount ?? build.bundle?.filesCount ?? 0;
  const manifestUrl = `${window.location.origin}/api/bundle-registry/builds/${bundleSlug}/manifest`;

  async function copy() {
    await navigator.clipboard.writeText(manifestUrl);
    toast.success("Copied");
  }

  return (
    <Card>
      <CardContent className="space-y-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Label className="text-text-hi">Manifest URL</Label>
            <Badge variant="secondary">{status}</Badge>
          </div>
          <span className="text-caption text-text-faint">
            {filesCount} file{filesCount === 1 ? "" : "s"}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <Input readOnly value={manifestUrl} className="font-mono text-sm" />
          <Button
            type="button"
            variant="outline"
            size="icon"
            aria-label="Copy manifest URL"
            onClick={() => void copy()}
          >
            <Copy className="size-4" />
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function BuildDetailsTab({ build }: { build: ClientAdmin }) {
  const [form, setForm] = useState<BuildFormState>(() => buildToForm(build));
  const [javaCustom, setJavaCustom] = useState(false);
  const updateClient = useUpdateClient();
  const versions = useVersions();

  // Re-seed on a value token, not `build.id`: the key={build.id} panel only remounts on identity change.
  const seededToken = useRef(buildFormToken(build));
  useEffect(() => {
    const token = buildFormToken(build);
    if (token !== seededToken.current) {
      seededToken.current = token;
      setForm(buildToForm(build));
    }
  }, [build]);

  function field<K extends keyof BuildFormState>(
    key: K,
    value: BuildFormState[K],
  ) {
    setForm((prev) => ({ ...prev, [key]: value }));
  }

  const catalog = versions.data;
  const mcOptions = withCurrent(
    (catalog?.minecraft ?? []).map((v) => v.id),
    form.minecraftVersion,
  );
  const mcChosen = form.minecraftVersion !== "";
  const forgeForMc = mcChosen
    ? (catalog?.forge?.[form.minecraftVersion] ?? [])
    : [];
  const forgeOptions = withCurrent(forgeForMc, form.forgeVersion);
  const fabricOptions = withCurrent(catalog?.fabric ?? [], form.fabricVersion);

  // Java options carry a component value + a visible label. A stored value not in
  // the catalog (legacy major like "25", or a custom component) is surfaced first
  // labelled by itself so editing never drops it.
  const javaCatalog = catalog?.java ?? [];
  const javaOptions: { value: string; label: string }[] = javaCatalog.map(
    (o) => ({ value: o.component, label: o.label }),
  );
  if (
    form.runtimeVersion !== "" &&
    !javaOptions.some((o) => o.value === form.runtimeVersion)
  ) {
    javaOptions.unshift({
      value: form.runtimeVersion,
      label: form.runtimeVersion,
    });
  }

  // Per-MC recommended picks: drive the Star markers and seed the cascade on MC change.
  const rec = catalog?.recommended?.[form.minecraftVersion] ?? {};
  const recForge = rec.forge ?? undefined;
  const recFabric = rec.fabric ?? undefined;
  const recJava = rec.java;

  // Changing MC cascades: Forge resets to recommended if its pick is now invalid; Fabric keeps an
  // existing pick and only seeds the recommended when empty; Java seeds the recommended COMPONENT when
  // the current value is empty or no longer a catalogued component (a valid existing component stays).
  function setMinecraft(next: string) {
    setForm((prev) => {
      if (next === "") {
        return { ...prev, minecraftVersion: "", forgeVersion: "", fabricVersion: "" };
      }
      const r = catalog?.recommended?.[next] ?? {};
      const forgeList = catalog?.forge?.[next] ?? [];
      const recJavaNext = r.java ?? catalog?.java?.[0]?.component ?? "";
      const javaValid = (catalog?.java ?? []).some(
        (o) => o.component === prev.runtimeVersion,
      );
      return {
        ...prev,
        minecraftVersion: next,
        forgeVersion: forgeList.includes(prev.forgeVersion)
          ? prev.forgeVersion
          : (r.forge ?? ""),
        fabricVersion: prev.fabricVersion || (r.fabric ?? ""),
        runtimeVersion:
          prev.runtimeVersion !== "" && javaValid
            ? prev.runtimeVersion
            : recJavaNext,
      };
    });
  }

  function handleSubmit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!form.slug.trim() || !form.title.trim()) {
      return;
    }
    updateClient.mutate({
      id: build.id,
      slug: form.slug.trim(),
      available: form.available,
      minecraftVersion: nullable(form.minecraftVersion),
      runtimeVersion: nullable(form.runtimeVersion),
      fabricVersion: nullable(form.fabricVersion),
      forgeVersion: nullable(form.forgeVersion),
      // Send only the verified bundle slug (update is a full column replace); null lets it re-derive + re-link.
      bundleSlug: build.bundle?.slug ?? null,
      locales: [
        {
          locale: "en",
          title: form.title.trim(),
          shortDescription: nullable(form.shortDescription),
          description: nullable(form.description),
        },
      ],
    });
  }

  return (
    <Card>
      <CardContent>
        <form
          onSubmit={handleSubmit}
          className="grid grid-cols-1 gap-4 sm:grid-cols-2"
        >
          <div className="space-y-1.5">
            <Label htmlFor="build-slug">Slug</Label>
            <Input
              id="build-slug"
              value={form.slug}
              placeholder="all-the-mods-9"
              onChange={(event) => field("slug", event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="build-title">Title</Label>
            <Input
              id="build-title"
              value={form.title}
              placeholder="All the Mods 9"
              onChange={(event) => field("title", event.target.value)}
            />
          </div>
          <div className="space-y-1.5 sm:col-span-2">
            <Label htmlFor="build-short">Short description</Label>
            <Input
              id="build-short"
              value={form.shortDescription}
              onChange={(event) =>
                field("shortDescription", event.target.value)
              }
            />
          </div>
          <div className="space-y-1.5 sm:col-span-2">
            <Label htmlFor="build-desc">Description</Label>
            <Textarea
              id="build-desc"
              rows={3}
              value={form.description}
              onChange={(event) => field("description", event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="build-mc">Minecraft version</Label>
            <Select
              value={form.minecraftVersion === "" ? NONE : form.minecraftVersion}
              disabled={versions.isLoading}
              onValueChange={(value) =>
                setMinecraft(value === NONE ? "" : value)
              }
            >
              <SelectTrigger
                id="build-mc"
                aria-label="Minecraft version"
                className="w-full"
              >
                <SelectValue placeholder="—" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>—</SelectItem>
                {mcOptions.map((id) => (
                  <SelectItem key={id} value={id}>
                    {id}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="build-runtime">Java version</Label>
            {javaCustom ? (
              <Input
                id="build-runtime"
                value={form.runtimeVersion}
                placeholder="java-runtime-delta"
                aria-label="Java version"
                autoFocus
                onChange={(event) => field("runtimeVersion", event.target.value)}
              />
            ) : (
              <Select
                value={form.runtimeVersion === "" ? NONE : form.runtimeVersion}
                onValueChange={(value) => {
                  if (value === CUSTOM) {
                    setJavaCustom(true);
                    return;
                  }
                  field("runtimeVersion", value === NONE ? "" : value);
                }}
              >
                <SelectTrigger
                  id="build-runtime"
                  aria-label="Java version"
                  className="w-full"
                >
                  <SelectValue placeholder="—" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NONE}>—</SelectItem>
                  {javaOptions.map((option) => (
                    <VersionSelectItem
                      key={option.value}
                      value={option.value}
                      label={option.label}
                      recommended={recJava === option.value}
                    />
                  ))}
                  <SelectItem value={CUSTOM}>Custom…</SelectItem>
                </SelectContent>
              </Select>
            )}
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="build-fabric">Fabric version</Label>
            <Select
              value={form.fabricVersion === "" ? NONE : form.fabricVersion}
              disabled={!mcChosen}
              onValueChange={(value) =>
                field("fabricVersion", value === NONE ? "" : value)
              }
            >
              <SelectTrigger
                id="build-fabric"
                aria-label="Fabric version"
                className="w-full"
              >
                <SelectValue
                  placeholder={
                    mcChosen ? "—" : "Select a Minecraft version first"
                  }
                />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>—</SelectItem>
                {fabricOptions.map((version) => (
                  <VersionSelectItem
                    key={version}
                    value={version}
                    recommended={version === recFabric}
                  />
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="build-forge">Forge version</Label>
            {/* Remount on MC change: Radix fires a spurious onValueChange("") when the controlled value
                points at an item not yet mounted during the MC-scoped item-set swap. */}
            <Select
              key={`forge-${form.minecraftVersion}`}
              value={form.forgeVersion === "" ? NONE : form.forgeVersion}
              disabled={!mcChosen}
              onValueChange={(value) =>
                field("forgeVersion", value === NONE ? "" : value)
              }
            >
              <SelectTrigger
                id="build-forge"
                aria-label="Forge version"
                className="w-full"
              >
                <SelectValue
                  placeholder={
                    mcChosen ? "—" : "Select a Minecraft version first"
                  }
                />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={NONE}>—</SelectItem>
                {forgeOptions.map((version) => (
                  <VersionSelectItem
                    key={version}
                    value={version}
                    recommended={version === recForge}
                  />
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center justify-between gap-3 rounded-md border border-edge bg-surface-1 px-3 py-2.5 sm:col-span-2">
            <div className="grid gap-0.5">
              <Label htmlFor="build-available">Available</Label>
              <span className="text-caption text-text-faint">
                Whether the launcher offers this build for install.
              </span>
            </div>
            <Switch
              id="build-available"
              checked={form.available}
              onCheckedChange={(value) => field("available", value)}
              aria-label="Available"
            />
          </div>
          <div className="flex justify-end sm:col-span-2">
            <Button
              type="submit"
              disabled={
                updateClient.isPending ||
                !form.slug.trim() ||
                !form.title.trim()
              }
            >
              {updateClient.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Save changes
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}

// Shared attach-existing control for the Keywords and Servers sections; each owns its own create form.
function AddExistingPicker<T extends { id: string }>({
  available,
  pickId,
  onPick,
  onAdd,
  onToggleCreate,
  pending,
  ariaLabel,
  emptyLabel,
  selectLabel,
  renderOption,
}: {
  available: T[];
  pickId: string;
  onPick: (id: string) => void;
  onAdd: () => void;
  onToggleCreate: () => void;
  pending: boolean;
  ariaLabel: string;
  emptyLabel: string;
  selectLabel: string;
  renderOption: (item: T) => React.ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Select
        value={pickId}
        disabled={pending || available.length === 0}
        onValueChange={onPick}
      >
        <SelectTrigger aria-label={ariaLabel} className="w-56">
          <SelectValue
            placeholder={available.length === 0 ? emptyLabel : selectLabel}
          />
        </SelectTrigger>
        <SelectContent>
          {available.map((item) => (
            <SelectItem key={item.id} value={item.id}>
              {renderOption(item)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button
        type="button"
        variant="outline"
        disabled={pending || !pickId}
        onClick={onAdd}
      >
        <Plus className="size-4" />
        Add
      </Button>
      <Button
        type="button"
        variant="ghost"
        disabled={pending}
        onClick={onToggleCreate}
      >
        Create new
      </Button>
    </div>
  );
}

function KeywordsSection({ build }: { build: ClientAdmin }) {
  const allKeywords = useKeywords();
  const attach = useAttachKeyword();
  const detach = useDetachKeyword();
  const createKeyword = useCreateKeyword();

  const [pickId, setPickId] = useState("");
  const [creating, setCreating] = useState(false);
  const [slug, setSlug] = useState("");
  const [title, setTitle] = useState("");

  const attachedIds = new Set(build.keywords.map((k) => k.id));
  const available = (allKeywords.data ?? []).filter(
    (k) => !attachedIds.has(k.id),
  );
  const pending = attach.isPending || detach.isPending || createKeyword.isPending;

  function attachExisting() {
    if (!pickId) {
      return;
    }
    attach.mutate(
      { clientId: build.id, keywordId: pickId },
      { onSuccess: () => setPickId("") },
    );
  }

  function handleCreate(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!slug.trim() || !title.trim()) {
      return;
    }
    createKeyword.mutate(
      {
        slug: slug.trim(),
        locales: [{ locale: "en", title: title.trim() }],
      },
      {
        onSuccess: (result) => {
          attach.mutate({ clientId: build.id, keywordId: result.id });
          setSlug("");
          setTitle("");
          setCreating(false);
        },
      },
    );
  }

  return (
    <Card>
      <CardContent className="space-y-4">
        <div className="flex items-center gap-2">
          <Tags className="size-4 text-text-mute" />
          <Label className="text-text-hi">Keywords</Label>
        </div>

        {build.keywords.length === 0 ? (
          <p className="text-body text-text-mute">No keywords attached yet.</p>
        ) : (
          <div className="flex flex-wrap gap-2">
            {build.keywords.map((keyword) => (
              <Badge
                key={keyword.id}
                variant="secondary"
                className="gap-1.5 pr-1"
              >
                {keyword.title}
                <button
                  type="button"
                  aria-label={`Remove ${keyword.title}`}
                  disabled={pending}
                  onClick={() =>
                    detach.mutate({
                      clientId: build.id,
                      keywordId: keyword.id,
                    })
                  }
                  className="rounded-sm text-text-faint hover:text-text-hi"
                >
                  <X className="size-3" />
                </button>
              </Badge>
            ))}
          </div>
        )}

        <AddExistingPicker
          available={available}
          pickId={pickId}
          onPick={setPickId}
          onAdd={attachExisting}
          onToggleCreate={() => setCreating((prev) => !prev)}
          pending={pending}
          ariaLabel="Add keyword"
          emptyLabel="No more keywords"
          selectLabel="Select a keyword…"
          renderOption={(keyword) => keyword.title}
        />

        {creating ? (
          <form
            onSubmit={handleCreate}
            className="flex flex-wrap items-end gap-2 rounded-md border border-edge bg-surface-1 p-3"
          >
            <div className="space-y-1.5">
              <Label htmlFor="new-keyword-slug">Slug</Label>
              <Input
                id="new-keyword-slug"
                value={slug}
                placeholder="tech"
                onChange={(event) => setSlug(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="new-keyword-title">Title</Label>
              <Input
                id="new-keyword-title"
                value={title}
                placeholder="Tech"
                onChange={(event) => setTitle(event.target.value)}
              />
            </div>
            <Button
              type="submit"
              disabled={pending || !slug.trim() || !title.trim()}
            >
              {createKeyword.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Create &amp; attach
            </Button>
          </form>
        ) : null}
      </CardContent>
    </Card>
  );
}

function ServersSection({ build }: { build: ClientAdmin }) {
  const allServers = useServers();
  const attach = useAttachServer();
  const detach = useDetachServer();
  const createServer = useCreateServer();

  const [pickId, setPickId] = useState("");
  const [creating, setCreating] = useState(false);
  const [slug, setSlug] = useState("");
  const [name, setName] = useState("");
  const [address, setAddress] = useState("");

  const attachedIds = new Set(build.servers.map((s) => s.id));
  const available = (allServers.data ?? []).filter(
    (s) => !attachedIds.has(s.id),
  );
  const pending = attach.isPending || detach.isPending || createServer.isPending;

  function attachExisting() {
    if (!pickId) {
      return;
    }
    attach.mutate(
      { clientId: build.id, serverId: pickId },
      { onSuccess: () => setPickId("") },
    );
  }

  function handleCreate(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!slug.trim() || !address.trim()) {
      return;
    }
    createServer.mutate(
      {
        slug: slug.trim(),
        name: name.trim() === "" ? null : name.trim(),
        address: address.trim(),
      },
      {
        onSuccess: (result) => {
          attach.mutate({ clientId: build.id, serverId: result.id });
          setSlug("");
          setName("");
          setAddress("");
          setCreating(false);
        },
      },
    );
  }

  return (
    <Card>
      <CardContent className="space-y-4">
        <div className="flex items-center gap-2">
          <ServerIcon className="size-4 text-text-mute" />
          <Label className="text-text-hi">Servers</Label>
        </div>

        {build.servers.length === 0 ? (
          <p className="text-body text-text-mute">No servers attached yet.</p>
        ) : (
          <ul className="space-y-2">
            {build.servers.map((server) => (
              <li
                key={server.id}
                className="flex items-center justify-between gap-3 rounded-md border border-edge bg-surface-1 px-3 py-2"
              >
                <div className="min-w-0">
                  <p className="truncate text-body-med text-text-hi">
                    {server.name ?? "—"}
                  </p>
                  <p className="truncate text-caption text-text-mute">
                    {server.address}
                  </p>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  aria-label={`Remove ${server.name ?? server.address}`}
                  disabled={pending}
                  onClick={() =>
                    detach.mutate({
                      clientId: build.id,
                      serverId: server.id,
                    })
                  }
                >
                  <X className="size-4" />
                </Button>
              </li>
            ))}
          </ul>
        )}

        <AddExistingPicker
          available={available}
          pickId={pickId}
          onPick={setPickId}
          onAdd={attachExisting}
          onToggleCreate={() => setCreating((prev) => !prev)}
          pending={pending}
          ariaLabel="Add server"
          emptyLabel="No more servers"
          selectLabel="Select a server…"
          renderOption={(server) => server.name ?? server.address}
        />

        {creating ? (
          <form
            onSubmit={handleCreate}
            className="flex flex-wrap items-end gap-2 rounded-md border border-edge bg-surface-1 p-3"
          >
            <div className="space-y-1.5">
              <Label htmlFor="new-server-slug">Slug</Label>
              <Input
                id="new-server-slug"
                value={slug}
                placeholder="survival-main"
                onChange={(event) => setSlug(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="new-server-name">Name</Label>
              <Input
                id="new-server-name"
                value={name}
                placeholder="Survival"
                onChange={(event) => setName(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="new-server-address">Address</Label>
              <Input
                id="new-server-address"
                value={address}
                placeholder="play.example.net"
                onChange={(event) => setAddress(event.target.value)}
              />
            </div>
            <Button
              type="submit"
              disabled={pending || !slug.trim() || !address.trim()}
            >
              {createServer.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Create &amp; attach
            </Button>
          </form>
        ) : null}
      </CardContent>
    </Card>
  );
}

function ServersTagsTab({ build }: { build: ClientAdmin }) {
  return (
    <div className="flex flex-col gap-4">
      <KeywordsSection build={build} />
      <ServersSection build={build} />
    </div>
  );
}

export function BuildDetailPage() {
  const { slug } = useParams();
  const { isLoading, build } = useBuildBySlug(slug);
  const publish = usePublishClient();
  const [tab, setTab] = useState<Tab>("details");

  if (isLoading) {
    return (
      <div className="flex flex-col gap-6">
        <Link
          to="/builds"
          className="inline-flex w-fit items-center gap-1.5 text-body text-text-mute hover:text-text-hi"
        >
          <ArrowLeft className="size-4" />
          Builds
        </Link>
        <div className="flex items-center justify-between gap-4">
          <Skeleton className="h-9 w-64" />
          <Skeleton className="h-9 w-24" />
        </div>
        <Skeleton className="h-20 w-full rounded-lg" />
        <Skeleton className="h-9 w-80 rounded-md" />
        <Skeleton className="h-96 w-full rounded-lg" />
      </div>
    );
  }

  if (!build) {
    return (
      <div className="flex flex-col gap-6">
        <Link
          to="/builds"
          className="inline-flex w-fit items-center gap-1.5 text-body text-text-mute hover:text-text-hi"
        >
          <ArrowLeft className="size-4" />
          Builds
        </Link>
        <EmptyState
          icon={Package}
          title="Build not found"
          description="This build does not exist or has been removed."
          action={
            <Button variant="outline" asChild>
              <Link to="/builds">Back to builds</Link>
            </Button>
          }
        />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-4">
        <Link
          to="/builds"
          className="inline-flex w-fit items-center gap-1.5 text-body text-text-mute hover:text-text-hi"
        >
          <ArrowLeft className="size-4" />
          Builds
        </Link>
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="flex items-center gap-3">
            <h1 className="text-h1 text-text-hi">{build.title}</h1>
            {build.published ? (
              <Badge variant="outline">Published</Badge>
            ) : (
              <Badge variant="secondary">Draft</Badge>
            )}
          </div>
          <Button
            variant="outline"
            disabled={publish.isPending}
            onClick={() =>
              publish.mutate({ id: build.id, publish: !build.published })
            }
          >
            {publish.isPending ? (
              <Loader2 className="size-4 animate-spin" />
            ) : null}
            {build.published ? "Unpublish" : "Publish"}
          </Button>
        </div>
      </div>

      <ManifestPanel build={build} />

      <SectionTabs tabs={TABS} value={tab} onChange={setTab} idBase="build" />

      {/* All panels stay mounted (toggled via `hidden`) so in-progress edits survive a tab switch;
          key={build.id} remounts the set only when the build identity changes. */}
      <div key={build.id} className="contents">
        <div
          role="tabpanel"
          id={panelId("build", "details")}
          aria-labelledby={tabId("build", "details")}
          hidden={tab !== "details"}
          aria-hidden={tab !== "details" || undefined}
        >
          <BuildDetailsTab build={build} />
        </div>
        <div
          role="tabpanel"
          id={panelId("build", "media")}
          aria-labelledby={tabId("build", "media")}
          hidden={tab !== "media"}
          aria-hidden={tab !== "media" || undefined}
        >
          <Card>
            <CardContent>
              <ClientMediaSection clientId={build.id} />
            </CardContent>
          </Card>
        </div>
        <div
          role="tabpanel"
          id={panelId("build", "files")}
          aria-labelledby={tabId("build", "files")}
          hidden={tab !== "files"}
          aria-hidden={tab !== "files" || undefined}
        >
          <BuildFilesTab build={build} />
        </div>
        <div
          role="tabpanel"
          id={panelId("build", "servers-tags")}
          aria-labelledby={tabId("build", "servers-tags")}
          hidden={tab !== "servers-tags"}
          aria-hidden={tab !== "servers-tags" || undefined}
        >
          <ServersTagsTab build={build} />
        </div>
      </div>
    </div>
  );
}
