import { Users } from "lucide-react";

import { PlaceholderPage } from "@/pages/PlaceholderPage";

export function UsersPage() {
  return (
    <PlaceholderPage
      title="Users"
      description="Manage accounts bound to Yggdrasil."
      icon={Users}
    />
  );
}
