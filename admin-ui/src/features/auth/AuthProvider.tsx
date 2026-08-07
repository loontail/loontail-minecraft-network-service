import { createContext, useContext, useMemo } from "react";
import type { ReactNode } from "react";
import { toast } from "sonner";

import { useLogin, useLogout, useSession } from "@/features/auth/api";
import { ApiError } from "@/shared/api/client";
import type { AdminMe, LoginRequest } from "@/shared/types";

interface AuthContextValue {
  user: AdminMe | null;
  isLoading: boolean;
  isAuthenticated: boolean;
  login: (input: LoginRequest) => Promise<AdminMe>;
  logout: () => Promise<void>;
  isLoggingIn: boolean;
  isLoggingOut: boolean;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const session = useSession();
  const loginMutation = useLogin();
  const logoutMutation = useLogout();

  // Depend on the stable primitives, not the whole mutation objects (react-query
  // returns fresh ones every render), to avoid re-rendering every useAuth consumer.
  const { mutateAsync: loginAsync } = loginMutation;
  const { mutateAsync: logoutAsync } = logoutMutation;
  const value = useMemo<AuthContextValue>(
    () => ({
      user: session.data ?? null,
      isLoading: session.isLoading,
      isAuthenticated: Boolean(session.data),
      isLoggingIn: loginMutation.isPending,
      isLoggingOut: logoutMutation.isPending,
      login: (input) => loginAsync(input),
      logout: async () => {
        try {
          await logoutAsync();
          toast.success("Signed out");
        } catch (error) {
          const message =
            error instanceof ApiError ? error.message : "Failed to sign out";
          toast.error(message);
        }
      },
    }),
    [
      session.data,
      session.isLoading,
      loginMutation.isPending,
      logoutMutation.isPending,
      loginAsync,
      logoutAsync,
    ],
  );

  return <AuthContext value={value}>{children}</AuthContext>;
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (!ctx) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return ctx;
}
