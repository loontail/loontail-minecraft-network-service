import { useState } from "react";

import { PageHeader } from "@/components/shared/PageHeader";
import { TimeRangeSelector } from "@/components/shared/TimeRangeSelector";
import { LiveLogsSection } from "@/pages/requestLogs/LiveLogsSection";
import { TrafficSection } from "@/pages/requestLogs/TrafficSection";
import type { TimeRange } from "@/shared/types";

export function RequestLogsPage() {
  const [range, setRange] = useState<TimeRange>("24h");

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Logs & Traffic"
        description="Request volume, status health, and a live backend log tail."
        actions={<TimeRangeSelector value={range} onChange={setRange} />}
      />

      <TrafficSection range={range} />
      <LiveLogsSection />
    </div>
  );
}
