import { type ClassValue, clsx } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// Register the custom `text-*` size tokens under `font-size` so tailwind-merge keeps
// them in their own group and stops dropping the size when a `text-<color>` is also present.
const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [
        {
          text: [
            "display",
            "h1",
            "h2",
            "body",
            "body-med",
            "eyebrow",
            "caption",
            "microlabel",
            "console-body",
            "console-meta",
            "console-badge",
            "progress-label",
            "progress-value",
          ],
        },
      ],
    },
  },
});

export const cn = (...inputs: ClassValue[]): string => twMerge(clsx(inputs));
