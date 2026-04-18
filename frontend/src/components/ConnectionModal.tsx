import { useState } from "react";
import { Loader2, Plug } from "lucide-react";

interface ConnectionModalProps {
  onSubmit: (target: string, password?: string) => void;
  onCancel: () => void;
  error?: string;
  connecting?: boolean;
}

export default function ConnectionModal({ onSubmit, onCancel, error, connecting }: ConnectionModalProps) {
  const [target, setTarget] = useState("");
  const [password, setPassword] = useState("");

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50 backdrop-blur-sm">
      <div className="bg-zinc-900 border border-zinc-700 rounded-xl p-6 w-full max-w-sm shadow-2xl">
        <div className="flex items-center gap-3 mb-4">
          <div className="p-2 rounded-lg bg-emerald-500/10">
            <Plug className="w-5 h-5 text-emerald-400" />
          </div>
          <div>
            <h2 className="text-lg font-semibold text-zinc-100">Connect to remote</h2>
            <p className="text-xs text-zinc-500">Enter the address of a drift server</p>
          </div>
        </div>

        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (!target.trim()) return;
            onSubmit(target.trim(), password || undefined);
          }}
        >
          <input
            type="text"
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            placeholder="192.168.0.2:8000"
            autoFocus
            disabled={connecting}
            className="w-full px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-emerald-500 focus:ring-1 focus:ring-emerald-500/20 mb-3 disabled:opacity-50"
          />

          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="Password (optional)"
            disabled={connecting}
            className="w-full px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-emerald-500 focus:ring-1 focus:ring-emerald-500/20 mb-4 disabled:opacity-50"
          />

          {error && (
            <p className="text-xs text-red-400 mb-3 font-mono">{error}</p>
          )}

          <div className="flex gap-2 justify-end">
            <button
              type="button"
              onClick={onCancel}
              disabled={connecting}
              className="px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200 transition-colors disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={connecting || !target.trim()}
              className="flex items-center gap-1.5 px-4 py-1.5 text-sm font-medium bg-emerald-500 hover:bg-emerald-400 text-zinc-900 rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {connecting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
              Connect
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
