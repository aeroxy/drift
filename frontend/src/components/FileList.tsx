import { ArrowUp } from "lucide-react";
import type { FileEntry } from "../types/protocol";
import FileRow from "./FileRow";

interface FileListProps {
  entries: FileEntry[];
  selected: Set<string>;
  onSelect: (name: string, multi: boolean) => void;
  onNavigate: (name: string) => void;
  onGoUp: () => void;
  canGoUp: boolean;
}

export default function FileList({
  entries,
  selected,
  onSelect,
  onNavigate,
  onGoUp,
  canGoUp,
}: FileListProps) {
  return (
    <div className="flex-1 overflow-y-auto">
      {canGoUp && (
        <div
          className="flex items-center gap-3 px-3 py-1.5 cursor-pointer hover:bg-zinc-800/50 border-l-2 border-transparent"
          onClick={onGoUp}
        >
          <span className="w-4" />
          <ArrowUp className="w-4 h-4 text-zinc-500" />
          <span className="text-sm text-zinc-500">..</span>
        </div>
      )}
      {entries.map((entry) => (
        <FileRow
          key={entry.name}
          entry={entry}
          selected={selected.has(entry.name)}
          onSelect={onSelect}
          onNavigate={onNavigate}
        />
      ))}
      {entries.length === 0 && (
        <div className="flex items-center justify-center h-32 text-zinc-600 text-sm">
          Empty directory
        </div>
      )}
    </div>
  );
}
