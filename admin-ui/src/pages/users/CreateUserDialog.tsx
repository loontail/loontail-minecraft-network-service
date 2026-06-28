import { zodResolver } from "@hookform/resolvers/zod";
import { CheckCircle2, Copy, Loader2, ShieldCheck, UserPlus } from "lucide-react";
import { useState } from "react";
import { useForm } from "react-hook-form";
import { toast } from "sonner";
import { z } from "zod";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { useCreateUser } from "@/features/users/api";
import type { AdminUser } from "@/shared/types";

const UUID_DASHED = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const UUID_UNDASHED = /^[0-9a-fA-F]{32}$/;

const schema = z.object({
  username: z.string().trim().min(1, "Username is required"),
  email: z.string().trim().min(1, "Email is required").email("Enter a valid email"),
  password: z.string().min(8, "Use at least 8 characters"),
  minecraftUuid: z
    .string()
    .trim()
    .refine(
      (value) => value === "" || UUID_DASHED.test(value) || UUID_UNDASHED.test(value),
      "Enter a valid Minecraft UUID (dashed or 32 hex chars)",
    ),
  isAdmin: z.boolean(),
});

type FormValues = z.infer<typeof schema>;

const EMPTY: FormValues = {
  username: "",
  email: "",
  password: "",
  minecraftUuid: "",
  isAdmin: false,
};

export function CreateUserDialog() {
  const [open, setOpen] = useState(false);
  const [created, setCreated] = useState<AdminUser | null>(null);
  const createUser = useCreateUser();

  const form = useForm<FormValues>({
    resolver: zodResolver(schema),
    defaultValues: EMPTY,
  });

  function reset() {
    form.reset(EMPTY);
    setCreated(null);
  }

  function handleOpenChange(next: boolean) {
    if (createUser.isPending) {
      return;
    }
    setOpen(next);
    if (!next) {
      reset();
    }
  }

  async function onSubmit(values: FormValues) {
    // handleSubmit re-throws from the async onValid callback; swallow the rejection (onError already toasts).
    try {
      const user = await createUser.mutateAsync({
        username: values.username.trim(),
        email: values.email.trim(),
        password: values.password,
        minecraftUuid: values.minecraftUuid.trim() || null,
        isAdmin: values.isAdmin,
      });
      setCreated(user);
    } catch {
      // onError toasts
    }
  }

  async function copyProfileUuid() {
    if (!created?.profileUuid) {
      return;
    }
    try {
      await navigator.clipboard.writeText(created.profileUuid);
      toast.success("Profile UUID copied");
    } catch {
      toast.error("Clipboard unavailable");
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogTrigger asChild>
        <Button>
          <UserPlus className="size-4" />
          Create user
        </Button>
      </DialogTrigger>
      <DialogContent className="sm:max-w-lg">
        {created ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <CheckCircle2 className="size-5 text-text-hi" />
                User created
              </DialogTitle>
              <DialogDescription>
                "{created.username}" is bound to Yggdrasil and ready to sign in.
              </DialogDescription>
            </DialogHeader>

            <div className="rounded-md border border-edge bg-surface-1 p-4">
              <p className="text-caption text-text-faint">Assigned profile UUID</p>
              <div className="mt-1 flex items-center justify-between gap-3">
                <code className="truncate font-mono text-body text-text-hi">
                  {created.profileUuid ?? "—"}
                </code>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={copyProfileUuid}
                  disabled={!created.profileUuid}
                >
                  <Copy className="size-4" />
                  Copy
                </Button>
              </div>
            </div>

            <DialogFooter>
              <Button type="button" variant="outline" onClick={reset}>
                Create another
              </Button>
              <Button type="button" onClick={() => handleOpenChange(false)}>
                Done
              </Button>
            </DialogFooter>
          </>
        ) : (
          <Form {...form}>
            <form
              onSubmit={form.handleSubmit(onSubmit)}
              className="flex flex-col gap-4"
            >
              <DialogHeader>
                <DialogTitle className="flex items-center gap-2">
                  <ShieldCheck className="size-5 text-text-hi" />
                  Create user bound to Yggdrasil
                </DialogTitle>
                <DialogDescription>
                  Creates an admin-origin, confirmed account with a profile UUID
                  assigned at creation.
                </DialogDescription>
              </DialogHeader>

              <FormField
                control={form.control}
                name="username"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Username</FormLabel>
                    <FormControl>
                      <Input autoComplete="off" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="email"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Email</FormLabel>
                    <FormControl>
                      <Input type="email" autoComplete="off" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="password"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Password</FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        autoComplete="new-password"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="minecraftUuid"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>
                      Minecraft UUID
                      <span className="text-text-faint">(optional)</span>
                    </FormLabel>
                    <FormControl>
                      <Input
                        placeholder="Leave blank to generate a fresh profile UUID"
                        autoComplete="off"
                        {...field}
                      />
                    </FormControl>
                    <FormDescription>
                      When supplied, the profile UUID is derived from it.
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <Separator />

              <FormField
                control={form.control}
                name="isAdmin"
                render={({ field }) => (
                  <FormItem className="flex-row items-center justify-between gap-4">
                    <div className="grid gap-1">
                      <FormLabel htmlFor="create-user-is-admin">
                        Administrator
                      </FormLabel>
                      <FormDescription>
                        Grants access to this admin console.
                      </FormDescription>
                    </div>
                    <FormControl>
                      <Switch
                        id="create-user-is-admin"
                        name={field.name}
                        checked={field.value}
                        onCheckedChange={field.onChange}
                        aria-label="Administrator"
                      />
                    </FormControl>
                  </FormItem>
                )}
              />

              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  disabled={createUser.isPending}
                  onClick={() => handleOpenChange(false)}
                >
                  Cancel
                </Button>
                <Button type="submit" disabled={createUser.isPending}>
                  {createUser.isPending && (
                    <Loader2 className="size-4 animate-spin" />
                  )}
                  Create user
                </Button>
              </DialogFooter>
            </form>
          </Form>
        )}
      </DialogContent>
    </Dialog>
  );
}
