import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { UsersPage } from "@/pages/UsersPage";
import { renderWithProviders } from "@/test/renderWithProviders";
import type { AdminUser, UserListResponse } from "@/shared/types";

const SAMPLE_USERS: AdminUser[] = [
  {
    id: "11111111-1111-1111-1111-111111111111",
    username: "alice",
    email: "alice@example.com",
    minecraftUuid: null,
    profileUuid: "aaaaaaaabbbbccccddddeeeeffff0000",
    origin: "admin",
    confirmed: true,
    blocked: false,
    isAdmin: true,
    createdAt: "2026-01-02T03:04:05Z",
    lastSeenAt: "2026-06-01T00:00:00Z",
  },
  {
    id: "22222222-2222-2222-2222-222222222222",
    username: "bob",
    email: "bob@example.com",
    minecraftUuid: null,
    profileUuid: "11112222333344445555666677778888",
    origin: "yggdrasil",
    confirmed: false,
    blocked: true,
    isAdmin: false,
    createdAt: "2026-02-10T10:00:00Z",
    lastSeenAt: "2026-06-01T00:00:00Z",
  },
];

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

describe("UsersPage", () => {
  beforeEach(() => {
    const response: UserListResponse = {
      data: SAMPLE_USERS,
      meta: { page: 1, pageSize: 20, total: 2, pageCount: 1 },
    };
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve(jsonResponse(response))),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders the users table with rows from the query", async () => {
    renderWithProviders(<UsersPage />);

    expect(
      screen.getByRole("heading", { name: /users/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: /username/i }),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("alice")).toBeInTheDocument();
    });
    expect(screen.getByText("bob")).toBeInTheDocument();
    expect(screen.getByText("alice@example.com")).toBeInTheDocument();
    expect(screen.getByText("Blocked")).toBeInTheDocument();
  });

  it("opens the create-user dialog bound to Yggdrasil", async () => {
    const user = userEvent.setup();
    renderWithProviders(<UsersPage />);

    await user.click(screen.getByRole("button", { name: /create user/i }));

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText(/create user bound to yggdrasil/i),
    ).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/username/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/email/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/minecraft uuid/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/administrator/i)).toBeInTheDocument();
  });

  it("validates the email field: required, format-checked, and trimmed before the format check", async () => {
    const user = userEvent.setup();
    renderWithProviders(<UsersPage />);

    await user.click(screen.getByRole("button", { name: /create user/i }));
    const dialog = await screen.findByRole("dialog");
    const email = within(dialog).getByLabelText(/email/i);
    const submit = within(dialog).getByRole("button", { name: /^create user$/i });

    await user.click(submit);
    expect(await within(dialog).findByText("Email is required")).toBeInTheDocument();

    await user.type(email, "nope");
    await user.click(submit);
    expect(
      await within(dialog).findByText("Enter a valid email"),
    ).toBeInTheDocument();

    await user.clear(email);
    await user.type(email, "  padded@example.com  ");
    await user.click(submit);
    await waitFor(() => {
      expect(within(dialog).queryByText("Enter a valid email")).toBeNull();
    });
    expect(within(dialog).queryByText("Email is required")).toBeNull();
  });
});
