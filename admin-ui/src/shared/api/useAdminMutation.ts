import {
  type QueryKey,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { toast } from "sonner";

import { errorMessage } from "@/shared/api/toast";

interface AdminMutationOptions<TData, TVars> {
  mutationFn: (vars: TVars) => Promise<TData>;
  invalidates?: (data: TData, vars: TVars) => QueryKey[] | null;
  success?: string | ((data: TData, vars: TVars) => string | null);
  failure: string | ((vars: TVars) => string | null);
}

// The invalidate-then-toast wiring every admin mutation shares. A message that
// resolves to null stays silent: that is how a batch caller suppresses the
// per-item toast and reports one summary of its own instead.
export function useAdminMutation<TData, TVars>({
  mutationFn,
  invalidates,
  success,
  failure,
}: AdminMutationOptions<TData, TVars>) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn,
    onSuccess: (data, vars) => {
      for (const queryKey of invalidates?.(data, vars) ?? []) {
        qc.invalidateQueries({ queryKey });
      }
      const message =
        typeof success === "function" ? success(data, vars) : success;
      if (message) {
        toast.success(message);
      }
    },
    onError: (error, vars) => {
      const fallback = typeof failure === "function" ? failure(vars) : failure;
      if (fallback) {
        toast.error(errorMessage(error, fallback));
      }
    },
  });
}
