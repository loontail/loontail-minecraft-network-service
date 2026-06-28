import { useMemo, useState } from "react";
import {
  Activity,
  AlertTriangle,
  BarChart3,
  Gauge,
  LineChart as LineChartIcon,
  ListOrdered,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useRequestsSummary,
  useRequestsTimeseries,
} from "@/features/requestLogs/api";
import { cn } from "@/shared/lib/cn";
import type {
  RequestMetric,
  StatusClass,
  TrafficWindow,
} from "@/shared/types";

import {
  formatBucketLabel,
  formatBucketTick,
  formatLatency,
  formatPercent,
} from "./format";

const TIMESERIES_METRICS: { value: RequestMetric; label: string }[] = [
  { value: "requests", label: "Requests" },
  { value: "errors", label: "Errors" },
  { value: "latency", label: "Latency" },
];

const seriesChartConfig = {
  value: {
    label: "Value",
    color: "var(--color-chart-1)",
  },
} satisfies ChartConfig;

const STATUS_MIX_ORDER: StatusClass[] = ["2xx", "3xx", "4xx", "5xx"];

const statusMixConfig = {
  count: { label: "Requests" },
  "2xx": { label: "2xx", color: "var(--color-chart-1)" },
  "3xx": { label: "3xx", color: "var(--color-chart-2)" },
  "4xx": { label: "4xx", color: "var(--color-chart-3)" },
  "5xx": { label: "5xx", color: "var(--color-chart-5)" },
} satisfies ChartConfig;

