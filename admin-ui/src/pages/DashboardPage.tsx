import { Gamepad2 } from "lucide-react";
import { Link } from "react-router";

import { PageHeader } from "@/components/shared/PageHeader";
import { StatCard } from "@/components/shared/StatCard";
import { useOverview } from "@/features/analytics/api";

export function DashboardPage() {
  const overview = useOverview();

  return (
    <div className="flex flex-col gap-6">
      <PageHeader title="Dashboard" description="Admin home." />

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
        <StatCard
          label="Playing now"
          icon={Gamepad2}
          hint="In a world or joinable"
          value={overview.data?.playingNow}
          display={overview.isError ? "—" : undefined}
          isLoading={overview.isLoading}
          footer={
            <Link
              to="/network"
              className="text-caption font-semibold text-text-mute transition-colors hover:text-text-hi"
            >
              View network →
            </Link>
          }
        />
      </div>
    </div>
  );
}
