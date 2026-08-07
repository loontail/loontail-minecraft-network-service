import { zodResolver } from "@hookform/resolvers/zod";
import { KeyRound, Loader2 } from "lucide-react";
import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { z } from "zod";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
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
import { useResetPassword } from "@/features/users/api";
import type { AdminUser } from "@/shared/types";

const schema = z.object({
  password: z.string().min(8, "Use at least 8 characters"),
});

type FormValues = z.infer<typeof schema>;

export interface ResetPasswordDialogProps {
  user: AdminUser | null;
  onOpenChange: (open: boolean) => void;
}

export function ResetPasswordDialog({
  user,
  onOpenChange,
}: ResetPasswordDialogProps) {
  const resetPassword = useResetPassword();
  const form = useForm<FormValues>({
    resolver: zodResolver(schema),
    defaultValues: { password: "" },
  });

  useEffect(() => {
    if (user) {
      form.reset({ password: "" });
    }
  }, [user, form]);

  async function onSubmit(values: FormValues) {
    if (!user) {
      return;
    }
    // handleSubmit re-throws from the async onValid callback; swallow the rejection (onError already toasts).
    try {
      await resetPassword.mutateAsync({
        id: user.id,
        password: values.password,
      });
      onOpenChange(false);
    } catch {
      // onError toasts
    }
  }

  return (
    <Dialog
      open={Boolean(user)}
      onOpenChange={(next) => !resetPassword.isPending && onOpenChange(next)}
    >
      <DialogContent className="sm:max-w-md">
        <Form {...form}>
          <form
            onSubmit={form.handleSubmit(onSubmit)}
            className="flex flex-col gap-4"
          >
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <KeyRound className="size-5 text-text-hi" />
                Reset password
              </DialogTitle>
              <DialogDescription>
                Set a new password for "{user?.username}". This revokes all
                existing sessions and tokens.
              </DialogDescription>
            </DialogHeader>

            <FormField
              control={form.control}
              name="password"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>New password</FormLabel>
                  <FormControl>
                    <Input
                      type="password"
                      autoComplete="new-password"
                      {...field}
                    />
                  </FormControl>
                  <FormDescription>
                    The user must sign in again everywhere.
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                disabled={resetPassword.isPending}
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button type="submit" disabled={resetPassword.isPending}>
                {resetPassword.isPending && (
                  <Loader2 className="size-4 animate-spin" />
                )}
                Reset password
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