function MetricSelector({
  value,
  onChange,
}: {
  value: RequestMetric;
  onChange: (next: RequestMetric) => void;
}) {
  return (
    <div
      role="radiogroup"
      aria-label="Timeseries metric"
      className="inline-flex items-center gap-1 rounded-md border border-edge bg-surface-1 p-1"
    >
      {TIMESERIES_METRICS.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option.value)}
            className={cn(
              "rounded px-2.5 py-1 text-caption font-semibold transition-colors",
              "outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
              active
                ? "bg-surface-3 text-text-hi"
                : "text-text-mute hover:text-text",
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

function EmptyChartState({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="flex aspect-video max-h-72 w-full flex-col items-center justify-center gap-2 rounded-md border border-dashed border-edge text-center">
      <LineChartIcon className="size-7 text-text-faint" aria-hidden />
      <p className="text-body-med text-text">{title}</p>
      <p className="max-w-xs text-caption text-text-faint">{detail}</p>
    </div>
  );
}

interface KpiDef {
  label: string;
  icon: LucideIcon;
  hint: string;
  render: (value: number) => string;
}

const KPIS: KpiDef[] = [
  {
    label: "Total requests",
    icon: Activity,
    hint: "In the selected window",
    render: (v) => v.toLocaleString(),
  },
  {
    label: "Error rate",
    icon: AlertTriangle,
    hint: "Share of 5xx responses",
    render: formatPercent,
  },
  {
    label: "Avg latency",
    icon: Gauge,
    hint: "Mean response time",
    render: formatLatency,
  },
];

export function TrafficSection({ window }: { window: TrafficWindow }) {
  const [metric, setMetric] = useState<RequestMetric>("requests");
  const summary = useRequestsSummary(window);
  const series = useRequestsTimeseries(window, metric);

  const points = series.data?.series ?? [];
  const hasSeries = useMemo(
    () => points.some((point) => point.value > 0),
    [points],
  );

  const kpiValues: (number | undefined)[] = summary.data
    ? [
        summary.data.totalRequests,
        summary.data.errorRate,
        summary.data.avgLatencyMs,
      ]
    : [undefined, undefined, undefined];

  const mixData = useMemo(() => {
    const map = new Map(
      (summary.data?.statusMix ?? []).map((s) => [s.statusClass, s.count]),
    );
    return STATUS_MIX_ORDER.map((statusClass) => ({
      statusClass,
      count: map.get(statusClass) ?? 0,
      fill: `var(--color-${statusClass})`,
    }));
  }, [summary.data]);

  const hasMix = mixData.some((row) => row.count > 0);
  const topPaths = summary.data?.topPaths ?? [];

  return (
    <section className="flex flex-col gap-4" aria-label="Traffic dashboard">
      {summary.isError ? (
        <Card>
          <CardContent className="py-8 text-center text-body text-text-mute">
            Could not load traffic summary. Retrying automatically.
          </CardContent>
        </Card>
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
          {KPIS.map((def, i) => {
            const raw = kpiValues[i];
            return (
              <StatCardDisplay
                key={def.label}
                label={def.label}
                icon={def.icon}
                hint={def.hint}
                display={raw === undefined ? undefined : def.render(raw)}
                isLoading={summary.isLoading}
              />
            );
          })}
        </div>
      )}

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-3">
        <Card className="xl:col-span-2">
          <CardHeader className="flex flex-row items-start justify-between gap-4">
            <div className="flex flex-col gap-1.5">
              <CardTitle className="flex items-center gap-2 text-h2 text-text-hi">
                <LineChartIcon
                  className="size-4 text-text-faint"
                  aria-hidden
                />
                Requests over time
              </CardTitle>
              <CardDescription>
                Bucketed over the selected window.
              </CardDescription>
            </div>
            <MetricSelector value={metric} onChange={setMetric} />
          </CardHeader>
          <CardContent>
            {series.isLoading ? (
              <Skeleton className="aspect-video max-h-72 w-full" />
            ) : series.isError ? (
              <EmptyChartState
                title="Couldn't load timeseries"
                detail="The request failed. Try another window or metric."
              />
            ) : !hasSeries ? (
              <EmptyChartState
                title="No traffic yet"
                detail="Request volume will appear here as the service handles traffic."
              />
            ) : (
              <ChartContainer
                config={seriesChartConfig}
                className="max-h-72 w-full"
              >
                <AreaChart data={points} margin={{ left: 4, right: 8, top: 8 }}>
                  <defs>
                    <linearGradient
                      id="fillRequests"
                      x1="0"
                      y1="0"
                      x2="0"
                      y2="1"
                    >
                      <stop
                        offset="5%"
                        stopColor="var(--color-value)"
                        stopOpacity={0.3}
                      />
                      <stop
                        offset="95%"
                        stopColor="var(--color-value)"
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
                    width={40}
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
                    dataKey="value"
                    type="monotone"
                    fill="url(#fillRequests)"
                    stroke="var(--color-value)"
                    strokeWidth={2}
                  />
                </AreaChart>
              </ChartContainer>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-h2 text-text-hi">
              <BarChart3 className="size-4 text-text-faint" aria-hidden />
              Status mix
            </CardTitle>
            <CardDescription>Responses by status class.</CardDescription>
          </CardHeader>
          <CardContent>
            {summary.isLoading ? (
              <Skeleton className="aspect-video max-h-72 w-full" />
            ) : !hasMix ? (
              <EmptyChartState
                title="No responses yet"
                detail="Status classes will chart here once requests are recorded."
              />
            ) : (
              <ChartContainer
                config={statusMixConfig}
                className="max-h-72 w-full"
              >
                <BarChart
                  data={mixData}
                  margin={{ left: 4, right: 8, top: 8 }}
                >
                  <CartesianGrid vertical={false} />
                  <XAxis
                    dataKey="statusClass"
                    tickLine={false}
                    axisLine={false}
                    tickMargin={8}
                  />
                  <YAxis
                    tickLine={false}
                    axisLine={false}
                    width={40}
                    allowDecimals={false}
                  />
                  <ChartTooltip
                    cursor={false}
                    content={<ChartTooltipContent nameKey="count" />}
                  />
                  <Bar dataKey="count" radius={4} />
                </BarChart>
              </ChartContainer>
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-h2 text-text-hi">
            <ListOrdered className="size-4 text-text-faint" aria-hidden />
            Top paths
          </CardTitle>
          <CardDescription>
            Most-requested routes in the selected window.
          </CardDescription>
        </CardHeader>
        <CardContent className="px-0">
          {summary.isLoading ? (
            <div className="px-6">
              <Skeleton className="h-40 w-full" />
            </div>
          ) : topPaths.length === 0 ? (
            <p className="px-6 py-6 text-center text-body text-text-mute">
              No path activity in this window yet.
            </p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Path</TableHead>
                  <TableHead className="text-right">Requests</TableHead>
                  <TableHead className="text-right">Avg latency</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {topPaths.map((row) => (
                  <TableRow key={row.path}>
                    <TableCell className="font-mono text-caption text-text">
                      {row.path}
                    </TableCell>
                    <TableCell className="text-right tabular-nums text-text-mute">
                      {row.count.toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right tabular-nums text-text-mute">
                      {formatLatency(row.avgLatencyMs)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </section>
  );
}

/// Like the dashboard StatCard, but renders a preformatted string (rate/latency)
/// rather than a raw number through toLocaleString.
function StatCardDisplay({
  label,
  icon: Icon,
  hint,
  display,
  isLoading,
}: {
  label: string;
  icon: LucideIcon;
  hint: string;
  display: string | undefined;
  isLoading?: boolean;
}) {
  const showSkeleton = isLoading || display === undefined;
  return (
    <Card className="gap-3 py-5 transition-colors hover:border-edge-md">
      <CardHeader className="gap-2">
        <CardDescription className="flex items-center gap-2 text-text-mute">
          <Icon className="size-4 text-text-faint" aria-hidden />
          {label}
        </CardDescription>
        <CardTitle className="text-display tabular-nums text-text-hi">
          {showSkeleton ? <Skeleton className="h-8 w-20" /> : display}
        </CardTitle>
        <p className="text-caption text-text-faint">{hint}</p>
      </CardHeader>
    </Card>
  );
}
