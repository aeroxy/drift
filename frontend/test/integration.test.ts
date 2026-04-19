import { describe, it, beforeAll, afterAll, expect } from 'vitest';
import * as path from 'path';
import * as fs from 'fs';
import { execSync } from 'child_process';
import { getAvailablePort } from './helpers/ports.js';
import { DriftProcess, runDriftCli } from './helpers/drift-process.js';
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
      destination_path: '.',
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
      destination_path: '.',
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

  it('lists remote files via CLI ls', () => {
    const target = `127.0.0.1:${host.port}`;
    const output = runDriftCli(['ls', '--target', target]);

    // Should contain the hostname header line and at least one entry name
    for (const entry of hostEntries) {
      expect(output, `ls output should contain "${entry.name}"`).toContain(entry.name);
    }
  }, 30_000);

  it('lists remote subdirectory via CLI ls', () => {
    // Find a directory entry on the host to browse into
    const dirEntry = hostEntries.find((e) => e.is_dir);
    if (!dirEntry) {
      console.warn('Skipping subdirectory ls test — no directories in host test-resources');
      return;
    }

    const target = `127.0.0.1:${host.port}`;
    const output = runDriftCli(['ls', '--target', target, dirEntry.name]);

    // Should contain the hostname:cwd header
    expect(output).toContain(dirEntry.name);
  }, 30_000);

  it('pulls a file via CLI pull', async () => {
    // Find a non-directory entry on the host
    const fileEntry = hostEntries.find((e) => !e.is_dir);
    if (!fileEntry) {
      console.warn('Skipping CLI pull test — no files in host test-resources');
      return;
    }

    const tmpDir = fs.mkdtempSync(path.join(PROJECT_ROOT, 'test-resources', '.pull-test-'));
    try {
      const target = `127.0.0.1:${host.port}`;
      runDriftCli(['pull', '--target', target, fileEntry.name, '--output', tmpDir], {
        timeoutMs: 60_000,
      });

      const pulledPath = path.join(tmpDir, fileEntry.name);
      expect(fs.existsSync(pulledPath), `pulled file should exist at ${pulledPath}`).toBe(true);

      // Verify checksum matches the original
      const originalChecksums = await computeAllChecksums(path.join(TEST_RESOURCES, 'host'));
      const pulledChecksums = await computeAllChecksums(tmpDir);
      const originalMd5 = originalChecksums.get(fileEntry.name);
      const pulledMd5 = pulledChecksums.get(fileEntry.name);
      expect(pulledMd5, `pulled file MD5 should match original`).toBe(originalMd5);
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  }, 60_000);

  it('pulls a directory via CLI pull', async () => {
    const dirEntry = hostEntries.find((e) => e.is_dir);
    if (!dirEntry) {
      console.warn('Skipping CLI pull directory test — no directories in host test-resources');
      return;
    }

    const tmpDir = fs.mkdtempSync(path.join(PROJECT_ROOT, 'test-resources', '.pull-test-'));
    try {
      const target = `127.0.0.1:${host.port}`;
      runDriftCli(['pull', '--target', target, dirEntry.name, '--output', tmpDir], {
        timeoutMs: 120_000,
      });

      const pulledPath = path.join(tmpDir, dirEntry.name);
      expect(fs.existsSync(pulledPath), `pulled directory should exist at ${pulledPath}`).toBe(true);

      // Verify checksums of all files within the directory
      const originalChecksums = await computeAllChecksums(path.join(TEST_RESOURCES, 'host', dirEntry.name));
      const pulledChecksums = await computeAllChecksums(pulledPath);
      for (const [rel, md5] of originalChecksums) {
        expect(pulledChecksums.get(rel), `pulled file "${rel}" should match original`).toBe(md5);
      }
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  }, 120_000);

  it('browses to an absolute path via REST /api/browse', async () => {
    // Get the host's resolved absolute cwd
    const rootRes = await fetch(`${host.baseUrl}/api/browse?path=.`);
    const rootData: BrowseResponse = await rootRes.json();
    const absoluteCwd = rootData.cwd;

    // Browsing by absolute path should return the same result as browsing by "."
    const res = await fetch(`${host.baseUrl}/api/browse?path=${encodeURIComponent(absoluteCwd)}`);
    expect(res.ok, `absolute path browse should succeed for "${absoluteCwd}"`).toBe(true);
    const data: BrowseResponse = await res.json();
    expect(data.cwd).toBe(absoluteCwd);
    expect(data.entries.length).toBe(rootData.entries.length);
  }, 15_000);

  it('returns non-OK for non-existent path via REST /api/browse', async () => {
    const res = await fetch(`${host.baseUrl}/api/browse?path=/nonexistent-drift-test-path`);
    expect(res.ok, 'non-existent path should return non-OK').toBe(false);
  }, 15_000);

  it('browses to an absolute path via WebSocket BrowseRequest', async () => {
    // BrowseRequest is forwarded to the remote (client), so use client's absolute root
    const clientRootRes = await fetch(`${client.baseUrl}/api/browse?path=.`);
    const clientRootData: BrowseResponse = await clientRootRes.json();
    const clientAbsoluteCwd = clientRootData.cwd;

    const ws = await WsBrowserClient.connect(host.wsUrl);
    try {
      const browsePromise = ws.waitForMessage(
        (msg) => msg.type === 'BrowseResponse',
        10_000,
      );
      ws.send({ type: 'BrowseRequest', path: clientAbsoluteCwd });
      const msg = await browsePromise;
      expect(msg.type).toBe('BrowseResponse');
      if (msg.type === 'BrowseResponse') {
        expect(msg.cwd).toBe(clientAbsoluteCwd);
      }
    } finally {
      ws.close();
    }
  }, 15_000);

  it('returns Error for non-existent path via WebSocket BrowseRequest', async () => {
    const ws = await WsBrowserClient.connect(host.wsUrl);
    try {
      const errorPromise = ws.waitForMessage(
        (msg) => msg.type === 'Error',
        10_000,
      );
      ws.send({ type: 'BrowseRequest', path: '/nonexistent-drift-test-path' });
      const msg = await errorPromise;
      expect(msg.type).toBe('Error');
    } finally {
      ws.close();
    }
  }, 15_000);

  it('verifies .drift temp cleanup (after push/pull)', async () => {
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

// ── Dynamic port + connection management tests ────────────────────────────────

describe('drift dynamic port and connection management', () => {
  let serverA: DriftProcess;
  let serverB: DriftProcess;

  beforeAll(async () => {
    if (!fs.existsSync(TEST_RESOURCES)) {
      throw new Error('test-resources/ not found. See frontend/test/README.md.');
    }
  });

  afterAll(async () => {
    await Promise.all([serverA?.stop(), serverB?.stop()]);
  }, 15_000);

  it('starts on a dynamic port when --port is omitted', async () => {
    serverA = new DriftProcess({ cwd: path.join(TEST_RESOURCES, 'host') });
    await serverA.start();

    expect(serverA.port, 'port should be assigned by OS (> 0)').toBeGreaterThan(0);

    const res = await fetch(`${serverA.baseUrl}/api/info`);
    expect(res.ok, '/api/info should be reachable on dynamic port').toBe(true);
    const info = await res.json();
    expect(info).toHaveProperty('hostname');
  }, 20_000);

  it('connects to a remote via POST /api/connect', async () => {
    // Start serverB (no --target so it starts standalone)
    serverB = new DriftProcess({ cwd: path.join(TEST_RESOURCES, 'client') });
    await serverB.start();

    // Connect serverB → serverA via the REST API
    const res = await fetch(`${serverB.baseUrl}/api/connect`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ target: `127.0.0.1:${serverA.port}` }),
    });
    expect(res.ok, 'POST /api/connect should return 200').toBe(true);
    const data = await res.json();
    expect(data.success, 'connect should succeed').toBe(true);
    expect(typeof data.fingerprint, 'fingerprint should be a string').toBe('string');
    expect(data.fingerprint).toHaveLength(6);

    // /api/info should now show has_remote: true
    const info = await fetch(`${serverB.baseUrl}/api/info`).then((r) => r.json());
    expect(info.has_remote, 'has_remote should be true after connecting').toBe(true);

    // Should be able to browse the remote (serverA) via serverB's WebSocket
    const ws = await WsBrowserClient.connect(serverB.wsUrl);
    try {
      const browsePromise = ws.waitForMessage((msg) => msg.type === 'BrowseResponse', 10_000);
      ws.send({ type: 'BrowseRequest', path: '.' });
      const msg = await browsePromise;
      expect(msg.type).toBe('BrowseResponse');
    } finally {
      ws.close();
    }
  }, 30_000);

  it('disconnects from remote via POST /api/disconnect', async () => {
    // serverB should currently be connected to serverA (from previous test)
    const infoBeforeRes = await fetch(`${serverB.baseUrl}/api/info`);
    const infoBefore = await infoBeforeRes.json();
    expect(infoBefore.has_remote, 'should be connected before disconnect').toBe(true);

    // Subscribe to WS before disconnecting to catch the ConnectionStatus event
    const ws = await WsBrowserClient.connect(serverB.wsUrl);
    const statusPromise = ws.waitForMessage(
      (msg) => msg.type === 'ConnectionStatus',
      10_000,
    );

    const res = await fetch(`${serverB.baseUrl}/api/disconnect`, { method: 'POST' });
    expect(res.ok, 'POST /api/disconnect should return 200').toBe(true);
    const data = await res.json();
    expect(data.success, 'disconnect should succeed').toBe(true);

    // Should receive ConnectionStatus { has_remote: false } over WebSocket
    const statusMsg = await statusPromise;
    expect(statusMsg.type).toBe('ConnectionStatus');
    if (statusMsg.type === 'ConnectionStatus') {
      expect(statusMsg.has_remote).toBe(false);
    }
    ws.close();

    // /api/info should confirm disconnected state
    const infoAfter = await fetch(`${serverB.baseUrl}/api/info`).then((r) => r.json());
    expect(infoAfter.has_remote, 'has_remote should be false after disconnect').toBe(false);
  }, 20_000);

  it('can switch connections by calling /api/connect again', async () => {
    // Start a third server to switch to
    const serverC = new DriftProcess({ cwd: path.join(TEST_RESOURCES, 'host') });
    await serverC.start();

    try {
      // Connect serverB → serverA
      const res1 = await fetch(`${serverB.baseUrl}/api/connect`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ target: `127.0.0.1:${serverA.port}` }),
      });
      const data1 = await res1.json();
      expect(data1.success, 'first connect should succeed').toBe(true);

      // Immediately reconnect serverB → serverC (switches connection)
      const res2 = await fetch(`${serverB.baseUrl}/api/connect`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ target: `127.0.0.1:${serverC.port}` }),
      });
      const data2 = await res2.json();
      expect(data2.success, 'second connect (switch) should succeed').toBe(true);

      // serverB should now have a remote connection
      const info = await fetch(`${serverB.baseUrl}/api/info`).then((r) => r.json());
      expect(info.has_remote, 'has_remote should be true after switch').toBe(true);
    } finally {
      await serverC.stop();
    }
  }, 30_000);
});

