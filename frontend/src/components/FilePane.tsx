import { useCallback, useState, useMemo } from "react";
import type { FileEntry } from "../types/protocol";
import type { Transfer } from "../hooks/useTransfer";
import type { SelectModifiers } from "./FileRow";
import PathBar from "./PathBar";
import FileList, { type SortKey, type SortDirection } from "./FileList";
import TransferBar from "./TransferBar";

interface FilePaneProps {
  label: "local" | "remote";
  hostname: string;
  cwd: string;
  entries: FileEntry[];
  selected: Set<string>;
  onSelect: (name: string, mods: SelectModifiers) => void;
  onNavigate: (path: string) => void;
  onNavigateTo: (absolutePath: string) => void;
  onRefresh: () => void;
  connected?: boolean;
  transfers: Transfer[];
  loading?: boolean;
  fetchSuggestions?: (input: string) => Promise<string[]>;
  onTabComplete?: (input: string) => void;
}

export default function FilePane({
  hostname,
  cwd,
  entries,
  selected,
  onSelect,
  onNavigate,
  onNavigateTo,
  onRefresh,
  connected,
  transfers,
  loading,
  fetchSuggestions,
  onTabComplete,
}: FilePaneProps) {
  const [sortKey, setSortKey] = useState<SortKey>("name");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");

  const handleSort = useCallback((key: SortKey) => {
    if (sortKey === key) {
      setSortDirection((prev) => (prev === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      setSortDirection("asc");
    }
  }, [sortKey]);

  const sortedEntries = useMemo(() => {
    return [...entries].sort((a, b) => {
      // Always directories first
      if (a.is_dir !== b.is_dir) {
        return a.is_dir ? -1 : 1;
      }

      let comparison = 0;
      if (sortKey === "name") {
        comparison = a.name.toLowerCase().localeCompare(b.name.toLowerCase());
      } else if (sortKey === "size") {
        comparison = a.size - b.size;
      } else if (sortKey === "date") {
        comparison = a.modified - b.modified;
      }

      return sortDirection === "asc" ? comparison : -comparison;
    });
  }, [entries, sortKey, sortDirection]);

  const handleGoUp = useCallback(() => {
    const parts = cwd.split("/").filter(Boolean);
    if (parts.length > 1) {
      onNavigate("..");
    }
  }, [cwd, onNavigate]);

  return (
    <div className="flex flex-col h-full bg-zinc-900/30 border border-zinc-800 rounded-lg overflow-hidden">
      <PathBar hostname={hostname} cwd={cwd} connected={connected} onRefresh={onRefresh} onNavigateTo={onNavigateTo} fetchSuggestions={fetchSuggestions} onTabComplete={onTabComplete} />
      {loading ? (
        <div className="flex-1 flex items-center justify-center">
          <div className="w-5 h-5 border-2 border-emerald-400/30 border-t-emerald-400 rounded-full animate-spin" />
        </div>
      ) : (
        <FileList
          entries={sortedEntries}
          selected={selected}
          onSelect={onSelect}
          onNavigate={onNavigate}
          onGoUp={handleGoUp}
          canGoUp={cwd !== "/"}
          sortKey={sortKey}
          sortDirection={sortDirection}
          onSort={handleSort}
        />
      )}
      <TransferBar transfers={transfers} />
    </div>
  );
}
