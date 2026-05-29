import * as path from 'path';
import * as fs from 'fs';

const PROJECT_ROOT = path.resolve(import.meta.dirname, '../../../');
const SHARED_TEST_RESOURCES = path.join(PROJECT_ROOT, 'test-resources');

/**
 * Copy the shared test-resources/ directory into a fresh temp directory so each
 * test suite works on its own isolated fixtures.  Returns the path to the copy.
 * Call `cleanupIsolatedTestResources(tmpDir)` in afterAll to remove it.
 */
export function isolateTestResources(): string {
  if (!fs.existsSync(SHARED_TEST_RESOURCES)) {
    throw new Error(
      'test-resources/ not found. Create test-resources/host/ (with a subdirectory) ' +
      'and test-resources/client/ (with files) before running tests.',
    );
  }

  const tmpDir = fs.mkdtempSync(path.join(PROJECT_ROOT, '.test-resources-'));
  fs.cpSync(SHARED_TEST_RESOURCES, path.join(tmpDir, 'test-resources'), { recursive: true });
  return path.join(tmpDir, 'test-resources');
}

/**
 * Remove the isolated test-resources copy created by `isolateTestResources()`.
 */
export function cleanupIsolatedTestResources(isolatedDir: string): void {
  const resolved = path.resolve(isolatedDir);
  const parent = path.dirname(resolved);
  const isExpectedLayout =
    path.basename(resolved) === 'test-resources' &&
    path.basename(parent).startsWith('.test-resources-') &&
    path.dirname(parent) === PROJECT_ROOT;

  if (!isExpectedLayout) return;

  try {
    fs.rmSync(parent, { recursive: true, force: true });
  } catch {
    // best-effort
  }
}
