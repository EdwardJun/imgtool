#!/usr/bin/env node
// 编译 imgtool 并复制为 Tauri sidecar：apps/desktop/src-tauri/binaries/imgtool-<target-triple>[.exe]
// 用法：node scripts/build-sidecar.mjs [--release] [--target <triple>]
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, chmodSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const release = args.includes('--release');
const targetIdx = args.indexOf('--target');
const hostTriple = execFileSync('rustc', ['-vV'], { encoding: 'utf8' }).match(/host: (\S+)/)[1];
const triple = targetIdx >= 0 ? args[targetIdx + 1] : hostTriple;

const cargoArgs = ['build', '-p', 'imgtool'];
if (release) cargoArgs.push('--release');
if (targetIdx >= 0) cargoArgs.push('--target', triple);
console.log(`[sidecar] cargo ${cargoArgs.join(' ')}`);
execFileSync('cargo', cargoArgs, { cwd: root, stdio: 'inherit' });

const ext = triple.includes('windows') ? '.exe' : '';
const profileDir = release ? 'release' : 'debug';
const built = targetIdx >= 0 ? join(root, 'target', triple, profileDir, `imgtool${ext}`) : join(root, 'target', profileDir, `imgtool${ext}`);
const destDir = join(root, 'apps/desktop/src-tauri/binaries');
mkdirSync(destDir, { recursive: true });
const dest = join(destDir, `imgtool-${triple}${ext}`);
copyFileSync(built, dest);
if (!ext) chmodSync(dest, 0o755);
console.log(`[sidecar] ${dest}`);
