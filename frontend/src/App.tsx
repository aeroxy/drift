import { useCallback, useEffect, useRef, useState } from "react";
import type { BrowseResponse, ConnectResponse, ControlMessage, FileEntry, InfoResponse, TransferEntry } from "./types/protocol";
import { useWebSocket } from "./hooks/useWebSocket";
import { useTransfer } from "./hooks/useTransfer";
import ConnectionModal from "./components/ConnectionModal";
import FilePane from "./components/FilePane";
import Toolbar from "./components/Toolbar";
import type { SelectModifiers } from "./components/FileRow";

export default function App() {
  // Local state
  const [localInfo, setLocalInfo] = useState<{ hostname: string; cwd: string; rootDir: string }>({
    hostname: "...",
    cwd: "...",
    rootDir: "",
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
  const [remotePath, setRemotePath] = useState(".");
  const [remoteSelected, setRemoteSelected] = useState<Set<string>>(new Set());
  const [hasRemote, setHasRemote] = useState(false);
  const [fingerprint, setFingerprint] = useState<string | null>(null);
  const [remoteHostname, setRemoteHostname] = useState<string | undefined>(undefined);
  const [canReconnect, setCanReconnect] = useState(false);
  const [lastTarget, setLastTarget] = useState<string | null>(null);

  // Connection modal state
  const [showConnectModal, setShowConnectModal] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [connectError, setConnectError] = useState<string | undefined>(undefined);

  // Error notification
  const [error, setError] = useState<string | null>(null);

  // Tracks the last successfully browsed remote path (for reverting on WS Error)
  const lastGoodRemotePathRef = useRef(".");

  // Range-select anchors: index of the last single-clicked entry per pane.
  // null after refreshes/path-changes so the next click re-establishes the anchor.
  const lastClickedLocalRef = useRef<number | null>(null);
  const lastClickedRemoteRef = useRef<number | null>(null);
  const localRootDirRef = useRef("");

  const { transfers, startTransfer, updateProgress, completeTransfer, failTransfer, hasActiveTransfers } = useTransfer();

  // Fetch local file listing via REST
  const fetchLocal = useCallback(async (path: string): Promise<boolean> => {
    setLocalLoading(true);
    try {
      const root = localRootDirRef.current;
      let relative = path;
      if (root && path.startsWith(root)) {
        relative = path.slice(root.length).replace(/^\//, "");
      }
      const res = await fetch(`/api/browse?path=${encodeURIComponent(relative)}`);
      if (res.ok) {
        const data: BrowseResponse = await res.json();
        setLocalInfo({ hostname: data.hostname, cwd: data.cwd, rootDir: localRootDirRef.current });
        setLocalEntries(data.entries);
        setLocalSelected(new Set());
        lastClickedLocalRef.current = null;
        return true;
      }
      return false;
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
      setFingerprint(info.fingerprint ?? null);
      setCanReconnect(info.can_reconnect);
      setLastTarget(info.last_target);
      localRootDirRef.current = info.root_dir;
      setLocalInfo((prev) => ({ ...prev, rootDir: info.root_dir }));
      if (!info.has_remote) {
        setRemoteEntries([]);
        setRemoteInfo({ hostname: "...", cwd: "..." });
        setRemoteHostname(undefined);
        setRemotePath(".");
        setRemoteSelected(new Set());
        lastClickedRemoteRef.current = null;
      }
    } catch {
      // ignore
    }
  }, []);

  const handleConnect = useCallback(async (target: string, password?: string) => {
    setConnecting(true);
    setConnectError(undefined);
    try {
      const res = await fetch("/api/connect", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ target, password: password ?? null }),
      });
      const data: ConnectResponse = await res.json();
      if (data.success) {
        setShowConnectModal(false);
        setFingerprint(data.fingerprint ?? null);
        // The ConnectionStatus WS event was broadcast before the browser's WS
        // entered the listening path, so poll /api/info directly instead.
        await fetchRemoteStatus();
      } else {
        setConnectError(data.error ?? "Connection failed");
      }
    } catch {
      setConnectError("Network error");
    } finally {
      setConnecting(false);
    }
  }, []);

  const handleDisconnect = useCallback(async () => {
    try {
      await fetch("/api/disconnect", { method: "POST" });
      // ConnectionStatus { has_remote: false } arrives via WebSocket
    } catch {
      // ignore
    }
  }, []);

  const handleReconnect = useCallback(async () => {
    setConnecting(true);
    try {
      const res = await fetch("/api/reconnect", { method: "POST" });
      const data: ConnectResponse = await res.json();
      if (data.success) {
        setFingerprint(data.fingerprint ?? null);
        await fetchRemoteStatus();
      } else {
        setError(data.error ?? "Reconnect failed");
        setTimeout(() => setError(null), 5000);
      }
    } catch {
      setError("Network error");
      setTimeout(() => setError(null), 5000);
    } finally {
      setConnecting(false);
    }
  }, [fetchRemoteStatus]);

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
        lastGoodRemotePathRef.current = msg.cwd;
        setRemoteInfo({ hostname: msg.hostname, cwd: msg.cwd });
        setRemoteHostname(msg.hostname);
        setRemoteEntries(msg.entries);
        setRemoteSelected(new Set());
        lastClickedRemoteRef.current = null;
        break;
      case "ConnectionStatus":
        setHasRemote(msg.has_remote);
        if (!msg.has_remote) {
          setRemoteEntries([]);
          setRemoteInfo({ hostname: "...", cwd: "..." });
          setRemoteHostname(undefined);
          setRemotePath(".");
          setRemoteSelected(new Set());
          lastClickedRemoteRef.current = null;
          setFingerprint(null);
        }
        fetchRemoteStatus();
        break;
      case "TransferProgress":
        updateProgress(msg);
        break;
      case "TransferComplete":
        completeTransfer(msg.id);
        // Refresh both panes after transfer
        fetchLocal(localPath);
        send({ type: "BrowseRequest", path: remotePath });
        break;
      case "TransferError":
        failTransfer(msg.id, msg.error);
        setError(msg.error);
        setTimeout(() => setError(null), 5000);
        break;
      case "Error":
        setError(msg.message);
        setRemotePath(lastGoodRemotePathRef.current);
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
      send({ type: "BrowseRequest", path: remotePath });
    }
  }, [connected, hasRemote, remotePath, send]);

  // Request remote browse when we have a remote
  useEffect(() => {
    if (connected && hasRemote) {
      setRemotePath(".");
      send({ type: "BrowseRequest", path: "." });
    }
  }, [connected, hasRemote, send]);

  // Selection handlers
  // - plain click: replace set with just this item, set anchor
  // - cmd/ctrl-click (multi): toggle this item in the existing set, set anchor
  // - shift-click (range): add the slice [anchor..idx] to the existing set; anchor unchanged
  const handleLocalSelect = useCallback(
    (name: string, mods: SelectModifiers) => {
      const idx = localEntries.findIndex((e) => e.name === name);
      if (idx < 0) return;
      setLocalSelected((prev) => {
        if (mods.range) {
          const anchor = lastClickedLocalRef.current ?? idx;
          const [lo, hi] = anchor < idx ? [anchor, idx] : [idx, anchor];
          const next = new Set(prev);
          for (let i = lo; i <= hi; i++) next.add(localEntries[i].name);
          return next;
        }
        const next = new Set(mods.multi ? prev : []);
        if (next.has(name)) next.delete(name);
        else next.add(name);
        lastClickedLocalRef.current = idx;
        return next;
      });
    },
    [localEntries],
  );

  const handleRemoteSelect = useCallback(
    (name: string, mods: SelectModifiers) => {
      const idx = remoteEntries.findIndex((e) => e.name === name);
      if (idx < 0) return;
      setRemoteSelected((prev) => {
        if (mods.range) {
          const anchor = lastClickedRemoteRef.current ?? idx;
          const [lo, hi] = anchor < idx ? [anchor, idx] : [idx, anchor];
          const next = new Set(prev);
          for (let i = lo; i <= hi; i++) next.add(remoteEntries[i].name);
          return next;
        }
        const next = new Set(mods.multi ? prev : []);
        if (next.has(name)) next.delete(name);
        else next.add(name);
        lastClickedRemoteRef.current = idx;
        return next;
      });
    },
    [remoteEntries],
  );

  const handleClearLocal = useCallback(() => {
    setLocalSelected(new Set());
    lastClickedLocalRef.current = null;
  }, []);

  const handleClearRemote = useCallback(() => {
    setRemoteSelected(new Set());
    lastClickedRemoteRef.current = null;
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
      let newPath: string;
      if (name === "..") {
        newPath = remotePath === "." ? "." : remotePath.split("/").slice(0, -1).join("/") || ".";
      } else {
        newPath = remotePath === "." ? name : `${remotePath}/${name}`;
      }
      setRemotePath(newPath);
      send({ type: "BrowseRequest", path: newPath });
    },
    [remotePath, send],
  );

  const handleLocalNavigateTo = useCallback(
    async (absolutePath: string) => {
      const root = localRootDirRef.current;
      const relative = root && absolutePath.startsWith(root)
        ? absolutePath.slice(root.length).replace(/^\//, "")
        : absolutePath;
      const success = await fetchLocal(relative);
      if (success) {
        setLocalPath(relative);
      } else {
        setError(`Path not found: ${absolutePath}`);
        setTimeout(() => setError(null), 5000);
      }
    },
    [fetchLocal],
  );

  const handleRemoteNavigateTo = useCallback(
    (absolutePath: string) => {
      if (!connected || !hasRemote) return;
      setRemotePath(absolutePath);
      send({ type: "BrowseRequest", path: absolutePath });
    },
    [connected, hasRemote, send],
  );

  const fetchLocalSuggestions = useCallback(async (inputValue: string): Promise<string[]> => {
    const lastSlash = inputValue.lastIndexOf("/");
    const parentDir = lastSlash > 0 ? inputValue.slice(0, lastSlash) : "/";
    const prefix = inputValue.slice(lastSlash + 1).toLowerCase();
    const root = localRootDirRef.current;
    // Use cached entries when browsing the current local cwd
    if (parentDir === localInfo.cwd || inputValue === localInfo.cwd) {
      return localEntries
        .filter((e) => e.is_dir && e.name.toLowerCase().startsWith(prefix))
        .map((e) => `${localInfo.cwd}/${e.name}`);
    }
    let relParent: string;
    if (root && parentDir.startsWith(root)) {
      relParent = parentDir.slice(root.length).replace(/^\//, "") || ".";
    } else if (root && inputValue.startsWith(root)) {
      // inputValue is at or below root_dir, but parentDir is outside — browse root_dir
      relParent = ".";
    } else if (root) {
      return []; // completely outside root_dir — no suggestions
    } else {
      relParent = parentDir;
    }
    try {
      const res = await fetch(`/api/browse?path=${encodeURIComponent(relParent)}`);
      if (!res.ok) return [];
      const data: BrowseResponse = await res.json();
      return data.entries
        .filter((e) => e.is_dir && e.name.toLowerCase().startsWith(prefix))
        .map((e) => `${data.cwd}/${e.name}`);
    } catch {
      return [];
    }
  }, [localInfo.cwd, localEntries]);

  // Remote suggestions — fetch from remote server via REST (no WS side effects)
  const fetchRemoteSuggestions = useCallback(async (inputValue: string): Promise<string[]> => {
    if (!connected || !hasRemote) return [];
    const lastSlash = inputValue.lastIndexOf("/");
    const parentDir = lastSlash > 0 ? inputValue.slice(0, lastSlash) : "/";
    const prefix = inputValue.slice(lastSlash + 1).toLowerCase();
    // Use cached entries when browsing the current remote cwd
    if (parentDir === remoteInfo.cwd || inputValue === remoteInfo.cwd) {
      return remoteEntries
        .filter((e) => e.is_dir && e.name.toLowerCase().startsWith(prefix))
        .map((e) => `${remoteInfo.cwd}/${e.name}`);
    }
    try {
      const res = await fetch(`/api/browse-remote?path=${encodeURIComponent(parentDir)}`);
      if (!res.ok) return [];
      const data: BrowseResponse = await res.json();
      return data.entries
        .filter((e) => e.is_dir && e.name.toLowerCase().startsWith(prefix))
        .map((e) => `${data.cwd}/${e.name}`);
    } catch {
      return [];
    }
  }, [connected, hasRemote, remoteInfo.cwd, remoteEntries]);

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
      relative_path: localPath === "." ? e.name : `${localPath}/${e.name}`,
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
      destination_path: remotePath,
    } as const;

    console.log("Sending transfer request:", msg);
    send(msg);

    // Clear selection
    setLocalSelected(new Set());
    lastClickedLocalRef.current = null;
  }, [hasRemote, localSelected, localEntries, localPath, remotePath, send, hasActiveTransfers, startTransfer]);

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
      relative_path: remotePath === "." ? e.name : `${remotePath}/${e.name}`,
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
      destination_path: localPath,
    });

    // Clear selection
    setRemoteSelected(new Set());
    lastClickedRemoteRef.current = null;
  }, [hasRemote, remoteSelected, remoteEntries, remotePath, localPath, send, hasActiveTransfers, startTransfer]);

  const activeTransfers = [...transfers.values()];

  return (
    <div className="h-screen flex flex-col bg-[#0a0a0f]">
      {/* Header */}
      <header className="flex items-center justify-between px-4 py-3 border-b border-zinc-800/50">
        <div className="flex items-center gap-3">
          <img src="/logo.svg" alt="drift" className="h-6 invert-0 brightness-0 invert" style={{ filter: "brightness(0) invert(1) sepia(1) saturate(5) hue-rotate(120deg)" }} />
          <span className="text-xs text-zinc-600 font-mono">v0.4.2</span>
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
        fingerprint={fingerprint}
        remoteHostname={remoteHostname}
        localSelected={localSelected.size}
        remoteSelected={remoteSelected.size}
        onCopyToRemote={handleCopyToRemote}
        onCopyToLocal={handleCopyToLocal}
        onClearLocal={handleClearLocal}
        onClearRemote={handleClearRemote}
        transferring={hasActiveTransfers}
        onConnect={() => { setConnectError(undefined); setShowConnectModal(true); }}
        onDisconnect={handleDisconnect}
        connecting={connecting}
        canReconnect={canReconnect}
        lastTarget={lastTarget}
        onReconnect={handleReconnect}
      />

      {/* Connection modal */}
      {showConnectModal && (
        <ConnectionModal
          onSubmit={handleConnect}
          onCancel={() => { setShowConnectModal(false); setConnectError(undefined); }}
          error={connectError}
          connecting={connecting}
        />
      )}

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
          onNavigateTo={handleLocalNavigateTo}
          onRefresh={() => fetchLocal(localPath)}
          fetchSuggestions={fetchLocalSuggestions}
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
          onNavigateTo={handleRemoteNavigateTo}
          onRefresh={refreshRemote}
          connected={hasRemote ? connected : undefined}
          transfers={[]}
          loading={false}
          fetchSuggestions={fetchRemoteSuggestions}
        />
      </div>
    </div>
  );
}
