export interface AnalyticsOverview {
  playingNow: number;
  onlineInNetwork: number;
  openWorlds: number;
  activeRelays: number;
  totalUsers: number;
}

export interface TimeseriesPoint {
  timestamp: string;
  value: number;
}

export interface Timeseries {
  metric: string;
  window: string;
  points: TimeseriesPoint[];
}

export interface AdminMe {
  id: string;
  username: string;
}
