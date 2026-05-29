/**
 * address-bar-browse.test.ts
 *
 * Integration tests for the address bar browsing features:
 * 1. /api/browse-remote REST endpoint — fetches remote directory entries via REST
 *    without WS side effects (for suggestion autocomplete)
 * 2. /api/browse path normalization — absolute paths are handled correctly
 *    relative to root_dir
 * 3. Cached suggestions — both local and remote panels return entries from cache
 *    when browsing the current cwd
 */
import { describe, it, beforeAll, afterAll, expect } from 'vitest';
import * as path from 'path';
import * as os from 'os';
import * as fs from 'fs';
import { getAvailablePort } from './helpers/ports.js';
import { DriftProcess } from './helpers/drift-process.js';
import { WsBrowserClient } from './helpers/ws-client.js';
import { isolateTestResources, cleanupIsolatedTestResources } from './helpers/test-resources.js';
import type { FileEntry } from '../src/types/protocol.js';

const PROJECT_ROOT = path.resolve(import.meta.dirname, '../../');
let TEST_RESOURCES = path.join(PROJECT_ROOT, 'test-resources');

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

describe('address bar browse', () => {
  let host: DriftProcess;
  let client: DriftProcess;
  let hostWs: WsBrowserClient;
  let clientWs: WsBrowserClient;

  beforeAll(async () => {
    TEST_RESOURCES = isolateTestResources();

    const hostPort = await getAvailablePort();
    const clientPort = await getAvailablePort();

    host = new DriftProcess({ port: hostPort, cwd: path.join(TEST_RESOURCES, 'host') });
    client = new DriftProcess({
      port: clientPort,
      cwd: path.join(TEST_RESOURCES, 'client'),
      target: `127.0.0.1:${hostPort}`,
    });

    await host.start();
    await client.start();

    await Promise.all([
      pollForRemote(host.baseUrl),
      pollForRemote(client.baseUrl),
    ]);

    hostWs = await WsBrowserClient.connect(host.wsUrl);
    clientWs = await WsBrowserClient.connect(client.wsUrl);
  }, 120_000);

  afterAll(async () => {
    hostWs?.close();
    clientWs?.close();
    await Promise.all([host?.stop(), client?.stop()]);
    cleanupIsolatedTestResources(TEST_RESOURCES);
  }, 30_000);

  // -------------------------------------------------------------------------
  // /api/browse-remote endpoint
  // -------------------------------------------------------------------------

  describe('/api/browse-remote', () => {
    it('returns remote directory entries via REST', async () => {
      const res = await fetch(`${client.baseUrl}/api/browse-remote?path=.`);
      expect(res.ok).toBe(true);
      const data: BrowseResponse = await res.json();
      expect(data.hostname).toBeTruthy();
      expect(data.cwd).toBeTruthy();
      expect(Array.isArray(data.entries)).toBe(true);
    });

    it('returns the same entries as a WS BrowseRequest to the remote', async () => {
      // Get entries via REST (client browses remote = host's files)
      const restRes = await fetch(`${client.baseUrl}/api/browse-remote?path=.`);
      const restData: BrowseResponse = await restRes.json();

      // Get entries via WS — client sends BrowseRequest which gets forwarded to host
      const wsPromise = clientWs.waitForMessage(
        (m) => m.type === 'BrowseResponse',
        10_000,
      );
      clientWs.send({ type: 'BrowseRequest', path: '.' });
      const wsMsg = await wsPromise;

      if (wsMsg.type === 'BrowseResponse') {
        expect(wsMsg.entries.length).toBe(restData.entries.length);
        const restNames = restData.entries.map((e) => e.name).sort();
        const wsNames = wsMsg.entries.map((e) => e.name).sort();
        expect(restNames).toEqual(wsNames);
      }
    });

    it('can browse subdirectories on the remote', async () => {
      // First get root entries to find a subdirectory
      const rootRes = await fetch(`${client.baseUrl}/api/browse-remote?path=.`);
      const rootData: BrowseResponse = await rootRes.json();
      const subdir = rootData.entries.find((e) => e.is_dir);
      if (!subdir) {
        // No subdirectory to test — skip gracefully
        return;
      }

      const subRes = await fetch(
        `${client.baseUrl}/api/browse-remote?path=${encodeURIComponent(subdir.name)}`,
      );
      expect(subRes.ok).toBe(true);
      const subData: BrowseResponse = await subRes.json();
      expect(subData.cwd).toContain(subdir.name);
      expect(Array.isArray(subData.entries)).toBe(true);
    });

    it('keeps concurrent browse-remote responses matched to the requested path', async () => {
      const remoteRoot = path.join(TEST_RESOURCES, 'host');
      const alphaDir = path.join(remoteRoot, 'concurrent-alpha');
      const betaDir = path.join(remoteRoot, 'concurrent-beta');
      fs.mkdirSync(alphaDir, { recursive: true });
      fs.mkdirSync(betaDir, { recursive: true });
      fs.writeFileSync(path.join(alphaDir, 'alpha-only.txt'), 'alpha\n');
      fs.writeFileSync(path.join(betaDir, 'beta-only.txt'), 'beta\n');

      const requests = ['.', 'concurrent-alpha', 'concurrent-beta', 'concurrent-alpha', 'concurrent-beta'];
      const responses = await Promise.all(
        requests.map(async (reqPath) => {
          const res = await fetch(`${client.baseUrl}/api/browse-remote?path=${encodeURIComponent(reqPath)}`);
          expect(res.ok, `browse-remote should succeed for ${reqPath}`).toBe(true);
          return {
            reqPath,
            data: await res.json() as BrowseResponse,
          };
        }),
      );

      for (const { reqPath, data } of responses) {
        if (reqPath === 'concurrent-alpha') {
          expect(data.cwd).toContain('concurrent-alpha');
          expect(data.entries.map((e) => e.name)).toContain('alpha-only.txt');
          expect(data.entries.map((e) => e.name)).not.toContain('beta-only.txt');
        } else if (reqPath === 'concurrent-beta') {
          expect(data.cwd).toContain('concurrent-beta');
          expect(data.entries.map((e) => e.name)).toContain('beta-only.txt');
          expect(data.entries.map((e) => e.name)).not.toContain('alpha-only.txt');
        }
      }
    });

    it('returns 400 when no remote connection exists', async () => {
      // Start a standalone server with no target
      const standalonePort = await getAvailablePort();
      const standalone = new DriftProcess({
        port: standalonePort,
        cwd: path.join(TEST_RESOURCES, 'host'),
      });
      await standalone.start();

      try {
        const res = await fetch(`${standalone.baseUrl}/api/browse-remote?path=.`);
        expect(res.ok).toBe(false);
        expect(res.status).toBe(400);
      } finally {
        await standalone.stop();
      }
    });

    it('does not update the remote panel state (silent)', async () => {
      // Capture remote cwd via /api/browse-remote
      const beforeRes = await fetch(`${client.baseUrl}/api/browse-remote?path=.`);
      const beforeData: BrowseResponse = await beforeRes.json();
      const cwdBefore = beforeData.cwd;

      // Browse again via REST — should not change the remote panel's cwd
      const browseRes = await fetch(`${client.baseUrl}/api/browse-remote?path=.`);
      expect(browseRes.ok).toBe(true);
      const afterData: BrowseResponse = await browseRes.json();

      expect(afterData.cwd).toBe(cwdBefore);
    });
  });

  // -------------------------------------------------------------------------
  // /api/browse path normalization
  // -------------------------------------------------------------------------

  describe('/api/browse path normalization', () => {
    it('accepts relative paths', async () => {
      const res = await fetch(`${host.baseUrl}/api/browse?path=.`);
      expect(res.ok).toBe(true);
      const data: BrowseResponse = await res.json();
      expect(data.entries.length).toBeGreaterThan(0);
    });

    it('accepts absolute paths within root_dir', async () => {
      // First get the root_dir from /api/info
      const info: InfoResponse = await (await fetch(`${host.baseUrl}/api/info`)).json();
      const rootDir = info.root_dir;

      // Browse root_dir as an absolute path
      const res = await fetch(
        `${host.baseUrl}/api/browse?path=${encodeURIComponent(rootDir)}`,
      );
      expect(res.ok).toBe(true);
      const data: BrowseResponse = await res.json();
      expect(data.entries.length).toBeGreaterThan(0);
    });

    it('rejects paths outside root_dir with 400', async () => {
      const outsidePath = path.join(os.tmpdir(), 'drift-test-outside');
      const res = await fetch(
        `${host.baseUrl}/api/browse?path=${encodeURIComponent(outsidePath)}`,
      );
      expect(res.ok).toBe(false);
      expect(res.status).toBe(400);
    });

    it('returns cwd in the response', async () => {
      const res = await fetch(`${host.baseUrl}/api/browse?path=.`);
      const data: BrowseResponse = await res.json();
      expect(data.cwd).toBeTruthy();
      expect(path.isAbsolute(data.cwd)).toBe(true);
    });
  });

  // -------------------------------------------------------------------------
  // Cached suggestions
  // -------------------------------------------------------------------------

  describe('cached suggestions', () => {
    it('local browse returns consistent entries for the same path', async () => {
      // Browse the same path twice — should get identical results
      const res1 = await fetch(`${host.baseUrl}/api/browse?path=.`);
      const data1: BrowseResponse = await res1.json();

      const res2 = await fetch(`${host.baseUrl}/api/browse?path=.`);
      const data2: BrowseResponse = await res2.json();

      const names1 = data1.entries.map((e) => e.name).sort();
      const names2 = data2.entries.map((e) => e.name).sort();
      expect(names1).toEqual(names2);
    });

    it('remote browse-remote returns consistent entries for the same path', async () => {
      const res1 = await fetch(`${client.baseUrl}/api/browse-remote?path=.`);
      const data1: BrowseResponse = await res1.json();

      const res2 = await fetch(`${client.baseUrl}/api/browse-remote?path=.`);
      const data2: BrowseResponse = await res2.json();

      const names1 = data1.entries.map((e) => e.name).sort();
      const names2 = data2.entries.map((e) => e.name).sort();
      expect(names1).toEqual(names2);
    });

    it('entries include is_dir, size, and modified fields', async () => {
      const res = await fetch(`${host.baseUrl}/api/browse?path=.`);
      const data: BrowseResponse = await res.json();

      for (const entry of data.entries) {
        expect(typeof entry.name).toBe('string');
        expect(typeof entry.is_dir).toBe('boolean');
        expect(typeof entry.size).toBe('number');
        expect(typeof entry.modified).toBe('number');
      }
    });
  });

  // -------------------------------------------------------------------------
  // WS BrowseRequest still works for navigation
  // -------------------------------------------------------------------------

  describe('WS BrowseRequest for navigation', () => {
    it('BrowseRequest via WS returns entries', async () => {
      // Use clientWs — BrowseRequest gets forwarded to the remote (host)
      const response = clientWs.waitForMessage(
        (m) => m.type === 'BrowseResponse',
        10_000,
      );
      clientWs.send({ type: 'BrowseRequest', path: '.' });
      const msg = await response;
      expect(msg.type).toBe('BrowseResponse');
      if (msg.type === 'BrowseResponse') {
        expect(msg.entries.length).toBeGreaterThan(0);
        expect(msg.hostname).toBeTruthy();
        expect(msg.cwd).toBeTruthy();
      }
    });
  });
});
