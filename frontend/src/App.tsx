import { useCallback, useEffect, useState } from "react";
import type { BrowseResponse, ControlMessage, FileEntry, InfoResponse, TransferEntry } from "./types/protocol";
import { useWebSocket } from "./hooks/useWebSocket";
import { useTransfer } from "./hooks/useTransfer";
import FilePane from "./components/FilePane";
import Toolbar from "./components/Toolbar";

export default function App() {
  // Local state
  const [localInfo, setLocalInfo] = useState<{ hostname: string; cwd: string }>({
    hostname: "...",
    cwd: "...",
  });
  const [localEntries, setLocalEntries] = useState<FileEntry[]>([]);
  const [localPath, setLocalPath] = useState(".");
  const [localSelected, setLocalSelected] = useState<Set<string>>(new Set());
  const [localLoading, setLocalLoading] = useState(true);

  // Remote state
  const [remoteInfo, setRemoteInfo] = useState<{ hostname: string; cwd: string }>({
    hostname: "...",
    cwd: "...",
  });
  const [remoteEntries, setRemoteEntries] = useState<FileEntry[]>([]);
  const [remoteSelected, setRemoteSelected] = useState<Set<string>>(new Set());
  const [hasRemote, setHasRemote] = useState(false);

  // Error notification
  const [error, setError] = useState<string | null>(null);

  const { transfers, startTransfer, updateProgress, completeTransfer, failTransfer, hasActiveTransfers } = useTransfer();

  // Fetch local file listing via REST
  const fetchLocal = useCallback(async (path: string) => {
    setLocalLoading(true);
    try {
      const res = await fetch(`/api/browse?path=${encodeURIComponent(path)}`);
      if (res.ok) {
        const data: BrowseResponse = await res.json();
        setLocalInfo({ hostname: data.hostname, cwd: data.cwd });
        setLocalEntries(data.entries);
        setLocalSelected(new Set());
      }
    } finally {
      setLocalLoading(false);
    }
  }, []);

  // Fetch remote status and clear stale remote state when disconnected
  const fetchRemoteStatus = useCallback(async () => {
    try {
      const r = await fetch("/api/info");
      const info: InfoResponse = await r.json();
      setHasRemote(info.has_remote);
      if (!info.has_remote) {
        setRemoteEntries([]);
        setRemoteInfo({ hostname: "...", cwd: "..." });
        setRemoteSelected(new Set());
      }
    } catch {
      // ignore
    }
  }, []);

  // Initial remote status fetch on mount
  useEffect(() => {
    fetchRemoteStatus();
  }, [fetchRemoteStatus]);

  // Initial local load
  useEffect(() => {
    fetchLocal(localPath);
  }, [localPath, fetchLocal]);

  const { connected, send } = useWebSocket((msg: ControlMessage) => {
    switch (msg.type) {
      case "BrowseResponse":
        setRemoteInfo({ hostname: msg.hostname, cwd: msg.cwd });
        setRemoteEntries(msg.entries);
        setRemoteSelected(new Set());
        break;
      case "ConnectionStatus":
        setHasRemote(msg.has_remote);
        if (!msg.has_remote) {
          setRemoteEntries([]);
          setRemoteInfo({ hostname: "...", cwd: "..." });
          setRemoteSelected(new Set());
        }
        break;
      case "TransferProgress":
        updateProgress(msg);
        break;
      case "TransferComplete":
        completeTransfer(msg.id);
        // Refresh both panes after transfer
        fetchLocal(localPath);
        send({ type: "BrowseRequest", path: "." });
        break;
      case "TransferError":
        failTransfer(msg.id, msg.error);
        setError(msg.error);
        setTimeout(() => setError(null), 5000);
        break;
      case "Error":
        setError(msg.message);
        setTimeout(() => setError(null), 5000);
        break;
    }
  });

  // On WS reconnect, re-check remote status (may have changed while browser WS was down)
  useEffect(() => {
    if (connected) {
      fetchRemoteStatus();
    }
  }, [connected, fetchRemoteStatus]);

  // Refresh remote file listing
  const refreshRemote = useCallback(() => {
    if (connected && hasRemote) {
      send({ type: "BrowseRequest", path: "." });
    }
  }, [connected, hasRemote, send]);

  // Request remote browse when we have a remote
  useEffect(() => {
    if (connected && hasRemote) {
      send({ type: "BrowseRequest", path: "." });
    }
  }, [connected, hasRemote, send]);

  // Selection handlers
  const handleLocalSelect = useCallback((name: string, multi: boolean) => {
    setLocalSelected((prev) => {
      const next = new Set(multi ? prev : []);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  }, []);

  const handleRemoteSelect = useCallback((name: string, multi: boolean) => {
    setRemoteSelected((prev) => {
      const next = new Set(multi ? prev : []);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  }, []);

  // Navigation
  const handleLocalNavigate = useCallback(
    (name: string) => {
      const newPath = localPath === "." ? name : `${localPath}/${name}`;
      setLocalPath(name === ".." ? localPath.split("/").slice(0, -1).join("/") || "." : newPath);
    },
    [localPath],
  );

  const handleRemoteNavigate = useCallback(
    (name: string) => {
      send({ type: "BrowseRequest", path: name === ".." ? ".." : name });
    },
    [send],
  );

  // Transfer actions
  const handleCopyToRemote = useCallback(() => {
    console.log("Copy to remote clicked", { hasRemote, localSelected: localSelected.size, hasActiveTransfers });
    if (!hasRemote || localSelected.size === 0 || hasActiveTransfers) return;

    // Get selected file entries
    const selectedEntries = localEntries.filter((e) => localSelected.has(e.name));

    // Create transfer request
    let transferId: string;
    try {
      transferId = crypto.randomUUID();
    } catch (e) {
      setError("Your browser doesn't support secure random IDs. Please use a modern browser or access via HTTPS.");
      return;
    }
    const transferEntries: TransferEntry[] = selectedEntries.map((e) => ({
      relative_path: e.name,
      size: e.size,
      is_dir: e.is_dir,
      permissions: e.permissions,
    }));

    // Start transfer tracking
    const totalBytes = selectedEntries.reduce((sum, e) => sum + e.size, 0);
    const paths = selectedEntries.map((e) => e.name).join(", ");
    startTransfer(transferId, paths, totalBytes);

    const msg = {
      type: "TransferRequest",
      id: transferId,
      entries: transferEntries,
      direction: "Push",
    } as const;

    console.log("Sending transfer request:", msg);
    send(msg);

    // Clear selection
    setLocalSelected(new Set());
  }, [hasRemote, localSelected, localEntries, send, hasActiveTransfers, startTransfer]);

  const handleCopyToLocal = useCallback(() => {
    if (!hasRemote || remoteSelected.size === 0 || hasActiveTransfers) return;

    // Get selected file entries
    const selectedEntries = remoteEntries.filter((e) => remoteSelected.has(e.name));

    // Create transfer request
    let transferId: string;
    try {
      transferId = crypto.randomUUID();
    } catch (e) {
      setError("Your browser doesn't support secure random IDs. Please use a modern browser or access via HTTPS.");
      return;
    }
    const transferEntries: TransferEntry[] = selectedEntries.map((e) => ({
      relative_path: e.name,
      size: e.size,
      is_dir: e.is_dir,
      permissions: e.permissions,
    }));

    // Start transfer tracking
    const totalBytes = selectedEntries.reduce((sum, e) => sum + e.size, 0);
    const paths = selectedEntries.map((e) => e.name).join(", ");
    startTransfer(transferId, paths, totalBytes);

    send({
      type: "TransferRequest",
      id: transferId,
      entries: transferEntries,
      direction: "Pull",
    });

    // Clear selection
    setRemoteSelected(new Set());
  }, [hasRemote, remoteSelected, remoteEntries, send, hasActiveTransfers, startTransfer]);

  const activeTransfers = [...transfers.values()];

  return (
    <div className="h-screen flex flex-col bg-[#0a0a0f]">
      {/* Header */}
      <header className="flex items-center justify-between px-4 py-3 border-b border-zinc-800/50">
        <div className="flex items-center gap-3">
          <img src="/logo.svg" alt="drift" className="h-6 invert-0 brightness-0 invert" style={{ filter: "brightness(0) invert(1) sepia(1) saturate(5) hue-rotate(120deg)" }} />
          <span className="text-xs text-zinc-600 font-mono">v0.1.0</span>
        </div>
      </header>

      {/* Error notification */}
      {error && (
        <div className="mx-2 mt-2 px-4 py-2 bg-red-500/10 border border-red-500/50 rounded text-red-400 text-sm font-mono">
          {error}
        </div>
      )}

      {/* Toolbar */}
      <Toolbar
        connected={connected}
        hasRemote={hasRemote}
        localSelected={localSelected.size}
        remoteSelected={remoteSelected.size}
        onCopyToRemote={handleCopyToRemote}
        onCopyToLocal={handleCopyToLocal}
        transferring={hasActiveTransfers}
      />

      {/* Two-pane layout */}
      <div className="flex-1 grid grid-cols-2 gap-2 px-2 pb-2 min-h-0">
        <FilePane
          label="local"
          hostname={localInfo.hostname}
          cwd={localInfo.cwd}
          entries={localEntries}
          selected={localSelected}
          onSelect={handleLocalSelect}
          onNavigate={handleLocalNavigate}
          onRefresh={() => fetchLocal(localPath)}
          transfers={activeTransfers.filter(() => true)}
          loading={localLoading}
        />
        <FilePane
          label="remote"
          hostname={remoteInfo.hostname}
          cwd={remoteInfo.cwd}
          entries={remoteEntries}
          selected={remoteSelected}
          onSelect={handleRemoteSelect}
          onNavigate={handleRemoteNavigate}
          onRefresh={refreshRemote}
          connected={hasRemote ? connected : undefined}
          transfers={[]}
          loading={false}
        />
      </div>
    </div>
  );
}
