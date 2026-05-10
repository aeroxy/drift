import { ArrowUp, ChevronDown, ChevronUp } from "lucide-react";
import type { FileEntry } from "../types/protocol";
import FileRow, { type SelectModifiers } from "./FileRow";
import { useColumnResize } from "../hooks/useColumnResize";

export type SortKey = "name" | "size" | "date";
export type SortDirection = "asc" | "desc";

interface FileListProps {
  entries: FileEntry[];
  selected: Set<string>;
  onSelect: (name: string, mods: SelectModifiers) => void;
  onNavigate: (name: string) => void;
  onGoUp: () => void;
  canGoUp: boolean;
  sortKey: SortKey;
  sortDirection: SortDirection;
  onSort: (key: SortKey) => void;
}

function SortIcon({ active, direction }: { active: boolean; direction: SortDirection }) {
  if (!active) return null;
  return direction === "asc" ? <ChevronUp className="w-3 h-3 inline ml-1" /> : <ChevronDown className="w-3 h-3 inline ml-1" />;
}

export default function FileList({
  entries,
  selected,
  onSelect,
  onNavigate,
  onGoUp,
  canGoUp,
  sortKey,
  sortDirection,
  onSort,
}: FileListProps) {
  // 64px = w-16, 112px = w-28
  const { width: sizeWidth, onMouseDown: onSizeResizeStart } = useColumnResize(64, 40, 200);
  const { width: dateWidth, onMouseDown: onDateResizeStart } = useColumnResize(112, 60, 300);

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="sticky top-0 z-10 flex items-center gap-3 px-3 py-1.5 bg-[#0a0a0f]/95 border-b border-zinc-800 text-xs text-zinc-500 font-medium select-none backdrop-blur-sm border-l-2 border-transparent">
        <div className="w-[13px]" /> {/* checkbox approx width */}
        <div className="w-4" /> {/* icon width */}
        
        {/* Name Column */}
        <div 
          className="flex-1 cursor-pointer hover:text-zinc-300 flex items-center truncate"
          onClick={() => onSort("name")}
        >
          Name <SortIcon active={sortKey === "name"} direction={sortDirection} />
        </div>

        {/* Size Column */}
        <div className="flex items-center group relative">
          <div 
            className="absolute -left-3 inset-y-0 w-3 cursor-col-resize flex items-center justify-center z-20"
            onMouseDown={onSizeResizeStart}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="w-[1px] h-3/4 bg-zinc-700/50 group-hover:bg-emerald-400 transition-colors" />
          </div>
          <div 
            className="cursor-pointer hover:text-zinc-300 flex items-center justify-end truncate"
            style={{ width: sizeWidth }}
            onClick={() => onSort("size")}
          >
            <SortIcon active={sortKey === "size"} direction={sortDirection} /> Size
          </div>
        </div>

        {/* Date Column */}
        <div className="hidden md:flex items-center group relative">
          <div 
            className="absolute -left-3 inset-y-0 w-3 cursor-col-resize flex items-center justify-center z-20"
            onMouseDown={onDateResizeStart}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="w-[1px] h-3/4 bg-zinc-700/50 group-hover:bg-emerald-400 transition-colors" />
          </div>
          <div 
            className="cursor-pointer hover:text-zinc-300 flex items-center justify-end truncate"
            style={{ width: dateWidth }}
            onClick={() => onSort("date")}
          >
            <SortIcon active={sortKey === "date"} direction={sortDirection} /> Date
          </div>
        </div>
      </div>
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
          sizeWidth={sizeWidth}
          dateWidth={dateWidth}
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
