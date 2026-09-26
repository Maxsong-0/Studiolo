#!/usr/bin/env bash
# T-132 P2：以 studiolo-sync 身份安装钉版本的 Node 与 obsidian-headless，不需要 sudo 权限本身。
# 由用户执行：sudo -u studiolo-sync -H bash -s < ~/studiolo-p2/p2-10-install.sh
set -euo pipefail
umask 077
[ "$(id -un)" = studiolo-sync ] || { echo "必须以 studiolo-sync 身份运行" >&2; exit 1; }
cd "$HOME"

NODE_VER=v22.23.2
NODE_TAR=node-${NODE_VER}-linux-x64.tar.xz
NODE_SHA=d60acfe00a2932254bb0ad20e01b0d74397a0875595de719654b214f4b03f307
OB_VER=0.0.14
OB_TGZ=obsidian-headless-${OB_VER}.tgz
OB_SHA=ef75298ed3a356dcb2a4de98c6efe10b2ac64b00556007118b6ac70535e73156

OPT="$HOME/opt"
DL="$HOME/dl"
NODE_DIR="$OPT/node-$NODE_VER"
OB_DIR="$OPT/obsidian-headless-$OB_VER"
RECEIPT="$HOME/p2/install-receipt.txt"

for p in "$NODE_DIR" "$OB_DIR" "$HOME/bin/ob"; do
  if [ -e "$p" ]; then echo "$p 已存在，停止，不覆盖" >&2; exit 1; fi
done
mkdir -p "$OPT" "$DL" "$HOME/bin" "$HOME/p2"

curl -fsSL --proto '=https' --tlsv1.2 -o "$DL/$NODE_TAR" "https://nodejs.org/dist/$NODE_VER/$NODE_TAR"
echo "$NODE_SHA  $DL/$NODE_TAR" | sha256sum -c -
curl -fsSL --proto '=https' --tlsv1.2 -o "$DL/$OB_TGZ" "https://registry.npmjs.org/obsidian-headless/-/$OB_TGZ"
echo "$OB_SHA  $DL/$OB_TGZ" | sha256sum -c -

mkdir "$NODE_DIR"
tar -xJf "$DL/$NODE_TAR" -C "$NODE_DIR" --strip-components=1 --no-same-owner
export PATH="$NODE_DIR/bin:/usr/bin:/bin"
export npm_config_update_notifier=false npm_config_fund=false npm_config_audit=false
[ "$(node --version)" = "$NODE_VER" ] || { echo "node 版本不符" >&2; exit 1; }

# 本机没有 make/g++：better-sqlite3 只能用官方预编译产物；取不到就失败，不装编译工具链
mkdir "$OB_DIR"
npm install --global --prefix "$OB_DIR" "$DL/$OB_TGZ"

PKG="$OB_DIR/lib/node_modules/obsidian-headless"
CLI_REL=$(node -e 'const p=require(process.argv[1]);const b=typeof p.bin==="string"?p.bin:p.bin.ob;process.stdout.write(b)' "$PKG/package.json")
CLI="$PKG/$CLI_REL"
[ -f "$CLI" ] || { echo "找不到 ob 入口 $CLI" >&2; exit 1; }
printf '#!/bin/sh\nexec %s %s "$@"\n' "$NODE_DIR/bin/node" "$CLI" > "$HOME/bin/ob"
chmod 0700 "$HOME/bin/ob"

{
  echo "# T-132 P2 安装回执 $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "node: $(node --version)  tarball_sha256: $NODE_SHA"
  echo "obsidian-headless: $(node -p "require('$PKG/package.json').version")  tgz_sha256: $OB_SHA"
  echo "ob 入口: $CLI"
  echo "## 依赖树"
  npm ls --global --prefix "$OB_DIR" --all 2>&1 || true
  echo "## native 产物"
  find "$OB_DIR" -name '*.node' -type f -exec sha256sum {} \;
  echo "## 安装目录总摘要（按路径排序的逐文件 sha256 再取 sha256）"
  (cd "$OB_DIR" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sha256sum)
  echo "## 自检"
  "$HOME/bin/ob" --version 2>&1 || "$HOME/bin/ob" --help 2>&1 | head -5
} > "$RECEIPT"
cat "$RECEIPT"
