// Bundle DTOs; field names are the camelCase wire contract, do not rename.

export interface Bundle {
  id: string;
  slug: string;
  name: string;
  description: string | null;
  version: string | null;
  status: string;
  filesCount: number;
  totalSize: number;
  processingError: string | null;
  lastGeneratedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface BundleArtifact {
  id: string;
  bundleId: string;
  relativePath: string;
  name: string;
  category: string;
  size: number;
  sha256: string | null;
  isDir: boolean;
  downloadOnce: boolean;
  fileModifiedAt: string | null;
}

/// A bundle with its artifact rows in display order.
export interface BundleWithArtifacts extends Bundle {
  artifacts: BundleArtifact[];
}

export interface CreateBundle {
  name: string;
  slug: string;
  description?: string | null;
  version?: string | null;
}

export interface UpdateBundle {
  name?: string | null;
  description?: string | null;
  version?: string | null;
}

export interface CreateFolder {
  relativePath: string;
}

export interface RenameFile {
  newRelativePath: string;
}

export interface ToggleDownloadOnce {
  downloadOnce: boolean;
}

export interface BulkDelete {
  ids: string[];
}

export interface MissingEntry {
  id: string;
  relativePath: string;
  name: string;
}

export interface OrphanEntry {
  relativePath: string;
}

export interface ValidateResult {
  missing: MissingEntry[];
  orphaned: OrphanEntry[];
}

export interface DiskSpace {
  free: number | null;
  total: number | null;
}

/// One manifest entry (manifest.rs). `sha256`/`url` are absent for directories;
/// `downloadOnce` is absent when false.
export interface ManifestEntry {
  path: string;
  name: string;
  size: number;
  isDir: boolean;
  sha256?: string;
  url?: string;
  downloadOnce?: boolean;
}

export type Manifest = Record<string, ManifestEntry[]>;
