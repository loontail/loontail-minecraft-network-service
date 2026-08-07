import { toast } from "sonner";

import { ApiError } from "@/shared/api/client";
import { errorMessage } from "@/shared/api/toast";

export interface BatchUploadOutcome {
  ok: number;
  failures: { name: string; message: string }[];
  notAttempted: string[];
}

// why: 401/403 is about the session, not the file, so every remaining upload would
// fail the same way — stop instead of firing N doomed multipart POSTs.
function stopsTheRun(error: unknown): boolean {
  return (
    error instanceof ApiError && (error.status === 401 || error.status === 403)
  );
}

// Upload one file at a time (parallel POSTs on a single mutation instance make
// `isPending` track only the last call and race server-side ordering) and keep the
// reason for every rejection, which the caller's summary needs.
export async function uploadSequentially(
  files: File[],
  send: (file: File) => Promise<unknown>,
): Promise<BatchUploadOutcome> {
  const outcome: BatchUploadOutcome = { ok: 0, failures: [], notAttempted: [] };
  for (const [index, file] of files.entries()) {
    try {
      await send(file);
      outcome.ok += 1;
    } catch (error) {
      outcome.failures.push({
        name: file.name,
        message: errorMessage(error, "upload failed"),
      });
      if (stopsTheRun(error)) {
        outcome.notAttempted = files.slice(index + 1).map((f) => f.name);
        break;
      }
    }
  }
  return outcome;
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

export function batchUploadSummary(
  outcome: BatchUploadOutcome,
  noun: string,
): string {
  const { ok, failures, notAttempted } = outcome;
  if (failures.length === 0) {
    return `Uploaded ${plural(ok, noun)}`;
  }
  const first = failures[0];
  const reason = `${first.name} — ${first.message}`;
  const skipped =
    notAttempted.length > 0 ? ` (${notAttempted.length} not attempted)` : "";
  if (ok === 0) {
    return `Failed to upload ${plural(failures.length, noun)}: ${reason}${skipped}`;
  }
  return `Uploaded ${ok}, failed ${failures.length}: ${reason}${skipped}`;
}

export function toastBatchUpload(
  outcome: BatchUploadOutcome,
  noun: string,
): void {
  const message = batchUploadSummary(outcome, noun);
  if (outcome.failures.length === 0) {
    toast.success(message);
  } else if (outcome.ok === 0) {
    toast.error(message);
  } else {
    toast.warning(message);
  }
}
