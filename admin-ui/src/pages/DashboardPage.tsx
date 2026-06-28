import { useMemo, useState } from "react";
import {
  Activity,
  Boxes,
  Gamepad2,
  Radio,
  Users,
  Wifi,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from "recharts";

import { EmptyChartState } from "@/components/shared/EmptyChartState";
import { PageHeader } from "@/components/shared/PageHeader";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import { useOverview, useTimeseries } from "@/features/analytics/api";
import { StatCard } from "@/pages/dashboard/StatCard";
import { WindowSelector } from "@/pages/dashboard/WindowSelector";
import type { AnalyticsOverview, TrafficWindow } from "@/shared/types";

interface StatDef {
  key: keyof AnalyticsOverview;
  label: string;
  icon: LucideIcon;
  hint: string;
}

const STATS: StatDef[] = [
  {
    key: "playingNow",
    label: "Playing now",
    icon: Gamepad2,
    hint: "In a world or joinable",
  },
  {
    key: "onlineInNetwork",
    label: "Online in network",
    icon: Wifi,
    hint: "Active heartbeats",
  },
  {
    key: "openWorlds",
    label: "Open worlds",
    icon: Boxes,
    hint: "Sessions accepting joins",
  },
  {
    key: "activeRelays",
    label: "Active relays",
    icon: Radio,
    hint: "Live relay tunnels",
  },
  {
    key: "totalUsers",
    label: "Total users",
    icon: Users,
    hint: "Registered accounts",
  },
];

const TIMESERIES_METRIC = "bootstraps";

const chartConfig = {
  count: {
    label: "Bootstraps",
    color: "var(--color-chart-1)",
  },
} satisfies ChartConfig;

function formatBucketTick(value: string, window: TrafficWindow): string {
  const date = new Date(value);
  if (window === "24h") {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

function formatBucketLabel(value: string, window: TrafficWindow): string {
  const date = new Date(value);
  if (window === "24h") {
    return date.toLocaleString([], {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  return date.toLocaleDateString([], {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

export function DashboardPage() {
  const [window, setWindow] = useState<TrafficWindow>("24h");
  const overview = useOverview();
  const series = useTimeseries(TIMESERIES_METRIC, window);

  const points = series.data?.series ?? [];
  const hasData = useMemo(
    () => points.some((point) => point.count > 0),
    [points],
  );

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Dashboard"
        description="Live network activity and account growth."
      />

      {overview.isError ? (
        <Card>
          <CardContent className="py-8 text-center text-body text-text-mute">
            Could not load network overview. Retrying automatically.
          </CardContent>
        </Card>
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
          {STATS.map((def) => (
            <StatCard
              key={def.key}
              label={def.label}
              icon={def.icon}
              hint={def.hint}
              value={overview.data?.[def.key]}
              isLoading={overview.isLoading}
            />
          ))}
        </div>
      )}

      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-4">
          <div className="flex flex-col gap-1.5">
            <CardTitle className="flex items-center gap-2 text-h2 text-text-hi">
              <Activity className="size-4 text-text-faint" aria-hidden />
              Client bootstraps
            </CardTitle>
            <CardDescription>
              Mod logins, bucketed over the selected window.
            </CardDescription>
          </div>
          <WindowSelector value={window} onChange={setWindow} />
        </CardHeader>
        <CardContent>
          {series.isLoading ? (
            <Skeleton className="aspect-video max-h-72 w-full" />
          ) : series.isError ? (
            <EmptyChartState
              title="Couldn't load activity"
              detail="The timeseries request failed. Try another window."
            />
          ) : !hasData ? (
            <EmptyChartState
              title="No activity yet"
              detail="Client bootstraps will appear here as players come online."
            />
          ) : (
            <ChartContainer config={chartConfig} className="max-h-72 w-full">
              <AreaChart
                data={points}
                margin={{ left: 4, right: 8, top: 8 }}
              >
                <defs>
                  <linearGradient
                    id="fillBootstraps"
                    x1="0"
                    y1="0"
                    x2="0"
                    y2="1"
                  >
                    <stop
                      offset="5%"
                      stopColor="var(--color-count)"
                      stopOpacity={0.3}
                    />
                    <stop
                      offset="95%"
                      stopColor="var(--color-count)"
                      stopOpacity={0.02}
                    />
                  </linearGradient>
                </defs>
                <CartesianGrid vertical={false} />
                <XAxis
                  dataKey="bucket"
                  tickLine={false}
                  axisLine={false}
                  tickMargin={8}
                  minTickGap={32}
                  tickFormatter={(value: string) =>
                    formatBucketTick(value, window)
                  }
                />
                <YAxis
                  tickLine={false}
                  axisLine={false}
                  width={32}
                  allowDecimals={false}
                />
                <ChartTooltip
                  content={
                    <ChartTooltipContent
                      labelFormatter={(_, payload) => {
                        const bucket = payload?.[0]?.payload?.bucket as
                          | string
                          | undefined;
                        return bucket
                          ? formatBucketLabel(bucket, window)
                          : "";
                      }}
                    />
                  }
                />
                <Area
                  dataKey="count"
                  type="monotone"
                  fill="url(#fillBootstraps)"
                  stroke="var(--color-count)"
                  strokeWidth={2}
                />
              </AreaChart>
            </ChartContainer>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
