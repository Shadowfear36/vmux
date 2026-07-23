// Kills any running vmuxd.exe before a rebuild — otherwise Windows refuses
// to overwrite the locked binary (the daemon is designed to keep running
// independently of vmux/dev-server restarts, so a stale instance from a
// previous run is often still alive). Safe to run when nothing's running;
// taskkill's "not found" case is expected, not an error.
import { spawnSync } from 'node:child_process';

spawnSync('taskkill', ['/F', '/IM', 'vmuxd.exe'], { stdio: 'inherit' });
