import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiTokensPage } from "@/pages/ApiTokensPage";
import { renderWithProviders } from "@/test/renderWithProviders";

const RAW_TOKEN = "lt_live_8f3c1a9b4e2d7c6f5a0b1e2d3c4f5a6b";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

beforeEach(() => {
  // The list query loads first (empty), then the create POST returns the raw token.
  const fetchMock = vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
    const method = init?.method ?? "GET";
    if (method === "POST") {
      return Promise.resolve(
        jsonResponse({
          id: "tok-1",
          name: "Launcher production",
          scopes: [],
          createdAt: new Date().toISOString(),
          token: RAW_TOKEN,
        }),
      );
    }
    return Promise.resolve(jsonResponse([]));
  });
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("ApiTokensPage create-token flow", () => {
  it("shows the raw token exactly once in a copyable field with a save warning", async () => {
    const user = userEvent.setup();
    renderWithProviders(<ApiTokensPage />);

    await waitFor(() =>
      expect(screen.getByText(/no api tokens yet/i)).toBeInTheDocument(),
    );

    await user.click(screen.getByRole("button", { name: /new token/i }));

    const dialog = await screen.findByRole("dialog");
    await user.type(
      within(dialog).getByLabelText(/name/i),
      "Launcher production",
    );
    await user.click(
      within(dialog).getByRole("button", { name: /create token/i }),
    );

    // The revealed token field carries the raw value exactly once.
    const tokenField = await screen.findByTestId<HTMLInputElement>("raw-token");
    expect(tokenField).toHaveValue(RAW_TOKEN);
    expect(tokenField).toHaveAttribute("readonly");

    // The once-only save warning is present.
    expect(screen.getByText(/shown only once/i)).toBeInTheDocument();

    // The raw value appears in a single place (the copyable input).
    expect(screen.queryAllByDisplayValue(RAW_TOKEN)).toHaveLength(1);
  });
});
