#!/usr/bin/env python3
"""项目注册表（T-209，ADR-034 方案 A）：新项目只改 scripts/project_registry.json 一处，其余由它生成。

AI 要读到一个项目，原来得手改三处，漏一处就读不到。现在三处各自这样跟上，两道门都还在：
- 副本清单（scripts/replica_common.py 的 ALLOW）和部署上限（deploy/ 下模板里的 --allow）：
  `sync` 从注册表重写，`check` 核对。改完仍要重发 Mac 副本包、发行并重启 agent，这一步要主人在场。
- 能力集的 read.project_whitelist：`desired` 拿一份现行能力集对着注册表出一份提案，照旧经策略平面由人批准。

注册表只记「路径、给不给 AI 看、备注」，不记代码仓库地址。只用标准库；不联网、不碰生产。

用法：
  python3 scripts/project_registry.py check [--vault <库根目录>]
  python3 scripts/project_registry.py sync [--dry-run]
  python3 scripts/project_registry.py desired --capability <现行能力集.json> [--grant <路径>]... [--grant-all]
"""
import argparse
import json
import os
import re
import stat
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from replica_common import Invalid, load_json, logical_path_ok  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
REGISTRY = "scripts/project_registry.json"
REPLICA = "scripts/replica_common.py"
DEPLOY = "deploy"
RULES = "_规则"
PROJECTS = "项目"
MAX_ENTRIES = 200
MAX_NOTE = 200
# 路径会原样写进 shell 双引号和 systemd 单元：这几个字符在那里有特殊含义，注册表里一律不收。
UNSAFE = set('"$`%\'\n\r\t')
BLOCK = re.compile(r'^ALLOW = sorted\(\[\n(?:    "[^"\n]*",\n)*\]\)\n', re.M)
RUN = re.compile(r'--allow "[^"\n]*"(?: --allow "[^"\n]*")*')


def fail(message):
    raise Invalid(message)


def parse_registry(raw):
    """读注册表；任何一处不合规都整份拒绝。返回 [(路径, 给不给 AI 看, 备注)]，按路径排序。"""
    try:
        value = load_json(raw)
    except (Invalid, ValueError, UnicodeDecodeError):
        fail("注册表不是合法 JSON，或有重复的键")
    if type(value) is not dict or set(value) != {"schema_version", "entries"}:
        fail("注册表顶层只许 schema_version 和 entries")
    if type(value["schema_version"]) is not int or value["schema_version"] != 1:
        fail("只认 schema_version 1")
    entries = value["entries"]
    if type(entries) is not list or not 0 < len(entries) <= MAX_ENTRIES:
        fail(f"entries 须为 1 到 {MAX_ENTRIES} 项的列表")
    seen, result = set(), []
    for entry in entries:
        if type(entry) is not dict or not {"path", "ai_visible"} <= set(entry) <= {"path", "ai_visible", "note"}:
            fail("每一项只许 path、ai_visible 和可选的 note")
        path, visible, note = entry["path"], entry["ai_visible"], entry.get("note", "")
        if type(visible) is not bool or type(note) is not str or len(note) > MAX_NOTE:
            fail(f"{path!r}：ai_visible 须为布尔值，note 须为不超过 {MAX_NOTE} 字的字符串")
        try:
            parts = logical_path_ok(path)
        except Invalid:
            fail(f"{path!r} 不是合法的逻辑路径")
        if not (parts == [RULES] or (len(parts) == 2 and parts[0] == PROJECTS)):
            fail(f"{path!r}：只收 {RULES} 或「{PROJECTS}/<一段项目名>」")
        if UNSAFE & set(path) or path != path.strip():
            fail(f"{path!r}：路径里有引号、$、%、反引号或首尾空白，不能进部署模板")
        if path in seen:
            fail(f"{path!r} 登记了两次")
        seen.add(path)
        result.append((path, visible, note))
    return sorted(result)


def load_registry(root=ROOT):
    return parse_registry((Path(root) / REGISTRY).read_bytes())


def allow_of(entries):
    """给 AI 看的路径：副本清单和部署上限都等于它。"""
    allow = sorted(path for path, visible, _ in entries if visible)
    if not allow:
        fail("注册表里没有一项给 AI 看：副本清单不能为空")
    return allow


def render_block(allow):
    return "ALLOW = sorted([\n" + "".join(f'    "{path}",\n' for path in allow) + "])\n"


