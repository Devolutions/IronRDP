import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repoRoot = fileURLToPath(new URL('../../', import.meta.url));
execFileSync('cargo', ['xtask', 'web', 'build-replay'], { cwd: repoRoot, stdio: 'inherit' });
