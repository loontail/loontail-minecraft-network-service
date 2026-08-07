// Admin DTOs; field names are the camelCase wire contract, do not rename.

export interface AdminMe {
  id: string;
  username: string;
  email: string | null;
  isAdmin: boolean;
}

export interface LoginRequest {
  username: string;
  password: string;
}

export interface AdminUser {
  id: string;
  username: string;
  email: string | null;
  minecraftUuid: string | null;
  profileUuid: string | null;
  origin: string;
  confirmed: boolean;
  blocked: boolean;
  isAdmin: boolean;
  createdAt: string;
  lastSeenAt: string;
}

export interface PageMeta {
  page: number;
  pageSize: number;
  total: number;
  pageCount: number;
}

export interface UserListResponse {
  data: AdminUser[];
  meta: PageMeta;
}

export interface UserSearchQuery {
  q?: string;
  page?: number;
}

export interface CreateUserRequest {
  username: string;
  email: string;
  password: string;
  minecraftUuid?: string | null;
  isAdmin?: boolean;
}

export interface ResetPasswordRequest {
  password: string;
}

export interface AnalyticsOverview {
  playingNow: number;
  onlineInNetwork: number;
  openWorlds: number;
  activeRelays: number;
  totalUsers: number;
}

export interface TimeseriesPoint {
  bucket: string;
  count: number;
}

export interface TimeseriesResponse {
  metric: string;
  window: string;
  series: TimeseriesPoint[];
}

export interface Ack {
  ok: boolean;
}
