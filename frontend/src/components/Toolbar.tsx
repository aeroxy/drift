import { ArrowLeft, ArrowRight, Wifi, WifiOff, Loader2 } from "lucide-react";

interface ToolbarProps {
  connected: boolean;
  hasRemote: boolean;
  fingerprint: string | null;
  localSelected: number;
  remoteSelected: number;
  onCopyToRemote: () => void;
  onCopyToLocal: () => void;
  transferring?: boolean;
}

export default function Toolbar({
  connected,
  hasRemote,
  fingerprint,
  localSelected,
  remoteSelected,
  onCopyToRemote,
  onCopyToLocal,
  transferring = false,
}: ToolbarProps) {
  return (
    <div className="flex items-center justify-center gap-3 py-3">
      <button
        onClick={onCopyToLocal}
        disabled={!hasRemote || remoteSelected === 0 || transferring}
        className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-md bg-zinc-800 hover:bg-zinc-700 disabled:opacity-30 disabled:cursor-not-allowed transition-colors border border-zinc-700"
      >
        {transferring ? (
          <Loader2 className="w-4 h-4 animate-spin" />
        ) : (
          <ArrowLeft className="w-4 h-4" />
        )}
        Copy
      </button>

      <div className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-zinc-800/50 border border-zinc-700/50">
        {connected ? (
          <Wifi className="w-4 h-4 text-emerald-400" />
        ) : (
          <WifiOff className="w-4 h-4 text-red-400" />
        )}
        <span className="text-xs text-zinc-400">
          {hasRemote ? (connected ? "Connected" : "Reconnecting...") : "No remote"}
        </span>
        {fingerprint && hasRemote && (
          <span className="text-xs font-mono text-amber-400/80" title="Connection fingerprint — verify this matches the remote terminal">
            {fingerprint}
          </span>
        )}
      </div>

      <button
        onClick={onCopyToRemote}
        disabled={!hasRemote || localSelected === 0 || transferring}
        className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-md bg-zinc-800 hover:bg-zinc-700 disabled:opacity-30 disabled:cursor-not-allowed transition-colors border border-zinc-700"
      >
        Copy
        {transferring ? (
          <Loader2 className="w-4 h-4 animate-spin" />
        ) : (
          <ArrowRight className="w-4 h-4" />
        )}
      </button>
    </div>
  );
}
