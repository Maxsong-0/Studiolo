#!/usr/bin/env python3
"""把 deploy/agent/upgrade-read-supply.sh.in 渲染成绑定到一个具体运行包与一个具体旧基线的一次性安装器。
用法：python3 scripts/render-agent-upgrade.py <运行包 payload 目录> --old-release <线上发布名> \
        --old-binary-sha <线上二进制 sha256> --old-unit-sha <已安装系统单元 sha256> --output <输出文件>
只在发布侧运行：读本地文件、算摘要、写一个文件；不联网、不连服务器、不执行任何东西。
安装器要用 root 执行，所以七个占位符一律由本脚本填入并校验，不手工 sed。
"""
import argparse
import hashlib
from pathlib import Path
import re
import sys

TEMPLATE = Path(__file__).resolve().parent.parent / "deploy/agent/upgrade-read-supply.sh.in"
TEMPLATE_NAME = "deploy/agent/upgrade-read-supply.sh.in"
RELEASE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$")
SHA = re.compile(r"^[0-9a-f]{64}$")
ROOT = "/home/maxsong/.local/share/studiolo-agent-releases/"


class RenderFailed(Exception):
    pass


def digest(path):
    if path.is_symlink() or not path.is_file():
        raise RenderFailed(f"运行包缺少普通文件：{path.name}")
    return hashlib.sha256(path.read_bytes()).hexdigest()


def render(payload, template, old_release, old_binary_sha, old_unit_sha):
    expected = (payload / "expected-source-root").read_text(encoding="utf-8").strip()
    if not (expected.startswith(ROOT) and expected.endswith("/source")):
        raise RenderFailed("expected-source-root 不是预期的服务器发布路径")
    release = expected[len(ROOT):-len("/source")]
    if not RELEASE.match(release) or not RELEASE.match(old_release) or release == old_release:
        raise RenderFailed("发布名不合法，或旧基线与新发布同名")
    if not SHA.match(old_binary_sha) or not SHA.match(old_unit_sha):
        raise RenderFailed("旧基线摘要必须是 64 位小写十六进制")
    # 模板必须就是这个运行包里带的那一份：否则安装器的逻辑与随包源码对不上。
    sums = (payload / "SOURCE_SHA256SUMS").read_text(encoding="utf-8").splitlines()
    packaged = {line.split("  ", 1)[1].removeprefix("./"): line.split("  ", 1)[0] for line in sums if "  " in line}
    if packaged.get(TEMPLATE_NAME) != digest(template):
        raise RenderFailed("模板与运行包内的同名文件摘要不一致；请用打包时的同一提交渲染")
    values = {
        "@RELEASE@": release,
        "@BINARY_SHA@": digest(payload / "bin/studiolo-agent"),
        "@UNIT_SHA@": digest(payload / "studiolo-agent-system.service"),
        "@SOURCE_SHA@": digest(payload / "SOURCE_SHA256SUMS"),
        "@FILES_SHA@": digest(payload / "source-files"),
        "@OLD_RELEASE@": old_release,
        "@OLD_BINARY_SHA@": old_binary_sha,
        "@OLD_UNIT_SHA@": old_unit_sha,
    }
    if values["@BINARY_SHA@"] == old_binary_sha:
        raise RenderFailed("新旧二进制摘要相同，无可升级")
    text = template.read_text(encoding="utf-8")
    for key, value in values.items():
        if key not in text:
            raise RenderFailed(f"模板缺少占位符 {key}")
        text = text.replace(key, value)
    if re.search(r"@[A-Z_]+@", text):
        raise RenderFailed("渲染后仍有未填的占位符")
    return text


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("payload", type=Path)
    parser.add_argument("--old-release", required=True)
    parser.add_argument("--old-binary-sha", required=True)
    parser.add_argument("--old-unit-sha", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--template", type=Path, default=TEMPLATE)
    args = parser.parse_args(argv)
    try:
        if args.output.exists() or args.output.is_symlink():
            raise RenderFailed("输出文件已存在，不覆盖")
        text = render(args.payload, args.template, args.old_release, args.old_binary_sha, args.old_unit_sha)
        args.output.write_text(text, encoding="utf-8")
        args.output.chmod(0o600)
    except (RenderFailed, OSError, UnicodeError, IndexError) as error:
        print(f"渲染失败：{error}", file=sys.stderr)
        return 1
    print(f"已渲染一次性安装器：{args.output}（{args.old_release} → 见文件内 release=）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
