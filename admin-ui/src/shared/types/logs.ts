// Wire contract for /admin/logs/tail and /admin/analytics/requests*; field names are camelCase, do not rename.

export type AuthKind = "session" | "admin" | "anon";

export type StatusClass = "2xx" | "3xx" | "4xx" | "5xx";

export type TrafficWindow = "24h" | "7d" | "30d";

export type RequestMetric = "requests" | "errors" | "latency";

/// One request from the in-memory ring buffer (logs tail).
export interface RequestLogEntry {
  ts: string;
  method: string;
  path: string;
  status: number;
  latencyMs: number;
  userId: string | null;
  authKind: AuthKind;
  ip: string | null;
  userAgent: string | null;
  bytesOut: number | null;
}

/// Persisted analytics row: the tail shape plus a stable id.
export interface RequestLogRow extends RequestLogEntry {
  id: string;
}

export interface LogsTailResponse {
  entries: RequestLogEntry[];
}

export interface RequestLogsQuery {
  since?: string;
  until?: string;
  method?: string;
  path?: string;
  status?: number;
  page?: number;
}

export interface RequestLogsResponse {
  data: RequestLogRow[];
  meta: {
    page: number;
    pageSize: number;
    total: number;
    pageCount: number;
  };
}

export interface StatusMixSlice {
  statusClass: StatusClass;
  count: number;
}

export interface TopPath {
  path: string;
  count: number;
  avgLatencyMs: number;
}

export interface RequestsSummary {
  window: TrafficWindow;
  totalRequests: number;
  // Share of responses with status >= 500 (0..1).
  errorRate: number;
  avgLatencyMs: number;
  statusMix: StatusMixSlice[];
  topPaths: TopPath[];
}

export interface RequestsTimeseriesPoint {
  bucket: string;
  value: number;
}

export interface RequestsTimeseriesResponse {
  window: TrafficWindow;
  metric: RequestMetric;
  series: RequestsTimeseriesPoint[];
}
