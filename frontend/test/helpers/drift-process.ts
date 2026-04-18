import { spawn, execFileSync, ChildProcess } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';

const PROJECT_ROOT = path.resolve(import.meta.dirname, '../../../');
const BINARY = path.join(PROJECT_ROOT, 'target/debug/drift');

interface DriftProcessOptions {
  port?: number;
  cwd: string;
  target?: string;
  password?: string;
}

export class DriftProcess {
  private _port: number;
  readonly cwd: string;
  readonly target?: string;
  readonly password?: string;
  private proc: ChildProcess | null = null;

  constructor(opts: DriftProcessOptions) {
    this._port = opts.port ?? 0;
    this.cwd = opts.cwd;
    this.target = opts.target;
    this.password = opts.password;
  }

  /** Actual port (available after start() resolves). */
  get port(): number {
    return this._port;
  }

  get baseUrl() {
    return `http://127.0.0.1:${this._port}`;
  }

  get wsUrl() {
    return `ws://127.0.0.1:${this._port}/ws`;
  }

  start(): Promise<void> {
    return new Promise((resolve, reject) => {
      if (!fs.existsSync(BINARY)) {
        reject(new Error(`drift binary not found at ${BINARY}. Run \`cargo build\` first.`));
        return;
      }

      const args = ['serve'];
      if (this._port !== 0) {
        args.push('--port', String(this._port));
      }
      if (this.target) {
        args.push('--target', this.target);
      }
      if (this.password) {
        args.push('--password', this.password);
      }

      this.proc = spawn(BINARY, args, {
        cwd: this.cwd,
        env: { ...process.env, RUST_LOG: 'drift=info' },
        stdio: ['ignore', 'pipe', 'pipe'],
      });

      const timeout = setTimeout(() => {
        reject(new Error(`drift process (port ${this._port}) failed to start within 15s`));
      }, 15_000);

      const onData = (data: Buffer) => {
        const text = data.toString();
        // Parse the actual port from log: "drift server listening on http://localhost:PORT"
        const match = text.match(/listening on http:\/\/localhost:(\d+)/);
        if (match) {
          this._port = parseInt(match[1], 10);
          clearTimeout(timeout);
          resolve();
        }
      };

      this.proc.stdout?.on('data', onData);
      this.proc.stderr?.on('data', onData);

      this.proc.on('error', (err) => {
        clearTimeout(timeout);
        reject(err);
      });

      this.proc.on('exit', (code) => {
        if (code !== null && code !== 0) {
          clearTimeout(timeout);
          reject(new Error(`drift process (port ${this._port}) exited with code ${code}`));
        }
      });
    });
  }

  stop(): Promise<void> {
    return new Promise((resolve) => {
      if (!this.proc || this.proc.exitCode !== null) {
        resolve();
        return;
      }

      const killTimer = setTimeout(() => {
        this.proc?.kill('SIGKILL');
        resolve();
      }, 5_000);

      this.proc.once('exit', () => {
        clearTimeout(killTimer);
        resolve();
      });

      this.proc.kill('SIGTERM');
    });
  }
}

/** Run a one-shot drift CLI command and return stdout. Throws on non-zero exit. */
export function runDriftCli(args: string[], opts?: { cwd?: string; timeoutMs?: number }): string {
  if (!fs.existsSync(BINARY)) {
    throw new Error(`drift binary not found at ${BINARY}. Run \`cargo build\` first.`);
  }
  return execFileSync(BINARY, args, {
    cwd: opts?.cwd ?? PROJECT_ROOT,
    timeout: opts?.timeoutMs ?? 30_000,
    env: { ...process.env, RUST_LOG: 'drift=info' },
    encoding: 'utf-8',
  });
}
