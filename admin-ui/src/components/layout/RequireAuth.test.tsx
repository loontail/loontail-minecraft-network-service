import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter, Route, Routes } from "react-router";

import { RequireAuth } from "@/components/layout/RequireAuth";
import { authKeys } from "@/features/auth/api";
import { AuthProvider } from "@/features/auth/AuthProvider";
import type { AdminMe } from "@/shared/types";
import { makeTestQueryClient } from "@/test/renderWithProviders";

const ME: AdminMe = {
  id: "u1",
  username: "root",
  email: "root@example.com",
  isAdmin: true,
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

// `never` keeps the session query in flight so the loading branch can be observed.
function mockSession(mode: "pending" | "ok" | "unauthorized") {
  vi.stubGlobal(
    "fetch",
    vi.fn(() => {
      if (mode === "pending") {
        return new Promise<Response>(() => {});
      }
      if (mode === "ok") {
        return Promise.resolve(jsonResponse(ME));
      }
      return Promise.resolve(
        jsonResponse({ error: { code: "unauthorized", message: "nope" } }, 401),
      );
    }),
  );
}

function renderGate() {
  const client = makeTestQueryClient();
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={["/"]}>
        <AuthProvider>
          <Routes>
            <Route
              path="/"
              element={
                <RequireAuth>
                  <p>Protected content</p>
                </RequireAuth>
              }
            />
            <Route path="/login" element={<p>Login screen</p>} />
          </Routes>
        </AuthProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  );
  return client;
}

describe("RequireAuth", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("shows the session spinner while /admin/auth/me is in flight", () => {
    mockSession("pending");
    renderGate();
    expect(screen.getByText(/loading session/i)).toBeInTheDocument();
    expect(screen.queryByText("Protected content")).not.toBeInTheDocument();
  });

  it("redirects to /login when there is no session", async () => {
    mockSession("unauthorized");
    renderGate();
    expect(await screen.findByText("Login screen")).toBeInTheDocument();
    expect(screen.queryByText("Protected content")).not.toBeInTheDocument();
  });

  it("renders the children for an authenticated admin", async () => {
    mockSession("ok");
    renderGate();
    expect(await screen.findByText("Protected content")).toBeInTheDocument();
    expect(screen.queryByText("Login screen")).not.toBeInTheDocument();
  });

  // A mid-session 401 sets me=null in the cache; the gate must follow it out.
  it("redirects when the cached session is cleared mid-session", async () => {
    mockSession("ok");
    const client = renderGate();
    expect(await screen.findByText("Protected content")).toBeInTheDocument();

    client.setQueryData(authKeys.me, null);
    expect(await screen.findByText("Login screen")).toBeInTheDocument();
  });
});
