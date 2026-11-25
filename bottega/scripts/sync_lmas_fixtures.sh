#!/usr/bin/env bash
# Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
# BottegaKit 的测试数据是 LMAS `fixtures/bottega/` 的原样副本。默认只比：有出入就列出来、退出码 1；--apply 才复制。
# 只读 LMAS 已提交的 main（git show），不读它的工作区。LMAS 在哪用 LMAS_REPO 指。
set -euo pipefail

lmas="${LMAS_REPO:-$HOME/All-Project/LMAS}"
here="$(cd "$(dirname "$0")/.." && pwd)/BottegaKit/Tests/BottegaKitTests/Fixtures"
mode="${1:-}"
[ -z "$mode" ] || [ "$mode" = --apply ] || { echo "用法：$0 [--apply]" >&2; exit 2; }

commit="$(git -C "$lmas" rev-parse main)"
new="$(mktemp -d)"
trap 'rm -rf "$new"' EXIT
different=0
for file in vectors.json events.jsonl session.jsonl; do
  git -C "$lmas" show "$commit:fixtures/bottega/$file" > "$new/$file"
  cmp -s "$new/$file" "$here/$file" && continue
  different=1
  echo "不一样：$file"
  [ "$mode" != --apply ] || cp "$new/$file" "$here/$file"
done
if [ "$different" = 0 ]; then
  echo "和 LMAS main ${commit:0:7} 一样"
elif [ "$mode" = --apply ]; then
  echo "已照 LMAS main ${commit:0:7} 换上；把 Fixtures/README.md 里的提交号改掉，再跑测试"
else
  exit 1
fi
