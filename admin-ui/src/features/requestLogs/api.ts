import { useQuery } from "@tanstack/react-query";

import { api, queryString } from "@/shared/api/client";
import type {
  LogsTailResponse,
  RequestLogsQuery,
  RequestLogsResponse,
  RequestMetric,
  RequestsSummary,
  RequestsTimeseriesResponse,
  TrafficWindow,
} from "@/shared/types";

export const requestLogsKeys = {
  all: ["requestLogs"] as const,
  tail: (limit: number) => [...requestLogsKeys.all, "tail", limit] as const,
  requests: (query: RequestLogsQuery) =>
    [...requestLogsKeys.all, "requests", query] as const,
  summary: (window: TrafficWindow) =>
    [...requestLogsKeys.all, "summary", window] as const,
  timeseries: (window: TrafficWindow, metric: RequestMetric) =>
    [...requestLogsKeys.all, "timeseries", window, metric] as const,
};

export function useLogsTail(limit = 100) {
  return useQuery({
    queryKey: requestLogsKeys.tail(limit),
    queryFn: () =>
      api.get<LogsTailResponse>(`/admin/logs/tail${queryString({ limit })}`),
    refetchInterval: 5_000,
  });
}

export function useRequestLogs(query: RequestLogsQuery) {
  return useQuery({
    queryKey: requestLogsKeys.requests(query),
    queryFn: () =>
      api.get<RequestLogsResponse>(
        `/admin/analytics/requests${queryString({ ...query })}`,
      ),
  });
}

export function useRequestsSummary(window: TrafficWindow) {
  return useQuery({
    queryKey: requestLogsKeys.summary(window),
    queryFn: () =>
      api.get<RequestsSummary>(
        `/admin/analytics/requests/summary${queryString({ window })}`,
      ),
    refetchInterval: 30_000,
  });
}

export function useRequestsTimeseries(
  window: TrafficWindow,
  metric: RequestMetric,
) {
  return useQuery({
    queryKey: requestLogsKeys.timeseries(window, metric),
    queryFn: () =>
      api.get<RequestsTimeseriesResponse>(
        `/admin/analytics/requests/timeseries${queryString({ window, metric })}`,
      ),
  });
}
