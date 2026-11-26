import { execFileSync, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

const root = path.resolve(import.meta.dirname, '..')
const target =
  process.argv[2] ??
  `${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-apple-darwin`
const architectures = {
  'aarch64-apple-darwin': 'arm64',
  'x86_64-apple-darwin': 'amd64',
}
const arch = architectures[target]
if (!arch || process.argv.length > 3) {
  throw new Error(
    'Usage: node scripts/build-hearthstone-core.mjs [aarch64-apple-darwin|x86_64-apple-darwin]',
  )
}
const cache =
  process.env.HEARTHSTONE_CORE_WORKDIR ??
  path.join(root, 'node_modules/.verge/hearthstone-core')
const go = process.env.GO_BINARY ?? 'go'
fs.mkdirSync(cache, { recursive: true })

function checkout(name, repository, tag, commit, patchName) {
  const directory = path.join(cache, name)
  if (!fs.existsSync(path.join(directory, '.git'))) {
    execFileSync(
      'git',
      ['clone', '--depth', '1', '--branch', tag, repository, directory],
      { stdio: 'inherit' },
    )
  }
  const head = execFileSync('git', ['rev-parse', 'HEAD'], {
    cwd: directory,
    encoding: 'utf8',
  }).trim()
  if (head !== commit)
    throw new Error(`Unexpected source commit in ${directory}: ${head}`)
  const patch = path.join(root, 'patches', patchName)
  const applied = spawnSync('git', ['apply', '--reverse', '--check', patch], {
    cwd: directory,
  })
  if (applied.status !== 0) {
    execFileSync('git', ['apply', '--check', patch], {
      cwd: directory,
      stdio: 'inherit',
    })
    execFileSync('git', ['apply', patch], { cwd: directory, stdio: 'inherit' })
  }
  return directory
}

checkout(
  'sing-tun-fast-close',
  'https://github.com/MetaCubeX/sing-tun.git',
  'v0.4.29',
  'e454d9c11e5a41ee348dd22fda93e9bb441ea2d8',
  'sing-tun-fast-reset.patch',
)
const source = checkout(
  'mihomo-fast-close',
  'https://github.com/MetaCubeX/mihomo.git',
  'v1.19.34',
  '0455d0d28c4526937380505153e65a843d8612e8',
  'mihomo-fast-reset.patch',
)
execFileSync(
  go,
  [
    'mod',
    'edit',
    '-replace',
    'github.com/metacubex/sing-tun=../sing-tun-fast-close',
  ],
  { cwd: source, stdio: 'inherit' },
)
const output = path.join(root, 'src-tauri/sidecar', `verge-mihomo-${target}`)
fs.mkdirSync(path.dirname(output), { recursive: true })
const version = 'v1.19.34-hsreset.1'
execFileSync(
  go,
  [
    'build',
    '-mod=readonly',
    '-tags',
    'with_gvisor',
    '-trimpath',
    '-ldflags',
    `-s -w -B gobuildid -X github.com/metacubex/mihomo/constant.Version=${version} -X github.com/metacubex/mihomo/constant.BuildTime=${new Date().toISOString()}`,
    '-o',
    output,
    '.',
  ],
  {
    cwd: source,
    stdio: 'inherit',
    env: {
      ...process.env,
      GOTOOLCHAIN: 'local',
      CGO_ENABLED: '0',
      GOOS: 'darwin',
      GOARCH: arch,
      GOAMD64: 'v2',
    },
  },
)
console.log(`Built Hearthstone reset core ${version}: ${output}`)