def render_run(allow):
    return " ".join(f'--allow "{path}"' for path in allow)


def templates(root):
    """deploy/ 下带 --allow 的模板。自动发现、不写死清单：写死的文件清单正是反复漏改的根源。"""
    found = []
    for path in sorted((Path(root) / DEPLOY).rglob("*")):
        if path.is_symlink() or not path.is_file() or path.suffix in {".py", ".md"}:
            continue
        if "--allow" in path.read_text(encoding="utf-8", errors="replace"):
            found.append(path)
    return found


def planned(root, allow):
    """照注册表该是什么样；返回 ({要改的文件: 新内容}, 读不懂的地方)。只算不写。"""
    root = Path(root)
    changes, problems = {}, []
    replica = root / REPLICA
    text = replica.read_text(encoding="utf-8")
    if len(BLOCK.findall(text)) != 1:
        problems.append(f"{REPLICA}: 找不到唯一一处 ALLOW = sorted([...]) 清单")
    else:
        updated = BLOCK.sub(lambda _: render_block(allow), text)
        if updated != text:
            changes[replica] = updated
    found = templates(root)
    if not found:
        problems.append(f"{DEPLOY}/ 下没有任何带 --allow 的模板，发现逻辑失效")
    for path in found:
        name = path.relative_to(root).as_posix()
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            problems.append(f"{name}: 不是 UTF-8 文本，读不懂")
            continue
        runs = RUN.findall(text)
        # 一份模板里只许有一串连着写的 --allow "…"；散着写、不带引号的都当读不懂，不猜。
        if len(runs) != 1 or text.count("--allow") != runs[0].count("--allow"):
            problems.append(f'{name}: 须恰好一串连着写的 --allow "…"，这份读不懂')
            continue
        updated = text.replace(runs[0], render_run(allow))
        if updated != text:
            changes[path] = updated
    return changes, problems


def vault_report(vault, entries):
    """对着真实的库看一眼：登记了但库里没有的（副本捕获会整批失败），和库里有但没登记的。"""
    vault = Path(vault)
    missing, unregistered = [], []
    for path, visible, _ in entries:
        try:
            info = os.lstat(vault / path)
        except OSError:
            info = None
        if visible and (info is None or not stat.S_ISDIR(info.st_mode)):
            missing.append(path)
    registered = {path for path, _, _ in entries}
    projects = vault / PROJECTS
    if projects.is_dir():
        for child in sorted(projects.iterdir()):
            name = f"{PROJECTS}/{child.name}"
            if not child.name.startswith(".") and not child.is_symlink() and child.is_dir() and name not in registered:
                unregistered.append(name)
    return missing, unregistered


def check(root=ROOT, vault=None):
    """返回 (问题, 提示)。问题非空就是红。"""
    root = Path(root)
    try:
        entries = load_registry(root)
        allow = allow_of(entries)
    except (Invalid, OSError) as error:
        return [f"{REGISTRY}: {error}"], []
    changes, problems = planned(root, allow)
    problems += [f"{path.relative_to(root).as_posix()}: 和注册表对不上，跑一次 sync" for path in sorted(changes)]
    notes = []
    if vault is not None:
        missing, unregistered = vault_report(vault, entries)
        problems += [f"{path}: 登记了给 AI 看，但库里没有这个目录" for path in missing]
        notes += [f"{path}: 库里有、注册表里没登记" for path in unregistered]
    return problems, notes


def sync(root=ROOT, dry_run=False):
    """照注册表重写副本清单和部署模板。读不懂任何一处就一处都不写。返回 (改了的文件, 问题)。"""
    root = Path(root)
    changes, problems = planned(root, allow_of(load_registry(root)))
    if problems:
        return [], problems
    if not dry_run:
        for path, text in sorted(changes.items()):
            temporary = path.with_name(f".{path.name}.registry-tmp")
            temporary.write_text(text, encoding="utf-8")
            os.chmod(temporary, stat.S_IMODE(os.stat(path).st_mode))
            os.replace(temporary, path)
    return [path.relative_to(root).as_posix() for path in sorted(changes)], []


def overlaps(prefix, allow):
    """白名单前缀和部署上限有没有交集：相等、在某一项之下、或是某一项的上级。"""
    return any(prefix == path or prefix.startswith(path + "/") or path.startswith(prefix + "/") for path in allow)


