import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { FileUpload } from "@/components/shared/FileUpload";

function pngFile(name: string): File {
  return new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], name, {
    type: "image/png",
  });
}

describe("FileUpload", () => {
  it("renders a dropzone with the prompt and hint", () => {
    render(
      <FileUpload onFiles={() => {}} label="Drop image or click">
        PNG, JPG, WebP or GIF
      </FileUpload>,
    );
    expect(screen.getByText("Drop image or click")).toBeInTheDocument();
    expect(screen.getByText(/png, jpg, webp or gif/i)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /drop image or click/i }),
    ).toBeInTheDocument();
  });

  it("calls onFiles with the picked file", async () => {
    const user = userEvent.setup();
    const onFiles = vi.fn();
    const { container } = render(<FileUpload onFiles={onFiles} />);

    const input = container.querySelector<HTMLInputElement>(
      "input[type=file]",
    );
    expect(input).not.toBeNull();
    await user.upload(input!, pngFile("poster.png"));

    expect(onFiles).toHaveBeenCalledTimes(1);
    const files = onFiles.mock.calls[0][0] as File[];
    expect(files).toHaveLength(1);
    expect(files[0].name).toBe("poster.png");
  });

  it("passes every file when multiple are selected", async () => {
    const user = userEvent.setup();
    const onFiles = vi.fn();
    const { container } = render(<FileUpload multiple onFiles={onFiles} />);

    const input = container.querySelector<HTMLInputElement>(
      "input[type=file]",
    );
    await user.upload(input!, [pngFile("a.png"), pngFile("b.png")]);

    expect(onFiles).toHaveBeenCalledTimes(1);
    const files = onFiles.mock.calls[0][0] as File[];
    expect(files.map((f) => f.name)).toEqual(["a.png", "b.png"]);
  });

  it("does not fire when disabled", async () => {
    const user = userEvent.setup();
    const onFiles = vi.fn();
    render(<FileUpload disabled onFiles={onFiles} label="Drop image" />);

    await user.click(screen.getByRole("button", { name: /drop image/i }));
    expect(onFiles).not.toHaveBeenCalled();
  });
});