describe('password authentication', () => {
  const hostDir = path.join(TEST_RESOURCES, 'host');
  let hostPort: number;
  let clientPort: number;
  let hostProc: DriftProcess;

  beforeAll(async () => {
    hostPort = await getAvailablePort();
    clientPort = await getAvailablePort();
  });

  afterAll(async () => {
    if (hostProc) await hostProc.stop();
  });

  it('connects with matching passwords', async () => {
    hostProc = new DriftProcess({ port: hostPort, cwd: hostDir, password: 'test-secret' });
    await hostProc.start();

    const output = runDriftCli(
      ['ls', '--target', `127.0.0.1:${hostPort}`, '--password', 'test-secret', '.'],
      { cwd: hostDir },
    );
    expect(output).toBeTruthy();

    await hostProc.stop();
  }, 30_000);

  it('rejects wrong password', () => {
    expect(async () => {
      hostProc = new DriftProcess({ port: await getAvailablePort(), cwd: hostDir, password: 'correct' });
      await hostProc.start();

      try {
        runDriftCli(
          ['ls', '--target', `127.0.0.1:${hostProc.port}`, '--password', 'wrong', '.'],
          { cwd: hostDir },
        );
      } finally {
        await hostProc.stop();
      }
    }).rejects.toThrow();
  }, 30_000);

  it('rejects client with no password when server requires one', () => {
    expect(async () => {
      hostProc = new DriftProcess({ port: await getAvailablePort(), cwd: hostDir, password: 'required' });
      await hostProc.start();

      try {
        runDriftCli(
          ['ls', '--target', `127.0.0.1:${hostProc.port}`, '.'],
          { cwd: hostDir },
        );
      } finally {
        await hostProc.stop();
      }
    }).rejects.toThrow();
  }, 30_000);
});

