import { type ClassValue, clsx } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// The palette defines custom font-size tokens (see `--text-*` in index.css).
// tailwind-merge doesn't know them, so by default it lumps e.g. `text-microlabel`
// into the same group as a `text-glass/85` colour and drops the size when both
// appear in one `cn(...)` call. Registering them under `font-size` keeps size and
// colour as independent groups so both survive the merge.
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
