import { useState } from "react";
import { Lock } from "lucide-react";

interface ConnectionModalProps {
  onSubmit: (password: string) => void;
  onCancel: () => void;
}

export default function ConnectionModal({ onSubmit, onCancel }: ConnectionModalProps) {
  const [password, setPassword] = useState("");

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50 backdrop-blur-sm">
      <div className="bg-zinc-900 border border-zinc-700 rounded-xl p-6 w-full max-w-sm shadow-2xl">
        <div className="flex items-center gap-3 mb-4">
          <div className="p-2 rounded-lg bg-emerald-500/10">
            <Lock className="w-5 h-5 text-emerald-400" />
          </div>
          <div>
            <h2 className="text-lg font-semibold text-zinc-100">Authentication</h2>
            <p className="text-xs text-zinc-500">Remote requires a password</p>
          </div>
        </div>

        <form
          onSubmit={(e) => {
            e.preventDefault();
            onSubmit(password);
          }}
        >
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="Enter password"
            autoFocus
            className="w-full px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-emerald-500 focus:ring-1 focus:ring-emerald-500/20 mb-4"
          />

          <div className="flex gap-2 justify-end">
            <button
              type="button"
              onClick={onCancel}
              className="px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200 transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="px-4 py-1.5 text-sm font-medium bg-emerald-500 hover:bg-emerald-400 text-zinc-900 rounded-lg transition-colors"
            >
              Connect
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
