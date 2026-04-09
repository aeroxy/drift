/**
 * web-panel-navigation.test.ts
 *
 * Verifies that push/pull transfers via the web panel place files in the
 * correct directory for every combination of local-panel dir × remote-panel
 * dir, from both the host and client browser perspectives.
 *
 * Uses WsBrowserClient to send the exact same WebSocket messages the React
 * frontend sends after navigating panels and clicking a copy button.
 */
import { describe, it, beforeAll, afterAll, expect } from 'vitest';
import * as path from 'path';
import * as fs from 'fs';
import { execSync } from 'child_process';
import { getAvailablePort } from './helpers/ports.js';
import { DriftProcess } from './helpers/drift-process.js';
import { WsBrowserClient } from './helpers/ws-client.js';
import type { FileEntry } from '../src/types/protocol.js';

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------
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
      // not ready yet
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`Remote not connected within ${timeoutMs}ms`);
}

/** Browse a directory on a server via the REST API to get file entries. */
async function browseDir(baseUrl: string, dirSegments: string[]): Promise<FileEntry[]> {
  const p = dirSegments.length ? dirSegments.join('/') : '.';
  const res = await fetch(`${baseUrl}/api/browse?path=${encodeURIComponent(p)}`);
  const data: BrowseResponse = await res.json();
  return data.entries;
}

/** Poll until a file appears on disk. */
async function waitForFile(filePath: string, timeoutMs = 30_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (fs.existsSync(filePath)) return;
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error(`File "${filePath}" did not appear within ${timeoutMs}ms`);
}

// ---------------------------------------------------------------------------
// Transfer helpers — mimic exactly what the React UI sends
// ---------------------------------------------------------------------------

/**
 * Pull a file from the remote panel's current directory to the local panel's
 * current directory.
 *
 * @param ws          WsBrowserClient connected to the *local* server
 * @param remoteNav   Path segments of the remote panel's current directory
 * @param localNav    Path segments of the local panel's current directory
 * @param entry       FileEntry for the file to pull (obtained from remote browse)
 */
