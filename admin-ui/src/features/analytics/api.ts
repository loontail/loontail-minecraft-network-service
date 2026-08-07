import { useQuery } from "@tanstack/react-query";

import { api, queryString } from "@/shared/api/client";
import type {
  AnalyticsOverview,
  TimeRange,
  TimeseriesResponse,
} from "@/shared/types";

export const analyticsKeys = {
  all: ["analytics"] as const,
  overview: () => [...analyticsKeys.all, "overview"] as const,
  timeseries: (metric: string, range: TimeRange) =>
    [...analyticsKeys.all, "timeseries", metric, range] as const,
};

export function useOverview() {
  return useQuery({
    queryKey: analyticsKeys.overview(),
    queryFn: () => api.get<AnalyticsOverview>("/admin/analytics/overview"),
    refetchInterval: 15_000,
  });
}

export function useTimeseries(metric: string, range: TimeRange) {
  return useQuery({
    queryKey: analyticsKeys.timeseries(metric, range),
    queryFn: () =>
      api.get<TimeseriesResponse>(
        `/admin/analytics/timeseries${queryString({ metric, window: range })}`,
      ),
  });
}