def desired(capability, entries, grants=(), grant_all=False):
    """对着注册表改一份能力集的读取白名单；返回 (新能力集或 None, 说明)。只生成，不提交。

    - 和部署上限没有交集的前缀删掉（留着也读不到），同名的黑板写项目一并删；
    - grants 点名的路径加进去；grant_all 时把注册表里给 AI 看的全加上。
    能力集是按 AI 类型收窄用的，所以默认不自动扩大，要加哪个必须点名。
    """
    allow = allow_of(entries)
    try:
        read, write = capability["read"], capability["write"]
        whitelist, board = read["project_whitelist"], write["board_projects"]
        ok = (type(whitelist) is list and type(board) is list
              and all(type(item) is str for item in whitelist + board))
    except (KeyError, TypeError):
        ok = False
    if not ok:
        fail("能力集缺少 read.project_whitelist 或 write.board_projects，或它们不是字符串列表")
    for grant in grants:
        if grant not in allow:
            fail(f"{grant!r} 不在注册表给 AI 看的范围里，先登记")
    dead = sorted(prefix for prefix in whitelist if not overlaps(prefix, allow))
    kept = [prefix for prefix in whitelist if prefix not in dead]
    wanted = allow if grant_all else list(grants)
    added = sorted(path for path in wanted if not any(path == p or path.startswith(p + "/") for p in kept))
    updated = sorted(set(kept) | set(added))
    board_dead = sorted(project for project in board if not overlaps(project, allow))
    notes = [f"删掉（部署上限之外）：{prefix}" for prefix in dead]
    notes += [f"加上：{path}" for path in added]
    notes += [f"黑板写项目同步删掉：{project}" for project in board_dead]
    if not notes:
        return None, ["读取白名单已经和注册表对得上，不用提案"]
    result = json.loads(json.dumps(capability))
    result["read"]["project_whitelist"] = updated
    result["write"]["board_projects"] = [project for project in board if project not in board_dead]
    return result, notes


def main(argv=None):
    parser = argparse.ArgumentParser(description="项目注册表：核对、同步、生成能力集提案")
    commands = parser.add_subparsers(dest="command", required=True)
    checker = commands.add_parser("check", help="核对副本清单、部署模板和注册表一致")
    checker.add_argument("--vault", help="库根目录；给了就再核对登记的目录在库里是否存在")
    syncer = commands.add_parser("sync", help="照注册表重写副本清单和部署模板")
    syncer.add_argument("--dry-run", action="store_true", help="只列出会改哪些文件，不写")
    proposer = commands.add_parser("desired", help="对着注册表生成一份能力集提案（只打印，不提交）")
    proposer.add_argument("--capability", required=True, help="现行能力集的 JSON 文件")
    proposer.add_argument("--grant", action="append", default=[], help="要加进读取白名单的路径，可重复")
    proposer.add_argument("--grant-all", action="store_true", help="把注册表里给 AI 看的全加上")
    args = parser.parse_args(argv)
    try:
        if args.command == "check":
            problems, notes = check(ROOT, args.vault)
            for line in notes:
                print("提示 " + line)
            for line in problems:
                print("不一致 " + line, file=sys.stderr)
            print("注册表、副本清单、部署模板一致" if not problems else f"共 {len(problems)} 处不一致", file=sys.stderr)
            return 1 if problems else 0
        if args.command == "sync":
            changed, problems = sync(ROOT, args.dry_run)
            for line in problems:
                print("读不懂 " + line, file=sys.stderr)
            if problems:
                return 1
            for name in changed:
                print(("会改 " if args.dry_run else "已改 ") + name)
            print("没有要改的" if not changed else ("预演：没有写文件" if args.dry_run else "改完后：重发 Mac 副本包、发行并重启 agent"),
                  file=sys.stderr)
            return 0
        capability = load_json(Path(args.capability).read_bytes())
        result, notes = desired(capability, load_registry(ROOT), args.grant, args.grant_all)
        for line in notes:
            print(line, file=sys.stderr)
        if result is not None:
            print(json.dumps(result, ensure_ascii=False, indent=2))
            print("以上是提案正文：经策略平面的提案入口提交，由人批准后才生效", file=sys.stderr)
        return 0
    except (Invalid, OSError, ValueError) as error:
        print(f"拒绝：{error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
