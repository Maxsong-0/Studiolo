#!/usr/bin/env bash
# 从白名单源码快照交叉构建 Linux x86_64 示例包；Rust 测试必须随后在服务器原生执行。
# 用法：bash scripts/package-preview.sh /绝对路径/尚不存在的输出目录
set -euo pipefail
if [[ $# != 1 || "$1" != /* || -e "$1" || -L "$1" ]]; then
  printf '%s\n' '请提供一个尚不存在、父目录已存在的绝对输出目录。' >&2
  exit 2
fi
STUDIOLO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STUDIOLO_OUT="$1"
STUDIOLO_RELEASE="$(basename "$STUDIOLO_OUT")"
if [[ ! "$STUDIOLO_RELEASE" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$ || ! -d "$(dirname "$STUDIOLO_OUT")" ]]; then
  printf '%s\n' '发布名须以字母数字开头，仅含字母数字点下划线短横线，最多80字符；父目录必须存在。' >&2
  exit 2
fi
STUDIOLO_SOURCE_ROOT="/home/maxsong/.local/share/studiolo-preview/releases/$STUDIOLO_RELEASE/source"
STUDIOLO_TARGET_VOLUME="studiolo-preview-target-amd64-$STUDIOLO_RELEASE"
STUDIOLO_IMAGE='rust:1.97.1-bookworm'
mkdir "$STUDIOLO_OUT"
mkdir "$STUDIOLO_OUT/source" "$STUDIOLO_OUT/payload" "$STUDIOLO_OUT/build-meta"
cd "$STUDIOLO_ROOT"
# 显式白名单：不归档 .git、凭据、缓存或用户笔记。
STUDIOLO_INPUTS=(
  Cargo.toml Cargo.lock rust-toolchain.toml crates
  web/src web/package.json web/package-lock.json web/index.html
  web/tsconfig.json web/vite.config.ts web/eslint.config.js web/.prettierrc.json web/.prettierignore
  examples/m1-vault scripts/package-preview.sh scripts/check-preview.py scripts/test-check-preview.py
  scripts/test-linux.sh scripts/run-preview-tests.sh .github/workflows/ci.yml
  deploy/preview/studiolo-preview.service.in
  deploy/preview/studiolo-access.service.in deploy/preview/studiolo-tunnel.service
)
COPYFILE_DISABLE=1 tar -cf - "${STUDIOLO_INPUTS[@]}" |
  tar -xf - -C "$STUDIOLO_OUT/source"
git rev-parse HEAD > "$STUDIOLO_OUT/payload/base-commit.txt"
printf '%s\n' "$STUDIOLO_SOURCE_ROOT" > "$STUDIOLO_OUT/payload/expected-source-root"
printf '%s\n' '来源：当前工作区文件快照（包含未提交改动，如有）；base-commit 不是本运行包的版本标识。' \
  > "$STUDIOLO_OUT/payload/provenance.txt"
COPYFILE_DISABLE=1 tar -czf "$STUDIOLO_OUT/payload/source.tar.gz" \
  -C "$STUDIOLO_OUT/source" .

# 在独立源码快照构建，避免产物与已归档源码发生偏差。
python3 "$STUDIOLO_OUT/source/scripts/test-check-preview.py"
npm --prefix "$STUDIOLO_OUT/source/web" ci --ignore-scripts
npm --prefix "$STUDIOLO_OUT/source/web" test
npm --prefix "$STUDIOLO_OUT/source/web" run lint
npm --prefix "$STUDIOLO_OUT/source/web" run format
npm --prefix "$STUDIOLO_OUT/source/web" run build
cp -R "$STUDIOLO_OUT/source/web/dist" "$STUDIOLO_OUT/payload/web"
cp -R "$STUDIOLO_OUT/source/examples/m1-vault" "$STUDIOLO_OUT/payload/example-vault"
cp "$STUDIOLO_OUT/source/scripts/check-preview.py" "$STUDIOLO_OUT/payload/"
cp "$STUDIOLO_OUT/source/scripts/run-preview-tests.sh" "$STUDIOLO_OUT/payload/"
cp "$STUDIOLO_OUT/source/deploy/preview/studiolo-preview.service.in" "$STUDIOLO_OUT/payload/"
sed "s/@RELEASE@/$STUDIOLO_RELEASE/g" \
  "$STUDIOLO_OUT/payload/studiolo-preview.service.in" > "$STUDIOLO_OUT/payload/studiolo-preview.service"
cp "$STUDIOLO_OUT/source/deploy/preview/studiolo-access.service.in" "$STUDIOLO_OUT/payload/"
sed "s/@RELEASE@/$STUDIOLO_RELEASE/g" \
  "$STUDIOLO_OUT/payload/studiolo-access.service.in" > "$STUDIOLO_OUT/payload/studiolo-access.service"
cp "$STUDIOLO_OUT/source/deploy/preview/studiolo-tunnel.service" "$STUDIOLO_OUT/payload/"

docker pull --platform linux/amd64 "$STUDIOLO_IMAGE"
# Cargo 缓存可能保留编译时的绝对源码路径；每个发布包使用全新目标卷。
if docker volume inspect "$STUDIOLO_TARGET_VOLUME" >/dev/null 2>&1; then
  printf '%s\n' '该发布名的构建卷已存在，请使用新发布名，避免复用旧绝对路径。' >&2
  exit 1
fi
docker volume create "$STUDIOLO_TARGET_VOLUME" >/dev/null
STUDIOLO_IMAGE_REF="$(docker image inspect --platform linux/amd64 "$STUDIOLO_IMAGE" --format '{{index .RepoDigests 0}}')"
if [[ ! "$STUDIOLO_IMAGE_REF" =~ @sha256:[0-9a-f]{64}$ ]]; then
  printf '%s\n' '无法固定构建镜像的 RepoDigest，停止构建。' >&2
  exit 1
fi
docker image inspect --platform linux/amd64 "$STUDIOLO_IMAGE" \
  --format '{{index .RepoDigests 0}} {{.Id}} {{.Architecture}}' > "$STUDIOLO_OUT/payload/build-image.txt"
docker run --rm --platform linux/amd64 \
  -v "$STUDIOLO_OUT/source":"$STUDIOLO_SOURCE_ROOT":ro -w "$STUDIOLO_SOURCE_ROOT" \
  -v "$STUDIOLO_OUT/payload":/out \
  -v "$STUDIOLO_OUT/build-meta":/build-meta \
  -v studiolo-preview-cargo-amd64:/usr/local/cargo/registry \
  -v "$STUDIOLO_TARGET_VOLUME":/build-target \
  -e CARGO_TARGET_DIR=/build-target \
  "$STUDIOLO_IMAGE_REF" bash -euo pipefail -c '
    cargo test --workspace --all-targets --release --locked --no-run --message-format=json > /build-meta/tests.jsonl
    cargo build --release --locked -p studiolo-local-server --features studiolo-auth/production
    cargo build --release --locked -p studiolo-auth-admin --features production
    mkdir /out/bin
    install -m 755 /build-target/release/studiolo-local-server /out/bin/
    install -m 755 /build-target/release/studiolo-auth-admin /out/bin/
    rustc --version > /out/rust-version.txt
    ldd /out/bin/studiolo-local-server > /out/runtime-libraries.txt
    ldd /out/bin/studiolo-auth-admin > /out/auth-admin-runtime-libraries.txt
  '

# Cargo JSON 是可执行程序的唯一来源；只在本机解析，不向服务器引入 Node 或构建日志。
node - "$STUDIOLO_OUT" "$STUDIOLO_SOURCE_ROOT" <<'NODE'
const fs = require('node:fs');
const path = require('node:path');
const [out, expectedRoot] = process.argv.slice(2);
const artifacts = new Map();
const targets = new Set();
let complete = false;
for (const line of fs.readFileSync(path.join(out, 'build-meta/tests.jsonl'), 'utf8').split('\n')) {
  if (!line.trim()) continue;
  const item = JSON.parse(line);
  if (item.reason === 'build-finished') complete = item.success === true;
  if (item.reason !== 'compiler-artifact' || item.profile?.test !== true || !item.executable) continue;
  if (typeof item.executable !== 'string' || typeof item.manifest_path !== 'string'
      || !item.manifest_path.startsWith(expectedRoot + '/')) throw new Error('unexpected test source root');
  const relative = path.posix.relative('/build-target', item.executable);
  if (!/^release\/(?:deps\/|examples\/)?[A-Za-z0-9][A-Za-z0-9._-]*$/.test(relative)) {
    throw new Error('unsafe test executable path');
  }
  const name = path.posix.basename(relative);
  if (artifacts.has(name) && artifacts.get(name) !== relative) throw new Error('duplicate test basename');
  artifacts.set(name, relative);
  targets.add(item.target.name);
}
if (!complete || artifacts.size === 0 || !targets.has('architecture') || !targets.has('linux_path_security')
    || !targets.has('startup_contract') || !targets.has('cli_process')) {
  throw new Error('incomplete test build or missing required safety tests');
}
const names = [...artifacts.keys()].sort();
fs.writeFileSync(path.join(out, 'build-meta/test-copy-list'), names.map(name => `${artifacts.get(name)}\t${name}\n`).join(''));
fs.writeFileSync(path.join(out, 'payload/test-list'), names.map(name => `tests/${name}\n`).join(''));
console.log(`已编译 ${names.length} 个 Rust 测试程序；尚未执行。`);
NODE

docker run --rm --platform linux/amd64 \
  -v "$STUDIOLO_OUT/payload":/out \
  -v "$STUDIOLO_OUT/build-meta":/build-meta:ro \
  -v "$STUDIOLO_TARGET_VOLUME":/build-target:ro \
  "$STUDIOLO_IMAGE_REF" bash -euo pipefail -c '
    mkdir /out/tests
    while IFS=$'"'"'\t'"'"' read -r relative name; do
      [[ "$relative" =~ ^release/(deps/|examples/)?[A-Za-z0-9][A-Za-z0-9._-]*$ ]]
      [[ "$name" == "${relative##*/}" && -f "/build-target/$relative" && ! -L "/build-target/$relative" ]]
      install -m 755 "/build-target/$relative" "/out/tests/$name"
    done < /build-meta/test-copy-list
    chmod 755 /out/run-preview-tests.sh
    cd /out
    find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum > /tmp/studiolo-checksums
    mv /tmp/studiolo-checksums SHA256SUMS
  '
printf '已交叉构建运行包：%s/payload；Rust 测试仍须在记录的服务器源码目录原生运行。\n' "$STUDIOLO_OUT"
