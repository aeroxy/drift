import { describe, it, beforeAll, afterAll, expect } from 'vitest';
import * as path from 'path';
import * as fs from 'fs';
import { execSync } from 'child_process';
import { getAvailablePort } from './helpers/ports.js';
import { DriftProcess } from './helpers/drift-process.js';
import { WsBrowserClient } from './helpers/ws-client.js';
import { computeAllChecksums } from './helpers/checksums.js';
import type { FileEntry } from '../src/types/protocol.js';

const PROJECT_ROOT = path.resolve(import.meta.dirname, '../../');
const TEST_RESOURCES = path.join(PROJECT_ROOT, 'test-resources');
const TEST_RESOURCES_BAK = path.join(PROJECT_ROOT, 'test-resources-bak');

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
      // server not ready yet
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`Remote connection not established within ${timeoutMs}ms`);
}

async function browseEntries(baseUrl: string): Promise<FileEntry[]> {
  const res = await fetch(`${baseUrl}/api/browse?path=.`);
  const data: BrowseResponse = await res.json();
  return data.entries;
}

async function pullEntries(ws: WsBrowserClient, entries: FileEntry[]): Promise<void> {
  for (const entry of entries) {
    const id = crypto.randomUUID();
    const timeoutMs = (entry.size > 50_000_000 || entry.is_dir) ? 120_000 : 60_000;

    const transferDone = ws.waitForTransferComplete(id, timeoutMs);

    ws.send({
      type: 'TransferRequest',
      id,
      entries: [{
        relative_path: entry.name,
        size: entry.size,
        is_dir: entry.is_dir,
        permissions: entry.permissions,
      }],
      direction: 'Pull',
    });

    await transferDone;
  }
}

async function pushEntries(ws: WsBrowserClient, entries: FileEntry[]): Promise<void> {
  for (const entry of entries) {
    const id = crypto.randomUUID();
    // Large files (>50MB) get more time; directories also need time for compression
    const timeoutMs = (entry.size > 50_000_000 || entry.is_dir) ? 120_000 : 60_000;

    const transferDone = ws.waitForTransferComplete(id, timeoutMs);

    ws.send({
      type: 'TransferRequest',
      id,
      entries: [{
        relative_path: entry.name,
        size: entry.size,
        is_dir: entry.is_dir,
        permissions: entry.permissions,
      }],
      direction: 'Push',
    });

    await transferDone;
  }
}

// Poll until all expected checksums are present in rootDir, with correct MD5.
// Needed because the receiver may still be writing after the browser WS TransferComplete fires.
async function waitForChecksums(
  rootDir: string,
  expected: Map<string, string>,
  timeoutMs = 60_000,
): Promise<Map<string, string>> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const actual = await computeAllChecksums(rootDir);
    const allPresent = [...expected.keys()].every((k) => actual.has(k));
    if (allPresent) return actual;
    await new Promise((r) => setTimeout(r, 500));
  }
  return computeAllChecksums(rootDir);
}

let host: DriftProcess;
let client: DriftProcess;
let hostChecksums: Map<string, string>;
let clientChecksums: Map<string, string>;
// Initial browse snapshots captured before any transfers, so the client→host
// push only sends the files that were originally there (not ones received from host).
let hostEntries: FileEntry[];
let clientEntries: FileEntry[];

// Safety net: kill drift processes if the process exits unexpectedly
function registerExitHandler() {
  const cleanup = () => {
    host?.stop().catch(() => {});
    client?.stop().catch(() => {});
  };
  process.once('exit', cleanup);
  process.once('SIGINT', cleanup);
  process.once('SIGTERM', cleanup);
}

