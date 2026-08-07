import { useQuery } from "@tanstack/react-query";

import { api, queryString } from "@/shared/api/client";
import type {
  RequestLogSummary,
  RequestLogTailResponse,
  RequestLogTimeseries,
  RequestMetric,
  TimeRange,
} from "@/shared/types";

export const requestLogsKeys = {
  all: ["requestLogs"] as const,
  tail: (limit: number) => [...requestLogsKeys.all, "tail", limit] as const,
  summary: (range: TimeRange) =>
    [...requestLogsKeys.all, "summary", range] as const,
  timeseries: (range: TimeRange, metric: RequestMetric) =>
    [...requestLogsKeys.all, "timeseries", range, metric] as const,
};

export function useRequestLogTail(limit = 100) {
  return useQuery({
    queryKey: requestLogsKeys.tail(limit),
    queryFn: () =>
      api.get<RequestLogTailResponse>(
        `/admin/logs/tail${queryString({ limit })}`,
      ),
    refetchInterval: 5_000,
  });
}

export function useRequestLogSummary(range: TimeRange) {
  return useQuery({
    queryKey: requestLogsKeys.summary(range),
    queryFn: () =>
      api.get<RequestLogSummary>(
        `/admin/analytics/requests/summary${queryString({ window: range })}`,
      ),
    refetchInterval: 30_000,
  });
}

export function useRequestLogTimeseries(
  range: TimeRange,
  metric: RequestMetric,
) {
  return useQuery({
    queryKey: requestLogsKeys.timeseries(range, metric),
    queryFn: () =>
      api.get<RequestLogTimeseries>(
        `/admin/analytics/requests/timeseries${queryString({ window: range, metric })}`,
      ),
  });
}
