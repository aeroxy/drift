import { useCallback, useState } from "react";
import type { TransferProgress } from "../types/protocol";

export interface Transfer {
  id: string;
  path: string;
  bytes_done: number;
  bytes_total: number;
  status: "pending" | "active" | "complete" | "error";
  error?: string;
}

export function useTransfer() {
  const [transfers, setTransfers] = useState<Map<string, Transfer>>(new Map());

  const startTransfer = useCallback((id: string, path: string, bytes_total: number) => {
    setTransfers((prev) => {
      const next = new Map(prev);
      next.set(id, {
        id,
        path,
        bytes_done: 0,
        bytes_total,
        status: "pending",
      });
      return next;
    });
  }, []);

  const updateProgress = useCallback((progress: TransferProgress) => {
    setTransfers((prev) => {
      const next = new Map(prev);
      next.set(progress.id, {
        id: progress.id,
        path: progress.path,
        bytes_done: progress.bytes_done,
        bytes_total: progress.bytes_total,
        status: "active",
      });
      return next;
    });
  }, []);

  const completeTransfer = useCallback((id: string) => {
    setTransfers((prev) => {
      const next = new Map(prev);
      const existing = next.get(id);
      if (existing) {
        next.set(id, { ...existing, status: "complete", bytes_done: existing.bytes_total });
      }
      return next;
    });
  }, []);

  const failTransfer = useCallback((id: string, error: string) => {
    setTransfers((prev) => {
      const next = new Map(prev);
      const existing = next.get(id);
      if (existing) {
        next.set(id, { ...existing, status: "error", error });
      } else {
        // Create failed transfer if it doesn't exist
        next.set(id, {
          id,
          path: "unknown",
          bytes_done: 0,
          bytes_total: 0,
          status: "error",
          error,
        });
      }
      return next;
    });
  }, []);

  const hasActiveTransfers = transfers.size > 0 && [...transfers.values()].some(
    (t) => t.status === "pending" || t.status === "active"
  );

  return { transfers, startTransfer, updateProgress, completeTransfer, failTransfer, hasActiveTransfers };
}
