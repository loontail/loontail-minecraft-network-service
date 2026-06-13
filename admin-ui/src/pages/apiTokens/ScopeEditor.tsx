import { Checkbox } from "@/components/ui/checkbox";
import { cn } from "@/shared/lib/cn";

export const WILDCARD = "*";

const RESOURCES = [
  {
    key: "catalog",
    label: "Catalog",
    description: "Clients, keywords, and servers",
  },
  { key: "bundles", label: "Bundles", description: "Builds and manifests" },
] as const;

const ACTIONS = ["read", "write"] as const;

export function ScopeEditor({
  value,
  onChange,
}: {
  value: string[];
  onChange: (scopes: string[]) => void;
}) {
  const allowAll = value.includes(WILDCARD);
  const has = (resource: string, action: string) =>
    allowAll || value.includes(`${resource}:${action}`);

  const setAllowAll = (next: boolean) => onChange(next ? [WILDCARD] : []);

  const toggle = (resource: string, action: string, next: boolean) => {
    const scope = `${resource}:${action}`;
    const set = new Set(value.filter((entry) => entry !== WILDCARD));
    if (next) {
      set.add(scope);
    } else {
      set.delete(scope);
    }
    onChange([...set]);
  };

  return (
    <div className="overflow-hidden rounded-md border border-edge bg-surface-1">
      <button
        type="button"
        role="checkbox"
        aria-checked={allowAll}
        aria-label="Full access"
        onClick={() => setAllowAll(!allowAll)}
        className="flex w-full items-center justify-between gap-3 px-3 py-2.5 text-left transition-colors hover:bg-ghost"
      >
        <div className="grid gap-0.5">
          <span className="text-body-med text-text-hi">Full access</span>
          <span className="text-caption text-text-mute">
            Grants every scope (<code className="font-mono">*</code>) — read and
            write on all resources.
          </span>
        </div>
        <Checkbox checked={allowAll} />
      </button>

      <div className="border-t border-edge">
        {RESOURCES.map((resource) => (
          <div
            key={resource.key}
            className="flex items-center justify-between gap-3 border-b border-edge/60 px-3 py-2.5 last:border-b-0"
          >
            <div className="grid gap-0.5">
              <span className="text-body text-text">{resource.label}</span>
              <span className="text-caption text-text-faint">
                {resource.description}
              </span>
            </div>
            <div className="flex items-center gap-4">
              {ACTIONS.map((action) => (
                <button
                  key={action}
                  type="button"
                  role="checkbox"
                  aria-checked={has(resource.key, action)}
                  aria-label={`${resource.label} ${action}`}
                  disabled={allowAll}
                  onClick={() =>
                    toggle(resource.key, action, !has(resource.key, action))
                  }
                  className={cn(
                    "flex items-center gap-1.5 text-caption capitalize transition-colors",
                    allowAll
                      ? "cursor-not-allowed text-text-faint"
                      : "cursor-pointer text-text-mute hover:text-text",
                  )}
                >
                  <Checkbox checked={has(resource.key, action)} disabled={allowAll} />
                  {action}
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
