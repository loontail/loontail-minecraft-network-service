import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import { SegmentedControl } from "@/components/shared/SegmentedControl";

const ITEMS = [
  { value: "a", label: "Alpha" },
  { value: "b", label: "Beta" },
  { value: "c", label: "Gamma" },
] as const;

function Harness({ mode }: { mode?: "tabs" | "radio" }) {
  const [value, setValue] = useState<(typeof ITEMS)[number]["value"]>("a");
  return (
    <SegmentedControl
      mode={mode}
      ariaLabel="test control"
      items={[...ITEMS]}
      value={value}
      onChange={setValue}
    />
  );
}

describe("SegmentedControl roving keyboard nav", () => {
  it("exposes a single tab stop (roving tabIndex)", () => {
    render(<Harness mode="tabs" />);
    const tabs = screen.getAllByRole("tab");
    const focusable = tabs.filter((t) => t.getAttribute("tabindex") === "0");
    expect(focusable).toHaveLength(1);
    expect(focusable[0]).toHaveTextContent("Alpha");
    expect(tabs[1].getAttribute("tabindex")).toBe("-1");
    expect(tabs[2].getAttribute("tabindex")).toBe("-1");
  });

  it("ArrowRight/ArrowLeft move selection and focus", async () => {
    const user = userEvent.setup();
    render(<Harness mode="tabs" />);
    const tabs = screen.getAllByRole("tab");

    tabs[0].focus();
    await user.keyboard("{ArrowRight}");
    expect(tabs[1]).toHaveAttribute("aria-selected", "true");
    expect(tabs[1]).toHaveFocus();
    expect(tabs[1].getAttribute("tabindex")).toBe("0");
    expect(tabs[0].getAttribute("tabindex")).toBe("-1");

    await user.keyboard("{ArrowLeft}");
    expect(tabs[0]).toHaveAttribute("aria-selected", "true");
    expect(tabs[0]).toHaveFocus();
  });

  it("wraps with ArrowLeft from the first item to the last", async () => {
    const user = userEvent.setup();
    render(<Harness mode="tabs" />);
    const tabs = screen.getAllByRole("tab");

    tabs[0].focus();
    await user.keyboard("{ArrowLeft}");
    expect(tabs[2]).toHaveAttribute("aria-selected", "true");
    expect(tabs[2]).toHaveFocus();
  });

  it("Home and End jump to the first and last item", async () => {
    const user = userEvent.setup();
    render(<Harness mode="tabs" />);
    const tabs = screen.getAllByRole("tab");

    tabs[0].focus();
    await user.keyboard("{End}");
    expect(tabs[2]).toHaveAttribute("aria-selected", "true");
    expect(tabs[2]).toHaveFocus();

    await user.keyboard("{Home}");
    expect(tabs[0]).toHaveAttribute("aria-selected", "true");
    expect(tabs[0]).toHaveFocus();
  });

  it("radio mode uses radiogroup/radio roles with aria-checked", async () => {
    const user = userEvent.setup();
    render(<Harness mode="radio" />);
    expect(screen.getByRole("radiogroup")).toBeInTheDocument();
    const radios = screen.getAllByRole("radio");
    expect(radios[0]).toHaveAttribute("aria-checked", "true");

    radios[0].focus();
    await user.keyboard("{ArrowRight}");
    expect(radios[1]).toHaveAttribute("aria-checked", "true");
    expect(radios[1]).toHaveFocus();
  });
});
