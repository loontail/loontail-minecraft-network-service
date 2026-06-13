import { ApiError } from "@/shared/api/client";

/// Pull a human-readable message out of any thrown value, preferring the typed
/// `ApiError` envelope message the backend returns.
export function errorMessage(error: unknown, fallback = "Something went wrong"): string {
  if (error instanceof ApiError) {
    return error.message;
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return fallback;
}
