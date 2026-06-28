import { Route, Routes } from "react-router-dom";

import { AppShell } from "@/components/layout/AppShell";
import { RequireAuth } from "@/components/layout/RequireAuth";
import { BuildDetailPage } from "@/pages/BuildDetailPage";
import { BuildsPage } from "@/pages/BuildsPage";
import { DashboardPage } from "@/pages/DashboardPage";
import { LogsTrafficPage } from "@/pages/LogsTrafficPage";
import { LoginPage } from "@/pages/LoginPage";
import { TexturesPage } from "@/pages/TexturesPage";
import { UsersPage } from "@/pages/UsersPage";

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
        <Route index element={<DashboardPage />} />
        <Route path="users" element={<UsersPage />} />
        <Route path="textures" element={<TexturesPage />} />
        <Route path="builds" element={<BuildsPage />} />
        <Route path="builds/:slug" element={<BuildDetailPage />} />
        <Route path="logs" element={<LogsTrafficPage />} />
      </Route>
    </Routes>
  );
}
