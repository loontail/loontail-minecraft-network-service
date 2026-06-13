import { Activity, Boxes, Gamepad2, Users, Wifi } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  XAxis,
  YAxis,
} from "recharts";

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
import type { AnalyticsOverview } from "@/shared/api/types";

interface StatDef {
  key: keyof AnalyticsOverview;
  label: string;
  icon: LucideIcon;
}

const STATS: StatDef[] = [
  { key: "playingNow", label: "Playing now", icon: Gamepad2 },
  { key: "onlineInNetwork", label: "Online in network", icon: Wifi },
  { key: "openWorlds", label: "Open worlds", icon: Boxes },
  { key: "totalUsers", label: "Total users", icon: Users },
];

const chartConfig = {
  value: {
    label: "Online",
    color: "var(--color-chart-1)",
  },
} satisfies ChartConfig;

function StatCard({ def, value }: { def: StatDef; value: number | undefined }) {
  const Icon = def.icon;
  return (
    <Card>
      <CardHeader>
        <CardDescription className="flex items-center gap-2">
          <Icon className="size-4" />
          {def.label}
        </CardDescription>
        <CardTitle className="text-display tabular-nums">
          {value === undefined ? (
            <Skeleton className="h-8 w-16" />
          ) : (
            value.toLocaleString()
          )}
        </CardTitle>
      </CardHeader>
    </Card>
  );
}

export function DashboardPage() {
  const overview = useOverview();
  const series = useTimeseries("onlineInNetwork", "24h");

  return (
    <div className="flex flex-col gap-6">
      <header>
        <h1 className="text-h1 text-text-hi">Dashboard</h1>
        <p className="text-body text-text-mute">
          Live network analytics and account activity.
        </p>
      </header>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {STATS.map((def) => (
          <StatCard
            key={def.key}
            def={def}
            value={overview.data?.[def.key]}
          />
        ))}
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-h2">
            <Activity className="size-4" />
            Online in network
          </CardTitle>
          <CardDescription>Last 24 hours</CardDescription>
        </CardHeader>
        <CardContent>
          {series.isLoading ? (
            <Skeleton className="aspect-video w-full" />
          ) : (
            <ChartContainer config={chartConfig} className="max-h-72 w-full">
              <AreaChart data={series.data?.points ?? []}>
                <CartesianGrid vertical={false} />
                <XAxis
                  dataKey="timestamp"
                  tickLine={false}
                  axisLine={false}
                  tickMargin={8}
                  minTickGap={32}
                  tickFormatter={(value: string) =>
                    new Date(value).toLocaleTimeString([], {
                      hour: "2-digit",
                    })
                  }
                />
                <YAxis
                  tickLine={false}
                  axisLine={false}
                  width={32}
                  allowDecimals={false}
                />
                <ChartTooltip content={<ChartTooltipContent />} />
                <Area
                  dataKey="value"
                  type="monotone"
                  fill="var(--color-value)"
                  fillOpacity={0.2}
                  stroke="var(--color-value)"
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
