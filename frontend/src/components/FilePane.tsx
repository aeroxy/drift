import { useCallback } from "react";
import type { FileEntry } from "../types/protocol";
import type { Transfer } from "../hooks/useTransfer";
import PathBar from "./PathBar";
import FileList from "./FileList";
import TransferBar from "./TransferBar";

interface FilePaneProps {
  label: "local" | "remote";
  hostname: string;
  cwd: string;
  entries: FileEntry[];
  selected: Set<string>;
  onSelect: (name: string, multi: boolean) => void;
  onNavigate: (path: string) => void;
  onRefresh: () => void;
  connected?: boolean;
  transfers: Transfer[];
  loading?: boolean;
}

export default function FilePane({
  hostname,
  cwd,
  entries,
  selected,
  onSelect,
  onNavigate,
  onRefresh,
  connected,
  transfers,
  loading,
}: FilePaneProps) {
  const handleGoUp = useCallback(() => {
    const parts = cwd.split("/").filter(Boolean);
    if (parts.length > 1) {
      onNavigate("..");
    }
  }, [cwd, onNavigate]);

  return (
    <div className="flex flex-col h-full bg-zinc-900/30 border border-zinc-800 rounded-lg overflow-hidden">
      <PathBar hostname={hostname} cwd={cwd} connected={connected} onRefresh={onRefresh} />
      {loading ? (
        <div className="flex-1 flex items-center justify-center">
          <div className="w-5 h-5 border-2 border-emerald-400/30 border-t-emerald-400 rounded-full animate-spin" />
        </div>
      ) : (
        <FileList
          entries={entries}
          selected={selected}
          onSelect={onSelect}
          onNavigate={onNavigate}
          onGoUp={handleGoUp}
          canGoUp={cwd !== "/"}
        />
      )}
      <TransferBar transfers={transfers} />
    </div>
  );
}
