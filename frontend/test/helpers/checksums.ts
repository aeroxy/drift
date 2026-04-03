import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';

function computeFileChecksum(filePath: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('md5');
    const stream = fs.createReadStream(filePath);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolve(hash.digest('hex')));
    stream.on('error', reject);
  });
}

function walkDir(dir: string, base: string, results: string[]): void {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === '.drift' || entry.name === '.DS_Store') continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walkDir(full, base, results);
    } else if (entry.isFile()) {
      results.push(full);
    }
  }
}

export async function computeAllChecksums(rootDir: string): Promise<Map<string, string>> {
  const files: string[] = [];
  if (fs.existsSync(rootDir)) {
    walkDir(rootDir, rootDir, files);
  }

  const map = new Map<string, string>();
  for (const file of files) {
    const rel = path.relative(rootDir, file);
    map.set(rel, await computeFileChecksum(file));
  }
  return map;
}
