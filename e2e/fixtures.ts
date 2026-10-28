import { spawn, ChildProcess } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export interface Bridge {
  url: string;
  dataDir: string;
  stop(): Promise<void>;
}

let nextPort = 5300;

/** Start the real core behind devbridge on a throw-away data dir. Pass `dataDir` to reuse one (restart tests). */
export async function startBridge(dataDir?: string): Promise<Bridge> {
  const dir = dataDir ?? mkdtempSync(join(tmpdir(), 'd256shell-e2e-'));
  const port = nextPort++;
  const bin = process.env.DEVBRIDGE_BIN ?? '/build/target/debug/d256shell-devbridge';
  const child: ChildProcess = spawn(bin, [], {
    env: { ...process.env, D256SHELL_DATA_DIR: dir, DEVBRIDGE_PORT: String(port), DEVBRIDGE_UI: join(__dirname, '..', 'ui', 'dist') },
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  await new Promise<void>((resolve, reject) => {
    child.stdout!.on('data', (d) => String(d).includes('listening') && resolve());
    child.on('exit', (c) => reject(new Error(`devbridge exited early (${c})`)));
    setTimeout(() => reject(new Error('devbridge start timeout')), 15000);
  });
  return {
    url: `http://127.0.0.1:${port}`,
    dataDir: dir,
    async stop() {
      child.kill();
      await new Promise((r) => setTimeout(r, 100));
      if (!dataDir) rmSync(dir, { recursive: true, force: true });
    },
  };
}
