import type { ReactNode } from "react";
import { Navigate } from "react-router-dom";

import { Skeleton } from "@/components/ui/skeleton";
import { useSession } from "@/features/auth/api";

export function RequireAuth({ children }: { children: ReactNode }) {
  const { data: me, isLoading } = useSession();

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center p-8">
        <Skeleton className="h-24 w-64" />
      </div>
    );
  }

  if (!me) {
    return <Navigate to="/login" replace />;
  }

  return <>{children}</>;
}
