/**
 * path-traversal.test.ts
 *
 * Security regression test: a TransferRequest with a `destination_path` that
 * escapes the served root (via `..` or an absolute path) must be rejected before
 * any file I/O. Without validation, a malicious browser or peer could write files
 * outside root_dir — especially dangerous for a password-less server, where any
 * peer can Push. Covers both receiver entry points:
 *   - Pull  → start_transfer_with_notify (receiver = the requesting server)
 *   - Push  → start_transfer            (receiver = the remote server)
 */
import { describe, it, beforeAll, afterAll, expect } from 'vitest';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';
import { getAvailablePort } from './helpers/ports.js';
import { DriftProcess } from './helpers/drift-process.js';
import { WsBrowserClient } from './helpers/ws-client.js';
import type { FileEntry } from '../src/types/protocol.js';

interface BrowseResponse {
  hostname: string;
  cwd: string;
  entries: FileEntry[];
}
interface InfoResponse {
  has_remote: boolean;
}

async function pollForRemote(baseUrl: string, timeoutMs = 15_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${baseUrl}/api/info`);
      const info: InfoResponse = await res.json();
      if (info.has_remote) return;
    } catch {
      // not ready yet
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`Remote not connected within ${timeoutMs}ms`);
}

async function browseRoot(baseUrl: string): Promise<FileEntry[]> {
  const res = await fetch(`${baseUrl}/api/browse?path=${encodeURIComponent('.')}`);
  const data: BrowseResponse = await res.json();
  return data.entries;
}

/** Send a transfer with an explicit (possibly malicious) destination_path. */
function transfer(
  ws: WsBrowserClient,
  direction: 'Pull' | 'Push',
  relativePath: string,
  destinationPath: string,
  entry: FileEntry,
): Promise<void> {
  const id = crypto.randomUUID();
  const done = ws.waitForTransferComplete(id, 15_000);
  ws.send({
    type: 'TransferRequest',
    id,
    entries: [{
      relative_path: relativePath,
      size: entry.size,
      is_dir: entry.is_dir,
      permissions: entry.permissions,
    }],
    direction,
    destination_path: destinationPath,
  });
  return done;
}

describe('destination_path traversal is rejected', () => {
  let host: DriftProcess;
  let client: DriftProcess;
  let tmpRoot: string;
  let hostRoot: string;
  let clientRoot: string;

  beforeAll(async () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'drift-traversal-'));
    hostRoot = path.join(tmpRoot, 'host');
    clientRoot = path.join(tmpRoot, 'client');
    fs.mkdirSync(hostRoot, { recursive: true });
    fs.mkdirSync(clientRoot, { recursive: true });
    fs.writeFileSync(path.join(hostRoot, 'host-file.txt'), 'host-secret');
    fs.writeFileSync(path.join(clientRoot, 'client-file.txt'), 'client-secret');

    const hostPort = await getAvailablePort();
    const clientPort = await getAvailablePort();
    host = new DriftProcess({ port: hostPort, cwd: hostRoot });
    client = new DriftProcess({ port: clientPort, cwd: clientRoot, target: `127.0.0.1:${hostPort}` });

    await host.start();
    await client.start();
    await Promise.all([pollForRemote(host.baseUrl), pollForRemote(client.baseUrl)]);
  }, 60_000);

  afterAll(async () => {
    await Promise.all([host?.stop(), client?.stop()]);
    fs.rmSync(tmpRoot, { recursive: true, force: true });
  }, 30_000);

  // Both validation branches: a parent-traversal (`..`) path and an absolute path.
  // For the absolute case the destination_path IS an absolute path pointing outside
  // the receiver's root (a sibling of the served dir under tmpRoot).
  const escapeKinds = [
    { label: 'a parent-traversal path (..)', kind: 'rel' as const },
    { label: 'an absolute path', kind: 'abs' as const },
  ];

  it.each(escapeKinds)(
    'rejects a Pull whose destination escapes the local root via $label',
    async ({ kind }) => {
      const entry = (await browseRoot(host.baseUrl)).find((e) => e.name === 'host-file.txt');
      expect(entry).toBeDefined();

      const leaf = `pwned-pull-${kind}`;
      const escapeTarget = path.join(tmpRoot, leaf); // clientRoot/.. = tmpRoot
      const dest = kind === 'abs' ? escapeTarget : `../${leaf}`;

      const ws = await WsBrowserClient.connect(client.wsUrl);
      try {
        // Receiver = client (start_transfer_with_notify). Must reject, not write outside root.
        await expect(transfer(ws, 'Pull', 'host-file.txt', dest, entry!)).rejects.toThrow();
      } finally {
        ws.close();
      }
      expect(fs.existsSync(escapeTarget)).toBe(false);
    },
    30_000,
  );

  it.each(escapeKinds)(
    'rejects a Push whose destination escapes the remote root via $label',
    async ({ kind }) => {
      const entry = (await browseRoot(client.baseUrl)).find((e) => e.name === 'client-file.txt');
      expect(entry).toBeDefined();

      const leaf = `pwned-push-${kind}`;
      const escapeTarget = path.join(tmpRoot, leaf); // hostRoot/.. = tmpRoot
      const dest = kind === 'abs' ? escapeTarget : `../${leaf}`;

      const ws = await WsBrowserClient.connect(client.wsUrl);
      try {
        // Receiver = host (start_transfer). Must reject, not write outside root.
        await expect(transfer(ws, 'Push', 'client-file.txt', dest, entry!)).rejects.toThrow();
      } finally {
        ws.close();
      }
      expect(fs.existsSync(escapeTarget)).toBe(false);
    },
    30_000,
  );
});