describe('drift integration', () => {
  beforeAll(async () => {
    // Verify test-resources exists
    if (!fs.existsSync(TEST_RESOURCES)) {
      throw new Error(
        'test-resources/ not found. Create test-resources/host/ (with a subdirectory) ' +
        'and test-resources/client/ (with files) before running tests.'
      );
    }

    // Backup test-resources before anything touches it
    if (fs.existsSync(TEST_RESOURCES_BAK)) {
      fs.rmSync(TEST_RESOURCES_BAK, { recursive: true, force: true });
    }
    execSync(`cp -a ${JSON.stringify(TEST_RESOURCES)} ${JSON.stringify(TEST_RESOURCES_BAK)}`);

    // Snapshot initial checksums BEFORE any transfers
    hostChecksums = await computeAllChecksums(path.join(TEST_RESOURCES, 'host'));
    clientChecksums = await computeAllChecksums(path.join(TEST_RESOURCES, 'client'));

    // Allocate ports
    const hostPort = await getAvailablePort();
    const clientPort = await getAvailablePort();

    // Start drift instances
    host = new DriftProcess({ port: hostPort, cwd: path.join(TEST_RESOURCES, 'host') });
    client = new DriftProcess({
      port: clientPort,
      cwd: path.join(TEST_RESOURCES, 'client'),
      target: `127.0.0.1:${hostPort}`,
    });

    registerExitHandler();

    await host.start();
    await client.start();

    // Wait for both sides to establish the encrypted peer connection
    await Promise.all([
      pollForRemote(host.baseUrl),
      pollForRemote(client.baseUrl),
    ]);

    // Capture browse entries NOW (before any transfers) so each push test
    // only sends the files that originally belonged to that side.
    hostEntries = await browseEntries(host.baseUrl);
    clientEntries = await browseEntries(client.baseUrl);

    expect(hostEntries.length, 'host should have at least one entry').toBeGreaterThan(0);
    expect(clientEntries.length, 'client should have at least one entry').toBeGreaterThan(0);
  }, 120_000);

  afterAll(async () => {
    await Promise.all([host?.stop(), client?.stop()]);

    // Restore test-resources
    try {
      fs.rmSync(TEST_RESOURCES, { recursive: true, force: true });
      if (fs.existsSync(TEST_RESOURCES_BAK)) {
        fs.renameSync(TEST_RESOURCES_BAK, TEST_RESOURCES);
      }
    } catch (err) {
      console.error('Failed to restore test-resources:', err);
    }
  }, 30_000);

  // Pull tests run first to avoid the frame channel being saturated by push data.
  // Pulls don't depend on push — the files already exist on their origin side.

  it('pulls host files to client via browser on client', async () => {
    const ws = await WsBrowserClient.connect(client.wsUrl);
    try {
      await pullEntries(ws, hostEntries);
    } finally {
      ws.close();
    }

    const clientDir = path.join(TEST_RESOURCES, 'client');
    const clientAfter = await waitForChecksums(clientDir, hostChecksums);
    for (const [rel, md5] of hostChecksums) {
      expect(clientAfter.get(rel), `pull: host file "${rel}" missing from client`).toBe(md5);
    }
  }, 120_000);

  it('pulls client files to host via browser on host', async () => {
    const ws = await WsBrowserClient.connect(host.wsUrl);
    try {
      await pullEntries(ws, clientEntries);
    } finally {
      ws.close();
    }

    const hostDir = path.join(TEST_RESOURCES, 'host');
    const hostAfter = await waitForChecksums(hostDir, clientChecksums);
    for (const [rel, md5] of clientChecksums) {
      expect(hostAfter.get(rel), `pull: client file "${rel}" missing from host`).toBe(md5);
    }
  }, 120_000);

  it('pushes host files to client', async () => {
    const ws = await WsBrowserClient.connect(host.wsUrl);
    try {
      await pushEntries(ws, hostEntries);
    } finally {
      ws.close();
    }
  }, 120_000);

  it('pushes client files to host', async () => {
    const ws = await WsBrowserClient.connect(client.wsUrl);
    try {
      await pushEntries(ws, clientEntries);
    } finally {
      ws.close();
    }
  }, 120_000);

  it('verifies transferred file integrity', async () => {
    // Files originally on host should now exist in client with matching MD5.
    // Poll briefly since the receiver may still be finalizing when TransferComplete fires.
    const clientAfter = await waitForChecksums(
      path.join(TEST_RESOURCES, 'client'),
      hostChecksums,
    );
    for (const [rel, md5] of hostChecksums) {
      expect(clientAfter.get(rel), `host file "${rel}" missing from client after transfer`).toBe(md5);
    }

    // Files originally on client should now exist in host with matching MD5.
    const hostAfter = await waitForChecksums(
      path.join(TEST_RESOURCES, 'host'),
      clientChecksums,
    );
    for (const [rel, md5] of clientChecksums) {
      expect(hostAfter.get(rel), `client file "${rel}" missing from host after transfer`).toBe(md5);
    }
  }, 120_000);

  it('verifies .drift temp cleanup', async () => {
    // Both sides should clean up their .drift/ temp dir after transfers.
    // Poll briefly in case finalize is still running.
    const deadline = Date.now() + 30_000;
    for (const side of ['host', 'client'] as const) {
      const driftDir = path.join(TEST_RESOURCES, side, '.drift');
      while (Date.now() < deadline) {
        if (!fs.existsSync(driftDir) || fs.readdirSync(driftDir).length === 0) break;
        await new Promise((r) => setTimeout(r, 500));
      }

      if (!fs.existsSync(driftDir)) continue;
      const remaining = fs.readdirSync(driftDir);
      expect(
        remaining,
        `test-resources/${side}/.drift/ should be empty after transfer, found: ${remaining.join(', ')}`
      ).toHaveLength(0);
    }
  }, 60_000);
});