async function pullFile(
  ws: WsBrowserClient,
  remoteNav: string[],
  localNav: string[],
  entry: FileEntry,
): Promise<void> {
  const id = crypto.randomUUID();
  const remotePath = [...remoteNav, entry.name].join('/'); // full path on remote
  const destinationPath = localNav.length ? localNav.join('/') : '.'; // where to write locally

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

/**
 * Push a file from the local panel's current directory to the remote panel's
 * current directory.
 *
 * @param ws          WsBrowserClient connected to the *local* server
 * @param localNav    Path segments of the local panel's current directory
 * @param remoteNav   Path segments of the remote panel's current directory
 * @param entry       FileEntry for the file to push (obtained from local browse)
 */
async function pushFile(
  ws: WsBrowserClient,
  localNav: string[],
  remoteNav: string[],
  entry: FileEntry,
): Promise<void> {
  const id = crypto.randomUUID();
  const localPath = [...localNav, entry.name].join('/'); // full path on local
  const destinationPath = remoteNav.length ? remoteNav.join('/') : '.'; // where to write remotely

  const done = ws.waitForTransferComplete(id, 30_000);
  ws.send({
    type: 'TransferRequest',
    id,
    entries: [{
      relative_path: localPath,
      size: entry.size,
      is_dir: entry.is_dir,
      permissions: entry.permissions,
    }],
    direction: 'Push',
    destination_path: destinationPath,
  });
  await done;
}

// ---------------------------------------------------------------------------
// Test matrix
// ---------------------------------------------------------------------------

const ROOT: string[] = [];
const SUB1_CLIENT = ['client-sub1'];
const SUB2_CLIENT = ['client-sub1', 'client-sub2'];
const SUB1_HOST = ['host-sub1'];
const SUB2_HOST = ['host-sub1', 'host-sub2'];

interface PullCase {
  label: string;
  localNav: string[];
  remoteNav: string[];
  file: string;
  content: string;
  dest: string; // relative to TEST_RESOURCES
}

// From client browser: pull FROM host INTO client
const clientPullMatrix: PullCase[] = [
  { label: 'local=root   remote=root ', localNav: ROOT,        remoteNav: ROOT,      file: 'host-root-file.txt', content: 'host-root-file', dest: 'client/host-root-file.txt' },
  { label: 'local=root   remote=sub1 ', localNav: ROOT,        remoteNav: SUB1_HOST, file: 'host-sub1-file.txt', content: 'host-sub1-file', dest: 'client/host-sub1-file.txt' },
  { label: 'local=root   remote=sub2 ', localNav: ROOT,        remoteNav: SUB2_HOST, file: 'host-file.txt',      content: 'host-file',      dest: 'client/host-file.txt' },
  { label: 'local=sub1   remote=root ', localNav: SUB1_CLIENT, remoteNav: ROOT,      file: 'host-root-file.txt', content: 'host-root-file', dest: 'client/client-sub1/host-root-file.txt' },
  { label: 'local=sub1   remote=sub1 ', localNav: SUB1_CLIENT, remoteNav: SUB1_HOST, file: 'host-sub1-file.txt', content: 'host-sub1-file', dest: 'client/client-sub1/host-sub1-file.txt' },
  { label: 'local=sub1   remote=sub2 ', localNav: SUB1_CLIENT, remoteNav: SUB2_HOST, file: 'host-file.txt',      content: 'host-file',      dest: 'client/client-sub1/host-file.txt' },
  { label: 'local=sub2   remote=root ', localNav: SUB2_CLIENT, remoteNav: ROOT,      file: 'host-root-file.txt', content: 'host-root-file', dest: 'client/client-sub1/client-sub2/host-root-file.txt' },
  { label: 'local=sub2   remote=sub1 ', localNav: SUB2_CLIENT, remoteNav: SUB1_HOST, file: 'host-sub1-file.txt', content: 'host-sub1-file', dest: 'client/client-sub1/client-sub2/host-sub1-file.txt' },
  { label: 'local=sub2   remote=sub2 ', localNav: SUB2_CLIENT, remoteNav: SUB2_HOST, file: 'host-file.txt',      content: 'host-file',      dest: 'client/client-sub1/client-sub2/host-file.txt' },
];

// From client browser: push FROM client TO host
const clientPushMatrix: PullCase[] = [
  { label: 'local=root   remote=root ', localNav: ROOT,        remoteNav: ROOT,      file: 'client-root-file.txt', content: 'client-root-file', dest: 'host/client-root-file.txt' },
  { label: 'local=root   remote=sub1 ', localNav: ROOT,        remoteNav: SUB1_HOST, file: 'client-root-file.txt', content: 'client-root-file', dest: 'host/host-sub1/client-root-file.txt' },
  { label: 'local=root   remote=sub2 ', localNav: ROOT,        remoteNav: SUB2_HOST, file: 'client-root-file.txt', content: 'client-root-file', dest: 'host/host-sub1/host-sub2/client-root-file.txt' },
  { label: 'local=sub1   remote=root ', localNav: SUB1_CLIENT, remoteNav: ROOT,      file: 'client-sub1-file.txt', content: 'client-sub1-file', dest: 'host/client-sub1-file.txt' },
  { label: 'local=sub1   remote=sub1 ', localNav: SUB1_CLIENT, remoteNav: SUB1_HOST, file: 'client-sub1-file.txt', content: 'client-sub1-file', dest: 'host/host-sub1/client-sub1-file.txt' },
  { label: 'local=sub1   remote=sub2 ', localNav: SUB1_CLIENT, remoteNav: SUB2_HOST, file: 'client-sub1-file.txt', content: 'client-sub1-file', dest: 'host/host-sub1/host-sub2/client-sub1-file.txt' },
  { label: 'local=sub2   remote=root ', localNav: SUB2_CLIENT, remoteNav: ROOT,      file: 'client-file.txt',      content: 'client-file',      dest: 'host/client-file.txt' },
  { label: 'local=sub2   remote=sub1 ', localNav: SUB2_CLIENT, remoteNav: SUB1_HOST, file: 'client-file.txt',      content: 'client-file',      dest: 'host/host-sub1/client-file.txt' },
  { label: 'local=sub2   remote=sub2 ', localNav: SUB2_CLIENT, remoteNav: SUB2_HOST, file: 'client-file.txt',      content: 'client-file',      dest: 'host/host-sub1/host-sub2/client-file.txt' },
];

// From host browser: pull FROM client INTO host
const hostPullMatrix: PullCase[] = [
  { label: 'local=root   remote=root ', localNav: ROOT,      remoteNav: ROOT,        file: 'client-root-file.txt', content: 'client-root-file', dest: 'host/client-root-file.txt' },
  { label: 'local=root   remote=sub1 ', localNav: ROOT,      remoteNav: SUB1_CLIENT, file: 'client-sub1-file.txt', content: 'client-sub1-file', dest: 'host/client-sub1-file.txt' },
  { label: 'local=root   remote=sub2 ', localNav: ROOT,      remoteNav: SUB2_CLIENT, file: 'client-file.txt',      content: 'client-file',      dest: 'host/client-file.txt' },
  { label: 'local=sub1   remote=root ', localNav: SUB1_HOST, remoteNav: ROOT,        file: 'client-root-file.txt', content: 'client-root-file', dest: 'host/host-sub1/client-root-file.txt' },
  { label: 'local=sub1   remote=sub1 ', localNav: SUB1_HOST, remoteNav: SUB1_CLIENT, file: 'client-sub1-file.txt', content: 'client-sub1-file', dest: 'host/host-sub1/client-sub1-file.txt' },
  { label: 'local=sub1   remote=sub2 ', localNav: SUB1_HOST, remoteNav: SUB2_CLIENT, file: 'client-file.txt',      content: 'client-file',      dest: 'host/host-sub1/client-file.txt' },
  { label: 'local=sub2   remote=root ', localNav: SUB2_HOST, remoteNav: ROOT,        file: 'client-root-file.txt', content: 'client-root-file', dest: 'host/host-sub1/host-sub2/client-root-file.txt' },
  { label: 'local=sub2   remote=sub1 ', localNav: SUB2_HOST, remoteNav: SUB1_CLIENT, file: 'client-sub1-file.txt', content: 'client-sub1-file', dest: 'host/host-sub1/host-sub2/client-sub1-file.txt' },
  { label: 'local=sub2   remote=sub2 ', localNav: SUB2_HOST, remoteNav: SUB2_CLIENT, file: 'client-file.txt',      content: 'client-file',      dest: 'host/host-sub1/host-sub2/client-file.txt' },
];

// From host browser: push FROM host TO client
const hostPushMatrix: PullCase[] = [
  { label: 'local=root   remote=root ', localNav: ROOT,      remoteNav: ROOT,        file: 'host-root-file.txt', content: 'host-root-file', dest: 'client/host-root-file.txt' },
  { label: 'local=root   remote=sub1 ', localNav: ROOT,      remoteNav: SUB1_CLIENT, file: 'host-root-file.txt', content: 'host-root-file', dest: 'client/client-sub1/host-root-file.txt' },
  { label: 'local=root   remote=sub2 ', localNav: ROOT,      remoteNav: SUB2_CLIENT, file: 'host-root-file.txt', content: 'host-root-file', dest: 'client/client-sub1/client-sub2/host-root-file.txt' },
  { label: 'local=sub1   remote=root ', localNav: SUB1_HOST, remoteNav: ROOT,        file: 'host-sub1-file.txt', content: 'host-sub1-file', dest: 'client/host-sub1-file.txt' },
  { label: 'local=sub1   remote=sub1 ', localNav: SUB1_HOST, remoteNav: SUB1_CLIENT, file: 'host-sub1-file.txt', content: 'host-sub1-file', dest: 'client/client-sub1/host-sub1-file.txt' },
  { label: 'local=sub1   remote=sub2 ', localNav: SUB1_HOST, remoteNav: SUB2_CLIENT, file: 'host-sub1-file.txt', content: 'host-sub1-file', dest: 'client/client-sub1/client-sub2/host-sub1-file.txt' },
  { label: 'local=sub2   remote=root ', localNav: SUB2_HOST, remoteNav: ROOT,        file: 'host-file.txt',      content: 'host-file',      dest: 'client/host-file.txt' },
  { label: 'local=sub2   remote=sub1 ', localNav: SUB2_HOST, remoteNav: SUB1_CLIENT, file: 'host-file.txt',      content: 'host-file',      dest: 'client/client-sub1/host-file.txt' },
  { label: 'local=sub2   remote=sub2 ', localNav: SUB2_HOST, remoteNav: SUB2_CLIENT, file: 'host-file.txt',      content: 'host-file',      dest: 'client/client-sub1/client-sub2/host-file.txt' },
];

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

let host: DriftProcess;
let client: DriftProcess;

function registerExitHandler() {
  const cleanup = () => {
    host?.stop().catch(() => {});
    client?.stop().catch(() => {});
  };
  process.once('exit', cleanup);
  process.once('SIGINT', cleanup);
  process.once('SIGTERM', cleanup);
}

describe('web panel subdirectory transfers', () => {
  beforeAll(async () => {
    if (!fs.existsSync(TEST_RESOURCES)) {
      throw new Error(
        'test-resources/ not found. See test-resources/README.md for setup instructions.',
      );
    }

    // Backup
    if (fs.existsSync(TEST_RESOURCES_BAK)) {
      fs.rmSync(TEST_RESOURCES_BAK, { recursive: true, force: true });
    }
    execSync(`cp -a ${JSON.stringify(TEST_RESOURCES)} ${JSON.stringify(TEST_RESOURCES_BAK)}`);

    const hostPort = await getAvailablePort();
    const clientPort = await getAvailablePort();

    host = new DriftProcess({ port: hostPort, cwd: path.join(TEST_RESOURCES, 'host') });
    client = new DriftProcess({
      port: clientPort,
      cwd: path.join(TEST_RESOURCES, 'client'),
      target: `127.0.0.1:${hostPort}`,
    });

    registerExitHandler();
    await host.start();
    await client.start();
    await Promise.all([pollForRemote(host.baseUrl), pollForRemote(client.baseUrl)]);
  }, 60_000);

  afterAll(async () => {
    await Promise.all([host?.stop(), client?.stop()]);
    try {
      fs.rmSync(TEST_RESOURCES, { recursive: true, force: true });
      if (fs.existsSync(TEST_RESOURCES_BAK)) {
        fs.renameSync(TEST_RESOURCES_BAK, TEST_RESOURCES);
      }
    } catch (err) {
      console.error('Failed to restore test-resources:', err);
    }
  }, 30_000);

  // -------------------------------------------------------------------------
  // From client browser (local = client root, remote = host)
  // -------------------------------------------------------------------------
  describe('from client browser', () => {
    describe('pull from host', () => {
      it.each(clientPullMatrix)(
        'pull $label → $file',
        async ({ localNav, remoteNav, file, content, dest }) => {
          const destPath = path.join(TEST_RESOURCES, dest);

          // Get entry metadata from host's browse API (remote side for pull)
          const remoteEntries = await browseDir(host.baseUrl, remoteNav);
          const entry = remoteEntries.find((e) => e.name === file);
          expect(entry, `host does not have "${file}" at ${remoteNav.join('/') || '.'}`).toBeDefined();

          const ws = await WsBrowserClient.connect(client.wsUrl);
          try {
            await pullFile(ws, remoteNav, localNav, entry!);
          } finally {
            ws.close();
          }

          await waitForFile(destPath);
          expect(fs.readFileSync(destPath, 'utf-8').trim()).toBe(content);
          fs.unlinkSync(destPath);
        },
        60_000,
      );
    });

    describe('push to host', () => {
      it.each(clientPushMatrix)(
        'push $label → $file',
        async ({ localNav, remoteNav, file, content, dest }) => {
          const destPath = path.join(TEST_RESOURCES, dest);

          // Get entry metadata from client's browse API (local side for push)
          const localEntries = await browseDir(client.baseUrl, localNav);
          const entry = localEntries.find((e) => e.name === file);
          expect(entry, `client does not have "${file}" at ${localNav.join('/') || '.'}`).toBeDefined();

          const ws = await WsBrowserClient.connect(client.wsUrl);
          try {
            await pushFile(ws, localNav, remoteNav, entry!);
          } finally {
            ws.close();
          }

          await waitForFile(destPath);
          expect(fs.readFileSync(destPath, 'utf-8').trim()).toBe(content);
          fs.unlinkSync(destPath);
        },
        60_000,
      );
    });
  });

  // -------------------------------------------------------------------------
  // From host browser (local = host root, remote = client)
  // -------------------------------------------------------------------------
  describe('from host browser', () => {
    describe('pull from client', () => {
      it.each(hostPullMatrix)(
        'pull $label → $file',
        async ({ localNav, remoteNav, file, content, dest }) => {
          const destPath = path.join(TEST_RESOURCES, dest);

          const remoteEntries = await browseDir(client.baseUrl, remoteNav);
          const entry = remoteEntries.find((e) => e.name === file);
          expect(entry, `client does not have "${file}" at ${remoteNav.join('/') || '.'}`).toBeDefined();

          const ws = await WsBrowserClient.connect(host.wsUrl);
          try {
            await pullFile(ws, remoteNav, localNav, entry!);
          } finally {
            ws.close();
          }

          await waitForFile(destPath);
          expect(fs.readFileSync(destPath, 'utf-8').trim()).toBe(content);
          fs.unlinkSync(destPath);
        },
        60_000,
      );
    });

    describe('push to client', () => {
      it.each(hostPushMatrix)(
        'push $label → $file',
        async ({ localNav, remoteNav, file, content, dest }) => {
          const destPath = path.join(TEST_RESOURCES, dest);

          const localEntries = await browseDir(host.baseUrl, localNav);
          const entry = localEntries.find((e) => e.name === file);
          expect(entry, `host does not have "${file}" at ${localNav.join('/') || '.'}`).toBeDefined();

          const ws = await WsBrowserClient.connect(host.wsUrl);
          try {
            await pushFile(ws, localNav, remoteNav, entry!);
          } finally {
            ws.close();
          }

          await waitForFile(destPath);
          expect(fs.readFileSync(destPath, 'utf-8').trim()).toBe(content);
          fs.unlinkSync(destPath);
        },
        60_000,
      );
    });
  });
});
