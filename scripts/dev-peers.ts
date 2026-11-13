import { spawn, spawnSync } from 'node:child_process';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';

const args = process.argv.slice(2);
const skipBuild = args.includes('--skip-build');
const count = Math.max(1, Math.min(6, Number(args.find(arg => !arg.startsWith('--')) ?? 2)));

const base = resolve('.dev-peers');
const binary = resolve('target/debug/alverchat-desktop');
const devUrl = 'http://127.0.0.1:1420';
const children: ReturnType<typeof spawn>[] = [];

function shutdown(code: number): never {
  for (const child of children) child.kill('SIGINT');
  process.exit(code);
}

process.on('SIGINT', () => shutdown(0));
process.on('SIGTERM', () => shutdown(0));

mkdirSync(base, { recursive: true });

if (!skipBuild) {
  const build = spawnSync('cargo', ['build', '-p', 'alverchat-desktop'], { stdio: 'inherit' });
  if (build.status !== 0) shutdown(build.status ?? 1);
}

const vite = spawn('pnpm', ['exec', 'vite', '--host', '127.0.0.1', '--port', '1420', '--strictPort'], {
  stdio: 'inherit',
  env: { ...process.env, ALVERCHAT_DEV_URL: devUrl },
});
children.push(vite);
vite.on('exit', code => shutdown(code ?? 0));

async function waitForDevServer(): Promise<boolean> {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(devUrl);
      if (response.ok) return true;
    } catch {
      // Vite is still booting.
    }
    await new Promise(done => setTimeout(done, 500));
  }
  return false;
}

if (!(await waitForDevServer())) {
  console.error(`Vite did not answer on ${devUrl} within 60s.`);
  shutdown(1);
}

for (let index = 1; index <= count; index += 1) {
  const directory = resolve(base, `peer-${index}`);
  mkdirSync(directory, { recursive: true });
  const child = spawn(binary, [], {
    stdio: 'inherit',
    env: {
      ...process.env,
      ALVERCHAT_MOCK_PEER: String(index),
      ALVERCHAT_DATA_DIR: resolve(directory, 'data'),
      ALVERCHAT_CONFIG_DIR: resolve(directory, 'config'),
    },
  });
  child.on('exit', code => {
    if (children.includes(child)) console.error(`peer-${index} exited with code ${code}.`);
  });
  children.push(child);
  console.log(`peer-${index}: ${binary} (data ${directory})`);
}
