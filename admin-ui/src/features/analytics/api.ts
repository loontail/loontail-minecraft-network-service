import { useQuery } from "@tanstack/react-query";

import { api, queryString } from "@/shared/api/client";
import type { AnalyticsOverview, TimeseriesResponse } from "@/shared/types";

export const analyticsKeys = {
  all: ["analytics"] as const,
  overview: () => [...analyticsKeys.all, "overview"] as const,
  timeseries: (metric: string, window: string) =>
    [...analyticsKeys.all, "timeseries", metric, window] as const,
};

export function useOverview() {
  return useQuery({
    queryKey: analyticsKeys.overview(),
    queryFn: () => api.get<AnalyticsOverview>("/admin/analytics/overview"),
    refetchInterval: 15_000,
  });
}

export function useTimeseries(metric: string, window: string) {
  return useQuery({
    queryKey: analyticsKeys.timeseries(metric, window),
    queryFn: () =>
      api.get<TimeseriesResponse>(
        `/admin/analytics/timeseries${queryString({ metric, window })}`,
      ),
  });
}
