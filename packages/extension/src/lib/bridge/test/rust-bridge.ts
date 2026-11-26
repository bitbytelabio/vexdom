/**
 * The Rust Bridge (`target/debug/vexdom` at the repository root) for the extension's integration
 * tests, in a scratch home, with ext-core's node build standing in for the wasm the browser loads.
 * Both are build output: `cargo build -p vexdom` at the root and `bun run ext-core:build` first.
 */
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const extension = resolve(__dirname, '../../../..');
const repo = resolve(extension, '../..');
export const BIN = join(repo, 'target/debug/vexdom');
export const EXT_CORE_NODE = join(extension, '.ext-core/pkg/node.js');

for (const [path, how] of [
  [BIN, 'cargo build -p vexdom (at the repository root)'],
  [EXT_CORE_NODE, 'bun run ext-core:build'],
] as const) {
  if (!existsSync(path)) throw new Error(`${path} is missing: run ${how}`);
}

export interface RustBridge {
  home: string;
  port: number;
  cli(...args: string[]): string;
  /** `vexdom native-host <caller>`: one framed message in, one framed reply out. */
  nativeHost(message: unknown, caller: string): Promise<unknown>;
  /** `auth.json`: the sessions the Bridge holds. */
  sessions(): { origin: string; installId?: string; pairedBy?: string }[];
  forgetSessions(): void;
  resetConfig(): void;
  stop(): void;
}

export function startRustBridge(): RustBridge {
  const home = realpathSync(mkdtempSync(join(tmpdir(), 'vexdom-link-test-')));
  const state = join(home, '.vexdom');
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: home, USERPROFILE: home, VEXDOM_HOME: state, VEXDOM_PORTS: '0' };
  delete env.VEXDOM_AGENT_RUN;
  const cli = (...args: string[]) => execFileSync(BIN, args, { env, encoding: 'utf8' });
  const started = /127\.0\.0\.1:(\d+)/.exec(cli('start'));
  if (!started) throw new Error('the Rust Bridge did not start');
  const authPath = join(state, 'auth.json');
  return {
    home,
    port: Number(started[1]),
    cli,
    nativeHost: (message, caller) =>
      new Promise((resolve, reject) => {
        const child = spawn(BIN, ['native-host', caller], { env, stdio: ['pipe', 'pipe', 'ignore'] });
        const chunks: Buffer[] = [];
        child.stdout.on('data', (chunk: Buffer) => chunks.push(chunk));
        child.on('error', reject);
        child.on('close', () => {
          const out = Buffer.concat(chunks);
          if (out.length < 4) return reject(new Error('the native host wrote no reply'));
          resolve(JSON.parse(out.subarray(4, 4 + out.readUInt32LE(0)).toString('utf8')));
        });
        const body = Buffer.from(JSON.stringify(message), 'utf8');
        const head = Buffer.alloc(4);
        head.writeUInt32LE(body.length);
        child.stdin.end(Buffer.concat([head, body]));
      }),
    sessions: () => {
      try {
        return (JSON.parse(readFileSync(authPath, 'utf8')) as { sessions?: [] }).sessions ?? [];
      } catch {
        return [];
      }
    },
    forgetSessions: () => rmSync(authPath, { force: true }),
    resetConfig: () => rmSync(join(state, 'config.json'), { force: true }),
    stop: () => {
      cli('stop');
      rmSync(home, { recursive: true, force: true });
    },
  };
}
