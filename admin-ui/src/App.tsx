import { lazy, Suspense } from "react";
import { Route, Routes } from "react-router-dom";

import { AppShell } from "@/components/layout/AppShell";
import { RequireAuth } from "@/components/layout/RequireAuth";
import { Skeleton } from "@/components/ui/skeleton";
import { LoginPage } from "@/pages/LoginPage";

// Route pages are lazy so the heavy, route-only dependencies (recharts on the
// Dashboard/Logs pages, react-aria-components on the build file manager) split
// out of the initial chunk loaded for login.
const DashboardPage = lazy(() =>
  import("@/pages/DashboardPage").then((m) => ({ default: m.DashboardPage })),
);
const UsersPage = lazy(() =>
  import("@/pages/UsersPage").then((m) => ({ default: m.UsersPage })),
);
const TexturesPage = lazy(() =>
  import("@/pages/TexturesPage").then((m) => ({ default: m.TexturesPage })),
);
const BuildsPage = lazy(() =>
  import("@/pages/BuildsPage").then((m) => ({ default: m.BuildsPage })),
);
const BuildDetailPage = lazy(() =>
  import("@/pages/BuildDetailPage").then((m) => ({
    default: m.BuildDetailPage,
  })),
);
const LogsTrafficPage = lazy(() =>
  import("@/pages/LogsTrafficPage").then((m) => ({
    default: m.LogsTrafficPage,
  })),
);

function RouteFallback() {
  return <Skeleton className="h-96 w-full rounded-lg" />;
}

export function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route
        element={
          <RequireAuth>
            <AppShell />
          </RequireAuth>
        }
      >
        <Route
          index
          element={
            <Suspense fallback={<RouteFallback />}>
              <DashboardPage />
            </Suspense>
          }
        />
        <Route
          path="users"
          element={
            <Suspense fallback={<RouteFallback />}>
              <UsersPage />
            </Suspense>
          }
        />
        <Route
          path="textures"
          element={
            <Suspense fallback={<RouteFallback />}>
              <TexturesPage />
            </Suspense>
          }
        />
        <Route
          path="builds"
          element={
            <Suspense fallback={<RouteFallback />}>
              <BuildsPage />
            </Suspense>
          }
        />
        <Route
          path="builds/:slug"
          element={
            <Suspense fallback={<RouteFallback />}>
              <BuildDetailPage />
            </Suspense>
          }
        />
        <Route
          path="logs"
          element={
            <Suspense fallback={<RouteFallback />}>
              <LogsTrafficPage />
            </Suspense>
          }
        />
      </Route>
    </Routes>
  );
}
