import { useState } from "react";

import { PageHeader } from "@/components/shared/PageHeader";
import { WindowSelector } from "@/pages/dashboard/WindowSelector";
import { LiveLogsSection } from "@/pages/logsTraffic/LiveLogsSection";
import { TrafficSection } from "@/pages/logsTraffic/TrafficSection";
import type { TrafficWindow } from "@/shared/types";

export function LogsTrafficPage() {
  const [window, setWindow] = useState<TrafficWindow>("24h");

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Logs & Traffic"
        description="Request volume, status health, and a live backend log tail."
        actions={<WindowSelector value={window} onChange={setWindow} />}
      />

      <TrafficSection window={window} />
      <LiveLogsSection />
    </div>
  );
}
