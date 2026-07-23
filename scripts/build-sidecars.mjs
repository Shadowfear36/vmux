// Builds vmuxctl/vmuxd in release mode and copies them into
// src-tauri/binaries/ under the target-triple-suffixed name Tauri's
// `externalBin` ("sidecar") bundling convention requires — see
// tauri.conf.json's `bundle.externalBin` and docs/session-reattach-design.md.
// Run before `tauri build`; not needed for `tauri dev` (see build:vmuxctl/
// build:vmuxd, which build debug binaries next to vmux.exe's own debug dir,
// where the app already looks for them at runtime).

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const srcTauri = path.join(repoRoot, 'src-tauri');
const manifestPath = path.join(srcTauri, 'Cargo.toml');
const binariesDir = path.join(srcTauri, 'binaries');

// This is a Windows-only app (native Win32 HWNDs, ConPTY) — hardcoding the
// triple avoids needing to shell out to `rustc -vV` to discover it.
const TARGET_TRIPLE = 'x86_64-pc-windows-msvc';

const BINARIES = ['vmuxctl', 'vmuxd'];

console.log(`[build-sidecars] cargo build --release --bin ${BINARIES.join(' --bin ')}`);
execFileSync(
  'cargo',
  ['build', '--release', ...BINARIES.flatMap(b => ['--bin', b]), '--manifest-path', manifestPath],
  { stdio: 'inherit' }
);

if (!existsSync(binariesDir)) mkdirSync(binariesDir, { recursive: true });

for (const bin of BINARIES) {
  const src = path.join(srcTauri, 'target', 'release', `${bin}.exe`);
  const dest = path.join(binariesDir, `${bin}-${TARGET_TRIPLE}.exe`);
  copyFileSync(src, dest);
  console.log(`[build-sidecars] ${src} -> ${dest}`);
}
