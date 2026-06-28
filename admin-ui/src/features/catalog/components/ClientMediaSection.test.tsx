import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ScreenshotsGallery } from "@/features/catalog/components/ClientMediaSection";
import type { MediaRow } from "@/shared/types";

const uploadMutate = vi.fn();
const deleteMutate = vi.fn();

vi.mock("@/features/catalog/api", () => ({
  useClientMedia: () => ({ data: [] as MediaRow[], isLoading: false }),
  useUploadMedia: () => ({ mutate: uploadMutate, isPending: false }),
  useDeleteMedia: () => ({ mutate: deleteMutate, isPending: false }),
}));

function pngFile(name: string): File {
  return new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], name, {
    type: "image/png",
  });
}

describe("ScreenshotsGallery", () => {
  beforeEach(() => {
    uploadMutate.mockClear();
    deleteMutate.mockClear();
  });

  it("uploads every selected screenshot (multi-file iteration)", async () => {
    const user = userEvent.setup();
    const { container } = render(
      <ScreenshotsGallery clientId="client-1" shots={[]} />,
    );

    const input = container.querySelector<HTMLInputElement>(
      "input[type=file]",
    );
    expect(input).not.toBeNull();
    expect(input?.multiple).toBe(true);

    await user.upload(input!, [pngFile("one.png"), pngFile("two.png")]);

    expect(uploadMutate).toHaveBeenCalledTimes(2);
    expect(uploadMutate.mock.calls[0][0]).toMatchObject({
      clientId: "client-1",
      role: "screenshot",
    });
    expect(uploadMutate.mock.calls[1][0]).toMatchObject({
      clientId: "client-1",
      role: "screenshot",
    });
  });

  it("removes a screenshot via the hover action", async () => {
    const user = userEvent.setup();
    render(
      <ScreenshotsGallery
        clientId="client-1"
        shots={[{ id: "shot-1", url: "/m/shot-1.png" }]}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /remove screenshot/i }),
    );
    expect(deleteMutate).toHaveBeenCalledWith({
      clientId: "client-1",
      mediaId: "shot-1",
    });
  });
});
