import { useQuery } from "@tanstack/react-query";

import { api } from "@/shared/api/client";
import type {
  AnalyticsOverview,
  Timeseries,
  TimeseriesPoint,
} from "@/shared/api/types";

// STUB: returns placeholder data until the analytics endpoints are wired in a later
// stage. Kept as the real shape so swapping in `api.get` is a one-line change.
const STUB_OVERVIEW: AnalyticsOverview = {
  playingNow: 0,
  onlineInNetwork: 0,
  openWorlds: 0,
  activeRelays: 0,
  totalUsers: 0,
};

function stubTimeseries(metric: string, window: string): Timeseries {
  const now = Date.now();
  const points: TimeseriesPoint[] = Array.from({ length: 24 }, (_, i) => ({
    timestamp: new Date(now - (23 - i) * 3_600_000).toISOString(),
    value: 0,
  }));
  return { metric, window, points };
}

async function fetchOverview(): Promise<AnalyticsOverview> {
  // Wiring deferred: fall back to the stub if the endpoint is not yet live.
  try {
    return await api.get<AnalyticsOverview>("/analytics/overview");
  } catch {
    return STUB_OVERVIEW;
  }
}

async function fetchTimeseries(
  metric: string,
  window: string,
): Promise<Timeseries> {
  try {
    return await api.get<Timeseries>(
      `/analytics/timeseries?metric=${encodeURIComponent(metric)}&window=${encodeURIComponent(window)}`,
    );
  } catch {
    return stubTimeseries(metric, window);
  }
}

export function useOverview() {
  return useQuery({
    queryKey: ["analytics", "overview"],
    queryFn: fetchOverview,
  });
}

export function useTimeseries(metric: string, window: string) {
  return useQuery({
    queryKey: ["analytics", "timeseries", metric, window],
    queryFn: () => fetchTimeseries(metric, window),
  });
}
