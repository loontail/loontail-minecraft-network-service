import { screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { TexturesPage } from "@/pages/TexturesPage";
import { renderWithProviders } from "@/test/renderWithProviders";

const SKIN = {
  userId: "user-1",
  profileUuid: "0123456789abcdef0123456789abcdef",
  username: "Steve",
  fileUrl: "/textures/0123456789abcdef0123456789abcdef/skin",
  filePath: "skins/steve.png",
  fileSize: 3 * 1024 ** 2,
  variant: "CLASSIC",
  updatedAt: "2026-08-01T00:00:00Z",
};

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

describe("TexturesPage", () => {
  beforeEach(() => {
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL) => {
        const url = String(input);
        if (url.includes("/admin/textures/skins")) {
          return Promise.resolve(
            jsonResponse({
              data: [SKIN, { ...SKIN, userId: "user-2", fileSize: 0 }],
              meta: { page: 1, pageSize: 20, total: 2, pageCount: 1 },
            }),
          );
        }
        return Promise.reject(new Error(`unexpected fetch: ${url}`));
      }),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("scales file sizes past KB and shows a zero size as 0 B", async () => {
    renderWithProviders(<TexturesPage />);

    expect(await screen.findByText("3.0 MB")).toBeInTheDocument();
    expect(screen.getByText("0 B")).toBeInTheDocument();
    expect(screen.queryByText("3072.0 KB")).not.toBeInTheDocument();
  });
});
