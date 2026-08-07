// Texture admin DTOs; field names are the camelCase wire contract, do not rename.

import type { PageMeta } from "@/shared/types/admin";

export type TextureKind = "skins" | "capes";

export interface TextureRow {
  userId: string;
  profileUuid: string;
  username: string;
  fileUrl: string;
  filePath: string;
  fileSize: number;
  // CLASSIC | SLIM for skins; null for capes.
  variant: string | null;
  updatedAt: string;
}

export interface TextureListResponse {
  data: TextureRow[];
  meta: PageMeta;
}

export interface TextureSearchQuery {
  q?: string;
  page?: number;
}

export interface OrphansResponse {
  skins: TextureRow[];
  capes: TextureRow[];
}

export interface PurgeResponse {
  purgedSkins: number;
  purgedCapes: number;
}

export interface DeleteAck {
  deleted: boolean;
}
