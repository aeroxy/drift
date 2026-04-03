import WebSocket from 'ws';
import type { ControlMessage } from '../../src/types/protocol.js';

type MessageHandler = (msg: ControlMessage) => void;

export class WsBrowserClient {
  private ws: WebSocket;
  private handlers: MessageHandler[] = [];
  private buffer: ControlMessage[] = [];

  private constructor(ws: WebSocket) {
    this.ws = ws;
    ws.on('message', (data: WebSocket.RawData, isBinary: boolean) => {
      if (isBinary) return;
      try {
        const msg = JSON.parse(data.toString());
        // The server sends a KeyExchange message first to probe connection type.
        // A real browser ignores it — so do we.
        if (msg.type === 'KeyExchange') return;
        const controlMsg = msg as ControlMessage;
        for (const handler of this.handlers) handler(controlMsg);
        this.buffer.push(controlMsg);
      } catch {
        // ignore malformed messages
      }
    });
  }

  // Connect like a real browser: just open the socket. No handshake dance.
  // The first message we send will be our actual request (TransferRequest etc.)
  // which tells the server we're a browser (not another drift instance).
  static connect(url: string): Promise<WsBrowserClient> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      const client = new WsBrowserClient(ws);
      ws.once('error', reject);
      ws.once('open', () => {
        ws.removeListener('error', reject);
        resolve(client);
      });
    });
  }

  send(msg: ControlMessage): void {
    this.ws.send(JSON.stringify(msg));
  }

  waitForMessage(predicate: (msg: ControlMessage) => boolean, timeoutMs = 30_000): Promise<ControlMessage> {
    return new Promise((resolve, reject) => {
      const idx = this.buffer.findIndex(predicate);
      if (idx !== -1) {
        const found = this.buffer.splice(idx, 1)[0];
        resolve(found);
        return;
      }

      const timer = setTimeout(() => {
        this.handlers.splice(this.handlers.indexOf(handler), 1);
        reject(new Error(`waitForMessage timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      const handler: MessageHandler = (msg) => {
        if (predicate(msg)) {
          clearTimeout(timer);
          this.handlers.splice(this.handlers.indexOf(handler), 1);
          resolve(msg);
        }
      };

      this.handlers.push(handler);
    });
  }

  waitForTransferComplete(transferId: string, timeoutMs = 60_000): Promise<void> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.handlers.splice(this.handlers.indexOf(handler), 1);
        reject(new Error(`Transfer ${transferId} timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      const handler: MessageHandler = (msg) => {
        if (msg.type === 'TransferComplete' && msg.id === transferId) {
          clearTimeout(timer);
          this.handlers.splice(this.handlers.indexOf(handler), 1);
          resolve();
        } else if (msg.type === 'TransferError' && msg.id === transferId) {
          clearTimeout(timer);
          this.handlers.splice(this.handlers.indexOf(handler), 1);
          reject(new Error(`Transfer error: ${msg.error}`));
        }
      };

      this.handlers.push(handler);
    });
  }

  close(): void {
    this.ws.close();
  }
}
