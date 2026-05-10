import {
  Folder,
  File,
  FileCode,
  FileImage,
  FileText,
  FileArchive,
} from "lucide-react";
import type { FileEntry } from "../types/protocol";

export type SelectModifiers = { multi: boolean; range: boolean };

interface FileRowProps {
  entry: FileEntry;
  selected: boolean;
  onSelect: (name: string, mods: SelectModifiers) => void;
  onNavigate: (name: string) => void;
  sizeWidth: number;
  dateWidth: number;
}

function getIcon(entry: FileEntry) {
  if (entry.is_dir) return <Folder className="w-4 h-4 text-amber-400" />;

  const ext = entry.name.split(".").pop()?.toLowerCase() ?? "";
  if (["ts", "tsx", "js", "jsx", "rs", "py", "go", "rb", "java", "c", "cpp", "h"].includes(ext))
    return <FileCode className="w-4 h-4 text-blue-400" />;
  if (["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"].includes(ext))
    return <FileImage className="w-4 h-4 text-purple-400" />;
  if (["md", "txt", "doc", "pdf", "csv"].includes(ext))
    return <FileText className="w-4 h-4 text-green-400" />;
  if (["zip", "tar", "gz", "bz2", "xz", "7z", "rar"].includes(ext))
    return <FileArchive className="w-4 h-4 text-orange-400" />;

  return <File className="w-4 h-4 text-zinc-400" />;
}

function formatSize(bytes: number): string {
  if (bytes === 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

function formatDate(timestamp: number): string {
  if (timestamp === 0) return "—";
  return new Date(timestamp * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default function FileRow({ entry, selected, onSelect, onNavigate, sizeWidth, dateWidth }: FileRowProps) {
  return (
    <div
      className={`flex items-center gap-3 px-3 py-1.5 cursor-pointer transition-colors group ${
        selected
          ? "bg-emerald-500/10 border-l-2 border-emerald-400"
          : "border-l-2 border-transparent hover:bg-zinc-800/50"
      }`}
      onClick={(e) => {
        const multi = e.metaKey || e.ctrlKey;
        const range = e.shiftKey;
        if (entry.is_dir && !multi && !range) {
          onNavigate(entry.name);
        } else {
          onSelect(entry.name, { multi, range });
        }
      }}
      onDoubleClick={() => {
        if (entry.is_dir) onNavigate(entry.name);
      }}
    >
      <input
        type="checkbox"
        checked={selected}
        onChange={() => onSelect(entry.name, { multi: true, range: false })}
        onClick={(e) => e.stopPropagation()}
        className="accent-emerald-400"
      />
      {getIcon(entry)}
      <span
        className={`flex-1 truncate text-sm ${
          entry.is_dir ? "text-amber-200 font-medium" : "text-zinc-300"
        }`}
      >
        {entry.name}
      </span>
      <span className="text-xs text-zinc-500 text-right tabular-nums" style={{ width: sizeWidth }}>
        {entry.is_dir ? "" : formatSize(entry.size)}
      </span>
      <span className="text-xs text-zinc-600 text-right hidden md:block truncate" style={{ width: dateWidth }}>
        {formatDate(entry.modified)}
      </span>
    </div>
  );
}
