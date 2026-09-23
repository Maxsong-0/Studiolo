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
  scripts/test-check-preview-exercise.py
  scripts/test-linux.sh scripts/run-preview-tests.sh scripts/test-preview-package.py
  scripts/test-release-package.py scripts/test-install-package.py .github/workflows/ci.yml
  deploy/preview/studiolo-preview.service.in
  deploy/preview/studiolo-access.service.in deploy/preview/studiolo-tunnel.service
  deploy/preview/install-runtime-package.sh docs/m3/release-packaging.md
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
python3 "$STUDIOLO_OUT/source/scripts/test-check-preview-exercise.py"
python3 "$STUDIOLO_OUT/source/scripts/test-preview-package.py"
python3 "$STUDIOLO_OUT/source/scripts/test-release-package.py"
python3 "$STUDIOLO_OUT/source/scripts/test-install-package.py"
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
cp "$STUDIOLO_OUT/source/deploy/preview/install-runtime-package.sh" "$STUDIOLO_OUT/payload/"

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
    # 生产程序构建的 Cargo JSON 是特征与链接结果的唯一凭据；三次构建追加到同一份台账。
    cargo build --release --locked -p studiolo-local-server --features studiolo-auth/production --bin studiolo-local-server --message-format=json > /build-meta/products.jsonl
    cargo build --release --locked -p studiolo-local-server --features studiolo-auth/production --bin studiolo-maintain --message-format=json >> /build-meta/products.jsonl
    cargo build --release --locked -p studiolo-auth-admin --features production --message-format=json >> /build-meta/products.jsonl
    mkdir /out/bin
    install -m 755 /build-target/release/studiolo-local-server /out/bin/
    install -m 755 /build-target/release/studiolo-auth-admin /out/bin/
    install -m 755 /build-target/release/studiolo-maintain /out/bin/
    rustc --version > /out/rust-version.txt
    ldd /out/bin/studiolo-local-server > /out/runtime-libraries.txt
    ldd /out/bin/studiolo-auth-admin > /out/auth-admin-runtime-libraries.txt
    ldd /out/bin/studiolo-maintain > /out/maintain-runtime-libraries.txt
    if grep -Fq "not found" /out/runtime-libraries.txt /out/auth-admin-runtime-libraries.txt /out/maintain-runtime-libraries.txt; then
      printf "%s\n" "生产程序缺少运行库，拒绝打包。" >&2
      exit 1
    fi
  '

# Cargo JSON 是可执行程序的唯一来源；只在本机解析，不向服务器引入 Node 或构建日志。
node - "$STUDIOLO_OUT" "$STUDIOLO_SOURCE_ROOT" <<'NODE'
const fs = require('node:fs');
const path = require('node:path');
const [out, expectedRoot] = process.argv.slice(2);
const artifacts = new Map();
const artifactOwners = new Map();
const packageIds = new Map();
const identities = new Map();
const targets = new Set();
const libraries = new Set();
let complete = false;
for (const line of fs.readFileSync(path.join(out, 'build-meta/tests.jsonl'), 'utf8').split('\n')) {
  if (!line.trim()) continue;
  const item = JSON.parse(line);
  if (item.reason === 'build-finished') complete = item.success === true;
  if (item.reason !== 'compiler-artifact' || item.profile?.test !== true || !item.executable) continue;
  if (typeof item.executable !== 'string' || typeof item.manifest_path !== 'string'
      || !item.manifest_path.startsWith(expectedRoot + '/')) throw new Error('unexpected test source root');
  const manifest = path.posix.relative(expectedRoot, item.manifest_path);
  const crate = /^crates\/([A-Za-z0-9_-]+)\/Cargo\.toml$/.exec(manifest)?.[1];
  if (!crate || typeof item.package_id !== 'string' || !item.package_id
      || typeof item.target?.name !== 'string' || !Array.isArray(item.target.kind)) {
    throw new Error('测试程序缺少工作区包身份');
  }
  if ((packageIds.has(crate) && packageIds.get(crate) !== item.package_id)
      || (identities.has(item.package_id) && identities.get(item.package_id) !== crate)) {
    throw new Error('Cargo 包身份与源码 manifest 不一致');
  }
  packageIds.set(crate, item.package_id);
  identities.set(item.package_id, crate);
  const relative = path.posix.relative('/build-target', item.executable);
  if (!/^release\/(?:deps\/|examples\/)?[A-Za-z0-9][A-Za-z0-9._-]*$/.test(relative)) {
    throw new Error('unsafe test executable path');
  }
  const name = path.posix.basename(relative);
  const owner = JSON.stringify([crate, item.target.name, item.target.kind]);
  if (artifacts.has(name) && (artifacts.get(name) !== relative || artifactOwners.get(name) !== owner)) {
    throw new Error('同一测试程序不能充当多个包或测试目标');
  }
  artifacts.set(name, relative);
  artifactOwners.set(name, owner);
  if (item.target.kind.includes('test')) targets.add(`${crate}/${item.target.name}`);
  if (item.target.kind.includes('lib')) libraries.add(crate);
}
// 按具体包及测试目标验收；T-046～T-048 新增的 crate 一并纳入，缺一即拒绝出包。
const required = ['studiolo/architecture', 'vault-broker/linux_path_security',
  'local-server/startup_contract', 'auth-admin/cli_process', 'local-server/maintenance_contract',
  'backup/contract',
  'observability/health_contract', 'observability/quota_contract', 'observability/alert_contract',
  'observability/deadman_contract', 'observability/store_contract', 'observability/whitelist_contract',
  'host-config/sync_transport_contract', 'host-config/alerting_contract'];
