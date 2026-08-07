import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/shared/api/client";
import { ScreenshotsGallery } from "@/features/builds/BuildMediaSection";
import type { MediaRow } from "@/shared/types";
import { setupUser } from "@/test/renderWithProviders";

const uploadMutate = vi.fn();
const uploadMutateAsync = vi.fn();
const deleteMutate = vi.fn();
const invalidateMedia = vi.fn();

vi.mock("@/features/builds/api", () => ({
  useBuildMedia: () => ({ data: [] as MediaRow[], isLoading: false }),
  useUploadMedia: () => ({
    mutate: uploadMutate,
    mutateAsync: uploadMutateAsync,
    isPending: false,
  }),
  useDeleteMedia: () => ({ mutate: deleteMutate, isPending: false }),
  useInvalidateBuildMedia: () => invalidateMedia,
}));

const toastSuccess = vi.fn();
const toastError = vi.fn();
const toastWarning = vi.fn();

vi.mock("sonner", () => ({
  toast: {
    success: (...args: unknown[]) => toastSuccess(...args),
    error: (...args: unknown[]) => toastError(...args),
    warning: (...args: unknown[]) => toastWarning(...args),
  },
}));

function pngFile(name: string): File {
  return new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], name, {
    type: "image/png",
  });
}

describe("ScreenshotsGallery", () => {
  beforeEach(() => {
    uploadMutate.mockReset();
    uploadMutateAsync.mockReset();
    deleteMutate.mockReset();
    invalidateMedia.mockReset();
    toastSuccess.mockReset();
    toastError.mockReset();
    toastWarning.mockReset();
  });

  it("uploads a single screenshot through the plain one-shot path", async () => {
    const user = setupUser();
    const { container } = render(
      <ScreenshotsGallery buildId="build-1" shots={[]} />,
    );

    const input = container.querySelector<HTMLInputElement>("input[type=file]");
    expect(input).not.toBeNull();
    expect(input?.multiple).toBe(true);

    await user.upload(input!, [pngFile("one.png")]);

    expect(uploadMutate).toHaveBeenCalledTimes(1);
    expect(uploadMutate.mock.calls[0][0]).toMatchObject({
      buildId: "build-1",
      role: "screenshot",
    });
    expect(uploadMutateAsync).not.toHaveBeenCalled();
  });

  it("uploads a multi-select sequentially with one summary toast", async () => {
    const resolvers: ((value: unknown) => void)[] = [];
    uploadMutateAsync.mockImplementation(
      () => new Promise((resolve) => resolvers.push(resolve)),
    );

    const user = setupUser();
    const { container } = render(
      <ScreenshotsGallery buildId="build-1" shots={[]} />,
    );
    const input = container.querySelector<HTMLInputElement>("input[type=file]");

    await user.upload(input!, [
      pngFile("one.png"),
      pngFile("two.png"),
      pngFile("three.png"),
    ]);

    // Sequential: the next POST only starts once the previous one settles.
    await waitFor(() => expect(uploadMutateAsync).toHaveBeenCalledTimes(1));
    expect(uploadMutateAsync.mock.calls[0][0]).toMatchObject({
      buildId: "build-1",
      role: "screenshot",
      silent: true,
    });

    resolvers[0]({});
    await waitFor(() => expect(uploadMutateAsync).toHaveBeenCalledTimes(2));
    expect(toastSuccess).not.toHaveBeenCalled();

    resolvers[1]({});
    await waitFor(() => expect(uploadMutateAsync).toHaveBeenCalledTimes(3));

    resolvers[2]({});
    await waitFor(() =>
      expect(toastSuccess).toHaveBeenCalledWith("Uploaded 3 screenshots"),
    );
    expect(toastSuccess).toHaveBeenCalledTimes(1);
    expect(invalidateMedia).toHaveBeenCalledTimes(1);
    expect(invalidateMedia).toHaveBeenCalledWith("build-1");
  });

  it("names the failing file and the server's reason in the summary toast", async () => {
    uploadMutateAsync
      .mockResolvedValueOnce({})
      .mockRejectedValueOnce(new ApiError(413, "file exceeds 32MB"));

    const user = setupUser();
    const { container } = render(
      <ScreenshotsGallery buildId="build-1" shots={[]} />,
    );
    const input = container.querySelector<HTMLInputElement>("input[type=file]");

    await user.upload(input!, [pngFile("one.png"), pngFile("big.png")]);

    await waitFor(() =>
      expect(toastWarning).toHaveBeenCalledWith(
        "Uploaded 1, failed 1: big.png — file exceeds 32MB",
      ),
    );
    expect(toastSuccess).not.toHaveBeenCalled();
  });

  it("stops the batch when the session expires mid-run", async () => {
    uploadMutateAsync
      .mockResolvedValueOnce({})
      .mockRejectedValueOnce(new ApiError(401, "session expired"));

    const user = setupUser();
    const { container } = render(
      <ScreenshotsGallery buildId="build-1" shots={[]} />,
    );
    const input = container.querySelector<HTMLInputElement>("input[type=file]");

    await user.upload(input!, [
      pngFile("one.png"),
      pngFile("two.png"),
      pngFile("three.png"),
      pngFile("four.png"),
    ]);

    await waitFor(() =>
      expect(toastWarning).toHaveBeenCalledWith(
        "Uploaded 1, failed 1: two.png — session expired (2 not attempted)",
      ),
    );
    // The two remaining files are never POSTed.
    expect(uploadMutateAsync).toHaveBeenCalledTimes(2);
  });

  it("removes a screenshot via the hover action", async () => {
    const user = setupUser();
    render(
      <ScreenshotsGallery
        buildId="build-1"
        shots={[{ id: "shot-1", url: "/m/shot-1.png" }]}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: /remove screenshot/i }),
    );
    expect(deleteMutate).toHaveBeenCalledWith({
      buildId: "build-1",
      mediaId: "shot-1",
    });
  });
});
