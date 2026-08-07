import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MemoryRouter, Route, Routes } from "react-router";

import { AuthProvider } from "@/features/auth/AuthProvider";
import { LoginPage } from "@/pages/LoginPage";
import type { AdminMe } from "@/shared/types";
import { makeTestQueryClient, setupUser } from "@/test/renderWithProviders";

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

const calls: string[] = [];

// /admin/auth/me answers 401 (nobody signed in yet); the login POST is scripted per test.
function mockAuth(login: (body: unknown) => Response) {
  calls.length = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      calls.push(url);
      if (url.includes("/admin/auth/login")) {
        const body =
          typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
        return Promise.resolve(login(body));
      }
      return Promise.resolve(
        jsonResponse({ error: { code: "unauthorized", message: "nope" } }, 401),
      );
    }),
  );
}

function renderLogin() {
  return render(
    <QueryClientProvider client={makeTestQueryClient()}>
      <MemoryRouter initialEntries={["/login"]}>
        <AuthProvider>
          <Routes>
            <Route path="/login" element={<LoginPage />} />
            <Route path="/" element={<p>Admin home</p>} />
          </Routes>
        </AuthProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("LoginPage", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("shows per-field errors on an empty submit and issues no request", async () => {
    const user = setupUser();
    mockAuth(() => jsonResponse(ME));
    renderLogin();

    await user.click(screen.getByRole("button", { name: /sign in/i }));

    expect(await screen.findByText(/username is required/i)).toBeInTheDocument();
    expect(screen.getByText(/password is required/i)).toBeInTheDocument();
    expect(calls.some((url) => url.includes("/admin/auth/login"))).toBe(false);
  });

  it("signs in and navigates to the admin home", async () => {
    const user = setupUser();
    mockAuth(() => jsonResponse(ME));
    renderLogin();

    await user.type(screen.getByLabelText(/username/i), "root");
    await user.type(screen.getByLabelText(/password/i), "hunter2");
    await user.click(screen.getByRole("button", { name: /sign in/i }));

    expect(await screen.findByText("Admin home")).toBeInTheDocument();
  });

  // why: queryClient's 401 handler deliberately skips a failed login (me is already
  // null), so a rejected sign-in must leave the operator on the form to retry.
  it("stays on the form when the credentials are rejected", async () => {
    const user = setupUser();
    mockAuth(() =>
      jsonResponse(
        { error: { code: "unauthorized", message: "Invalid credentials" } },
        401,
      ),
    );
    renderLogin();

    await user.type(screen.getByLabelText(/username/i), "root");
    await user.type(screen.getByLabelText(/password/i), "wrong");
    await user.click(screen.getByRole("button", { name: /sign in/i }));

    await waitFor(() =>
      expect(calls.some((url) => url.includes("/admin/auth/login"))).toBe(true),
    );
    expect(screen.queryByText("Admin home")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /sign in/i })).toBeInTheDocument();
  });
});
