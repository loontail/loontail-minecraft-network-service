import type { KeyboardEvent } from "react";

import { cn } from "@/shared/lib/cn";

type SwitchProps = {
  checked?: boolean;
  disabled?: boolean;
  onCheckedChange?: (value: boolean) => void;
  id?: string;
  name?: string;
  className?: string;
  "aria-label"?: string;
};

export function Switch({
  checked = false,
  disabled = false,
  onCheckedChange,
  id,
  name,
  className,
  "aria-label": ariaLabel,
}: SwitchProps) {
  const toggle = () => {
    if (!disabled) onCheckedChange?.(!checked);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === " " || event.key === "Enter") {
      event.preventDefault();
      toggle();
    }
  };

  return (
    <button
      type="button"
      role="switch"
      id={id}
      name={name}
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={toggle}
      onKeyDown={onKeyDown}
      className={cn(
        "relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50",
        checked ? "bg-primary" : "bg-input",
        className,
      )}
    >
      <span
        className={cn(
          "pointer-events-none ml-0.5 inline-block size-4 rounded-full bg-background transition-transform",
          checked ? "translate-x-4" : "translate-x-0",
        )}
      />
    </button>
  );
}