describe('--disable-ui flag', () => {
  const hostDir = TEST_RESOURCES;
  let hostProc: DriftProcess;

  afterAll(async () => {
    await hostProc?.stop();
  });

  it('hides REST API and frontend when --disable-ui is set', async () => {
    hostProc = new DriftProcess({ cwd: hostDir, disableUi: true });
    await hostProc.start();

    const [browseRes, infoRes, rootRes] = await Promise.all([
      fetch(`${hostProc.baseUrl}/api/browse`),
      fetch(`${hostProc.baseUrl}/api/info`),
      fetch(`${hostProc.baseUrl}/`),
    ]);

    expect(browseRes.status).toBe(404);
    expect(infoRes.status).toBe(404);
    expect(rootRes.status).toBe(404);
  }, 30_000);

  it('still accepts WebSocket connections on /ws when --disable-ui is set', async () => {
    // hostProc is still running from previous test
    const output = runDriftCli(
      ['ls', '--target', `127.0.0.1:${hostProc.port}`, '.'],
      { cwd: hostDir },
    );
    expect(output).toContain(':');
  }, 30_000);

  it('--allow-insecure-tls and --disable-ui flags are accepted by the root command and subcommands', () => {
    const helpRoot = runDriftCli(['--help']);
    const helpSend = runDriftCli(['send', '--help']);
    const helpLs   = runDriftCli(['ls', '--help']);
    const helpPull  = runDriftCli(['pull', '--help']);

    expect(helpRoot).toContain('allow-insecure-tls');
    expect(helpRoot).toContain('disable-ui');
    expect(helpRoot).toContain('daemon');
    for (const help of [helpSend, helpLs, helpPull]) {
      expect(help).toContain('allow-insecure-tls');
    }
  }, 15_000);

  it('--daemon starts server in background and writes to drift.log', async () => {
    const tmpDir = fs.mkdtempSync('/tmp/drift-daemon-test-');
    try {
      const result = runDriftCli(['--daemon'], { cwd: tmpDir });
      expect(result).toContain('PID:');
      expect(result).toContain('drift.log');

      // Give daemon time to start and write its port line
      await new Promise((r) => setTimeout(r, 1_500));

      const logContent = fs.readFileSync(path.join(tmpDir, 'drift.log'), 'utf-8');
      const portMatch = logContent.match(/localhost:(\d+)/);
      expect(portMatch).not.toBeNull();

      const port = parseInt(portMatch![1], 10);
      const res = await fetch(`http://127.0.0.1:${port}/api/info`);
      expect(res.status).toBe(200);

      // Extract PID and kill daemon
      const pidMatch = result.match(/PID: (\d+)/);
      if (pidMatch) process.kill(parseInt(pidMatch[1], 10));
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  }, 30_000);
});
