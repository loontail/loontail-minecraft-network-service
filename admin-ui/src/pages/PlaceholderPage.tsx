import type { LucideIcon } from "lucide-react";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

interface PlaceholderPageProps {
  title: string;
  description: string;
  icon: LucideIcon;
}

// Stage-1 stub: the real screen (table / dialogs / data wiring) lands in a later
// stage. This keeps the route navigable and on-design until then.
export function PlaceholderPage({
  title,
  description,
  icon: Icon,
}: PlaceholderPageProps) {
  return (
    <div className="flex flex-col gap-6">
      <header>
        <h1 className="text-h1 text-text-hi">{title}</h1>
        <p className="text-body text-text-mute">{description}</p>
      </header>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-h2">
            <Icon className="size-4" />
            Coming soon
          </CardTitle>
          <CardDescription>
            This section is scaffolded and will be wired in a later stage.
          </CardDescription>
        </CardHeader>
        <CardContent className="text-body text-text-mute">
          Nothing to show yet.
        </CardContent>
      </Card>
    </div>
  );
}
