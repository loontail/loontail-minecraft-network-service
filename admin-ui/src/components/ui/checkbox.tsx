import { Check } from "lucide-react";

import { cn } from "@/shared/lib/cn";

type CheckboxProps = {
  checked?: boolean;
  disabled?: boolean;
  // when omitted the checkbox is decorative and the surrounding control owns the
  // accessible role/state (e.g. a row button with role="checkbox")
  onCheckedChange?: (value: boolean) => void;
  id?: string;
  className?: string;
  "aria-label"?: string;
};

export function Checkbox({
  checked = false,
  disabled = false,
  onCheckedChange,
  id,
  className,
  "aria-label": ariaLabel,
}: CheckboxProps) {
  const interactive = typeof onCheckedChange === "function";

  const classes = cn(
    "flex size-4 shrink-0 items-center justify-center rounded-[4px] border transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
    checked
      ? "border-primary bg-primary text-primary-foreground"
      : "border-edge-lg bg-surface-2",
    disabled && "opacity-50",
    interactive && !disabled && "cursor-pointer",
    interactive && disabled && "cursor-not-allowed",
    className,
  );

  const icon = checked ? <Check className="size-3" strokeWidth={3} /> : null;

  if (!interactive) {
    return (
      <span aria-hidden className={classes}>
        {icon}
      </span>
    );
  }

  return (
    <button
      type="button"
      role="checkbox"
      id={id}
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => {
        if (!disabled) onCheckedChange(!checked);
      }}
      className={classes}
    >
      {icon}
    </button>
  );
}
