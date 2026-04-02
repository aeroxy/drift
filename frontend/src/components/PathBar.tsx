import { Monitor, RefreshCw } from "lucide-react";

interface PathBarProps {
  hostname: string;
  cwd: string;
  connected?: boolean;
  onRefresh: () => void;
}

export default function PathBar({ hostname, cwd, connected, onRefresh }: PathBarProps) {
  return (
    <div className="flex items-center gap-2 px-3 py-2 bg-zinc-800/50 border-b border-zinc-700/50 text-sm">
      <Monitor className="w-4 h-4 text-zinc-400 shrink-0" />
      <span className="font-semibold text-emerald-400">{hostname}</span>
      <span className="text-zinc-500">:</span>
      <span className="text-zinc-300 truncate font-mono text-xs">{cwd}</span>
      <button
        onClick={onRefresh}
        className="ml-auto p-1 hover:bg-zinc-700/50 rounded transition-colors"
        title="Refresh"
      >
        <RefreshCw className="w-3.5 h-3.5 text-zinc-400 hover:text-emerald-400" />
      </button>
      {connected !== undefined && (
        <span
          className={`w-2 h-2 rounded-full shrink-0 ${
            connected ? "bg-emerald-400" : "bg-red-400"
          }`}
        />
      )}
    </div>
  );
}
