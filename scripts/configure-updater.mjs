import { readFile, writeFile } from 'node:fs/promises';

const repository = process.env.GITHUB_REPOSITORY;
if (!repository || !/^[\w.-]+\/[\w.-]+$/.test(repository)) {
  throw new Error('GITHUB_REPOSITORY must be in owner/repository format.');
}

const configPath = 'apps/desktop/src-tauri/tauri.conf.json';
const config = JSON.parse(await readFile(configPath, 'utf8'));
config.plugins ??= {};
config.plugins.updater ??= {};
config.plugins.updater.endpoints = [
  `https://github.com/${repository}/releases/latest/download/latest.json`,
];
await writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`);