// sync-transport 的一次一运行契约写在 lib 目标内（`#[path] tests/owned_run_private`），
// 没有独立集成测试目标；按 lib 测试程序验收，不用其他 crate 的同名目标充数。
const requiredLibraries = ['sync-transport'];
if (!complete || artifacts.size === 0 || required.some(name => !targets.has(name))
    || requiredLibraries.some(crate => !libraries.has(crate))) {
  throw new Error('incomplete test build or missing required safety tests');
}
const names = [...artifacts.keys()].sort();
fs.writeFileSync(path.join(out, 'build-meta/test-copy-list'), names.map(name => `${artifacts.get(name)}\t${name}\n`).join(''));
fs.writeFileSync(path.join(out, 'payload/test-list'), names.map(name => `tests/${name}\n`).join(''));
console.log(`已编译 ${names.length} 个 Rust 测试程序；尚未执行。`);
NODE

# 生产程序的特征与链接结果同样只以 Cargo JSON 为准；回执随包发布，供服务器原生验收复核。
node - "$STUDIOLO_OUT" "$STUDIOLO_SOURCE_ROOT" <<'NODE_PRODUCTS'
const fs = require('node:fs');
const path = require('node:path');
const [out, expectedRoot] = process.argv.slice(2);
// 运行包应发布的生产程序，及其所属的工作区 crate 目录。
const products = new Map([
  ['studiolo-local-server', 'local-server'],
  ['studiolo-maintain', 'local-server'],
  ['studiolo-auth-admin', 'auth-admin'],
]);
// T-046～T-048 新增的运行时 crate 与既有安全边界 crate 必须真实参与生产构建；
// 只被工作区声明而没有进入链接结果，等同于没有发布这些程序。
const linked = ['backup', 'observability', 'sync-transport', 'host-config',
  'core', 'vault-broker', 'auth'];
const features = new Map();
const executables = new Map();
let finished = 0;
for (const line of fs.readFileSync(path.join(out, 'build-meta/products.jsonl'), 'utf8').split('\n')) {
  if (!line.trim()) continue;
  const item = JSON.parse(line);
  if (item.reason === 'build-finished') {
    if (item.success !== true) throw new Error('生产程序构建未成功，拒绝出包');
    finished += 1;
    continue;
  }
  if (item.reason !== 'compiler-artifact') continue;
  if (item.profile?.test === true) throw new Error('生产构建混入了测试产物，拒绝出包');
  if (typeof item.manifest_path !== 'string') throw new Error('生产构建产物缺少 manifest 路径');
  // 第三方依赖来自注册表缓存，不在源码快照内；只对工作区内的包判定身份与特征。
  if (!item.manifest_path.startsWith(expectedRoot + '/')) continue;
  const manifest = path.posix.relative(expectedRoot, item.manifest_path);
  const crate = /^crates\/([A-Za-z0-9_-]+)\/Cargo\.toml$/.exec(manifest)?.[1];
  if (!crate || typeof item.target?.name !== 'string' || !Array.isArray(item.target.kind)
      || !Array.isArray(item.features)) {
    throw new Error('工作区生产产物缺少包身份或特征清单');
  }
  const enabled = [...item.features].sort().join(',');
  if (features.has(crate) && features.get(crate) !== enabled) {
    throw new Error('同一工作区包在三次生产构建中启用了不同特征');
  }
  features.set(crate, enabled);
  if (!item.target.kind.includes('bin')) continue;
  if (typeof item.executable !== 'string' || !item.executable) continue;
  if (products.has(item.target.name) && products.get(item.target.name) !== crate) {
    throw new Error('生产程序不属于预期的工作区包');
  }
  executables.set(item.target.name, crate);
}
if (finished !== products.size) throw new Error('生产构建次数与应发布的程序数不符');
for (const [product, crate] of products) {
  if (executables.get(product) !== crate) throw new Error('运行包缺少应发布的生产程序：' + product);
}
for (const crate of linked) {
  if (!features.has(crate)) throw new Error('生产程序未链接必须随包发布的 crate：' + crate);
}
for (const [crate, enabled] of features) {
  if (enabled.split(',').includes('test-support')) {
    throw new Error('生产构建启用了测试验证器特征：' + crate);
  }
}
if (!features.get('auth').split(',').includes('production')) {
  throw new Error('身份模块未以生产特征构建，拒绝出包');
}
const receipt = [...products].map(([product, crate]) => `product:${product} crate:${crate}\n`)
  .concat([...features.keys()].sort().map(crate => `crate:${crate} features:${features.get(crate)}\n`));
fs.writeFileSync(path.join(out, 'payload/product-features.txt'), receipt.join(''));
console.log(`已核对 ${products.size} 个生产程序与 ${features.size} 个工作区包的特征；回执已随包。`);
NODE_PRODUCTS

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
