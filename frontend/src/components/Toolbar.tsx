import { ArrowLeft, ArrowRight, Loader2, Plug, PlugZap, RotateCw, Unplug, Wifi, WifiOff, X } from "lucide-react";

interface ToolbarProps {
  connected: boolean;
  hasRemote: boolean;
  fingerprint: string | null;
  remoteHostname?: string;
  localSelected: number;
  remoteSelected: number;
  onCopyToRemote: () => void;
  onCopyToLocal: () => void;
  onClearLocal?: () => void;
  onClearRemote?: () => void;
  transferring?: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
  connecting?: boolean;
  canReconnect?: boolean;
  lastTarget?: string | null;
  onReconnect?: () => void;
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
  onClearLocal,
  onClearRemote,
  transferring = false,
  onConnect,
  onDisconnect,
  connecting = false,
  canReconnect = false,
  lastTarget = null,
  onReconnect,
}: ToolbarProps) {
  return (
    <div className="flex items-center justify-center gap-3 py-3">
      {remoteSelected > 0 && onClearRemote && (
        <button
          onClick={onClearRemote}
          title="Clear remote selection"
          className="flex items-center text-zinc-500 hover:text-red-400 transition-colors"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      )}
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
        Copy{remoteSelected > 0 ? ` (${remoteSelected})` : ""}
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
            {canReconnect && onReconnect && (
              <button
                onClick={onReconnect}
                title={lastTarget ? `Reconnect to ${lastTarget}` : "Reconnect to last target"}
                className="flex items-center gap-1 text-xs text-amber-400/90 hover:text-amber-300 transition-colors"
              >
                <RotateCw className="w-3.5 h-3.5" />
                Reconnect
              </button>
            )}
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
        Copy{localSelected > 0 ? ` (${localSelected})` : ""}
        {transferring ? (
          <Loader2 className="w-4 h-4 animate-spin" />
        ) : (
          <ArrowRight className="w-4 h-4" />
        )}
      </button>
      {localSelected > 0 && onClearLocal && (
        <button
          onClick={onClearLocal}
          title="Clear local selection"
          className="flex items-center text-zinc-500 hover:text-red-400 transition-colors"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      )}
    </div>
  );
}
