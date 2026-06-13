import {
  Boxes,
  KeyRound,
  LayoutDashboard,
  Library,
  LogOut,
  Users,
} from "lucide-react";
import { NavLink, Outlet } from "react-router-dom";

import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { useAuth } from "@/features/auth/AuthProvider";
import { cn } from "@/shared/lib/cn";

const NAV = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard, end: true },
  { to: "/users", label: "Users", icon: Users, end: false },
  { to: "/catalog", label: "Catalog", icon: Library, end: false },
  { to: "/bundles", label: "Bundles", icon: Boxes, end: false },
  { to: "/api-tokens", label: "API Tokens", icon: KeyRound, end: false },
] as const;

export function AppShell() {
  const { user, logout, isLoggingOut } = useAuth();

  const initials = user?.username?.slice(0, 2).toUpperCase() ?? "AD";

  return (
    <div className="flex h-full">
      <aside className="glass flex w-60 shrink-0 flex-col border-r border-edge">
        <div className="flex h-14 items-center gap-2 px-4">
          <div className="flex size-7 items-center justify-center rounded-md bg-cta text-on-cta">
            <span className="text-caption font-extrabold">L</span>
          </div>
          <span className="text-h2 font-bold tracking-tight">Loontail</span>
        </div>
        <Separator />
        <nav className="flex flex-1 flex-col gap-1 p-2">
          {NAV.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-3 rounded-md px-3 py-2 text-body transition-colors",
                  "text-text-mute hover:bg-ghost-hover hover:text-text-hi",
                  isActive &&
                    "bg-accent-soft text-text-hi font-semibold hover:bg-accent-soft",
                )
              }
            >
              <Icon className="size-4" />
              {label}
            </NavLink>
          ))}
        </nav>
        <Separator />
        <div className="flex items-center gap-3 p-3">
          <Avatar>
            <AvatarFallback>{initials}</AvatarFallback>
          </Avatar>
          <div className="min-w-0 flex-1">
            <p className="truncate text-body-med text-text-hi">
              {user?.username ?? "Administrator"}
            </p>
            <p className="truncate text-caption text-text-faint">
              {user?.email ?? "Admin"}
            </p>
          </div>
          <Button
            variant="ghost"
            size="icon"
            aria-label="Log out"
            disabled={isLoggingOut}
            onClick={() => void logout()}
          >
            <LogOut className="size-4" />
          </Button>
        </div>
      </aside>
      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto max-w-7xl animate-[var(--animate-route-fade)] p-8">
          <Outlet />
        </div>
      </main>
    </div>
  );
}
