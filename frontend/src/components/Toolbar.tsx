import { ArrowLeft, ArrowRight, Loader2, Plug, PlugZap, Unplug, Wifi, WifiOff } from "lucide-react";

interface ToolbarProps {
  connected: boolean;
  hasRemote: boolean;
  fingerprint: string | null;
  remoteHostname?: string;
  localSelected: number;
  remoteSelected: number;
  onCopyToRemote: () => void;
  onCopyToLocal: () => void;
  transferring?: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  connecting?: boolean;
}

export default function Toolbar({
  connected,
  hasRemote,
  fingerprint,
  remoteHostname,
  localSelected,
  remoteSelected,
  onCopyToRemote,
  onCopyToLocal,
  transferring = false,
  onConnect,
  onDisconnect,
  connecting = false,
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
        {hasRemote ? (
          <>
            {connected ? (
              <Wifi className="w-4 h-4 text-emerald-400" />
            ) : (
              <WifiOff className="w-4 h-4 text-yellow-400" />
            )}
            <span className="text-xs text-zinc-400">
              {remoteHostname ?? "remote"}
            </span>
            {fingerprint && (
              <span
                className="text-xs font-mono text-amber-400/80"
                title="Connection fingerprint — verify this matches the remote terminal"
              >
                {fingerprint}
              </span>
            )}
            <button
              onClick={onDisconnect}
              title="Disconnect"
              className="ml-1 text-zinc-500 hover:text-red-400 transition-colors"
            >
              <Unplug className="w-3.5 h-3.5" />
            </button>
          </>
        ) : connecting ? (
          <>
            <PlugZap className="w-4 h-4 text-emerald-400 animate-pulse" />
            <span className="text-xs text-zinc-400">Connecting…</span>
          </>
        ) : (
          <>
            <Plug className="w-4 h-4 text-zinc-500" />
            <button
              onClick={onConnect}
              className="text-xs text-zinc-400 hover:text-emerald-400 transition-colors"
            >
              Connect to remote
            </button>
          </>
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
