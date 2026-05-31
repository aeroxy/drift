/**
 * readonly-root-transfer.test.ts
 *
 * Regression test for the bug where a browser-initiated pull failed with
 * "Read-only file system (os error 30)" when drift was launched from a
 * read-only directory (e.g. `/`).
 *
 * The receiver used to stage its `.drift` temp archive under the *served root*
 * (`root_dir/.drift`). When the root was read-only — but the chosen destination
 * subdirectory was writable — staging failed and the transfer hung forever
 * ("0/N bytes received ... waiting for remaining"). The fix stages `.drift`
 * under the destination directory instead.
 *
 * This reproduces the exact shape: receiver root is chmod 0o555 (read-only),
 * with a pre-created writable destination subdir. A pull into that subdir must
 * succeed.
 *
 * Unix-only: relies on POSIX directory permission semantics.
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
  hostname: string;
  root_dir: string;
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

async function browseDir(baseUrl: string, dir: string): Promise<FileEntry[]> {
  const res = await fetch(`${baseUrl}/api/browse?path=${encodeURIComponent(dir)}`);
  const data: BrowseResponse = await res.json();
  return data.entries;
}

/** Pull a remote entry into `destinationPath` (relative to the local root). */
async function pull(
  ws: WsBrowserClient,
  remotePath: string,
  destinationPath: string,
  entry: FileEntry,
): Promise<void> {
  const id = crypto.randomUUID();
  const done = ws.waitForTransferComplete(id, 30_000);
  ws.send({
    type: 'TransferRequest',
    id,
    entries: [{
      relative_path: remotePath,
      size: entry.size,
      is_dir: entry.is_dir,
      permissions: entry.permissions,
    }],
    direction: 'Pull',
    destination_path: destinationPath,
  });
  await done;
}

const itUnix = process.platform === 'win32' ? it.skip : it;

describe('pull into writable destination under a read-only root', () => {
  let host: DriftProcess; // sender (remote)
  let client: DriftProcess; // receiver — root is read-only
  let tmpRoot: string;
  let senderRoot: string;
  let receiverRoot: string;
  const DEST = 'writable-dest';

  beforeAll(async () => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'drift-ro-root-'));
    senderRoot = path.join(tmpRoot, 'sender');
    receiverRoot = path.join(tmpRoot, 'receiver');

    // Sender: a directory to pull (mirrors the `.git` folder from the report)
    // plus a top-level file, both under a writable root.
    fs.mkdirSync(path.join(senderRoot, 'payload', 'nested'), { recursive: true });
    fs.writeFileSync(path.join(senderRoot, 'payload', 'a.txt'), 'payload-a');
    fs.writeFileSync(path.join(senderRoot, 'payload', 'nested', 'b.txt'), 'payload-b');
    fs.writeFileSync(path.join(senderRoot, 'loose.txt'), 'loose-file');

    // Receiver: a writable destination subdir, then the root itself made
    // read-only so any attempt to stage `.drift` at the root would fail.
    fs.mkdirSync(path.join(receiverRoot, DEST), { recursive: true });
    fs.chmodSync(receiverRoot, 0o555);

    const hostPort = await getAvailablePort();
    const clientPort = await getAvailablePort();
    host = new DriftProcess({ port: hostPort, cwd: senderRoot });
    client = new DriftProcess({ port: clientPort, cwd: receiverRoot, target: `127.0.0.1:${hostPort}` });

    await host.start();
    await client.start();
    await Promise.all([pollForRemote(host.baseUrl), pollForRemote(client.baseUrl)]);
  }, 60_000);

  afterAll(async () => {
    await Promise.all([host?.stop(), client?.stop()]);
    // Restore write permission so cleanup can remove the tree.
    try {
      fs.chmodSync(receiverRoot, 0o755);
    } catch {
      // best-effort
    }
    fs.rmSync(tmpRoot, { recursive: true, force: true });
  }, 30_000);

  itUnix('pulls a directory into the writable destination (not the read-only root)', async () => {
    const entries = await browseDir(host.baseUrl, '.');
    const payload = entries.find((e) => e.name === 'payload');
    expect(payload, 'sender should expose the payload directory').toBeDefined();
    expect(payload!.is_dir).toBe(true);

    const ws = await WsBrowserClient.connect(client.wsUrl);
    try {
      await pull(ws, 'payload', DEST, payload!);
    } finally {
      ws.close();
    }

    // Files land under the destination subdir, with their tree intact.
    expect(fs.readFileSync(path.join(receiverRoot, DEST, 'payload', 'a.txt'), 'utf-8')).toBe('payload-a');
    expect(fs.readFileSync(path.join(receiverRoot, DEST, 'payload', 'nested', 'b.txt'), 'utf-8')).toBe('payload-b');
    // Nothing was (or could be) staged at the read-only root.
    expect(fs.existsSync(path.join(receiverRoot, '.drift'))).toBe(false);
    // The staging dir under the destination is cleaned up after finalize.
    expect(fs.existsSync(path.join(receiverRoot, DEST, '.drift'))).toBe(false);
  }, 60_000);

  itUnix('pulls a single file into the writable destination', async () => {
    const entries = await browseDir(host.baseUrl, '.');
    const loose = entries.find((e) => e.name === 'loose.txt');
    expect(loose, 'sender should expose loose.txt').toBeDefined();

    const ws = await WsBrowserClient.connect(client.wsUrl);
    try {
      await pull(ws, 'loose.txt', DEST, loose!);
    } finally {
      ws.close();
    }

    expect(fs.readFileSync(path.join(receiverRoot, DEST, 'loose.txt'), 'utf-8')).toBe('loose-file');
    expect(fs.existsSync(path.join(receiverRoot, '.drift'))).toBe(false);
    expect(fs.existsSync(path.join(receiverRoot, DEST, '.drift'))).toBe(false);
  }, 60_000);
});
