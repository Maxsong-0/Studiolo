#!/usr/bin/env python3
"""上线前的备份只读核查：现存数据里有没有会让每一次整批封存都失败的东西（T-207，接 T-204 拍板 4）。

用法（在存放数据的那台机器上跑；以主服务账号跑最准，属主按运行账号比）：
    python3 -I scripts/check-backup-preflight.py --vault <Vault 根> --data <写者数据根> --control <control.sqlite3>
三项至少给一项。从 Mac 经 ssh 跑，脚本走标准输入，不用先拷过去：
    ssh <机器> python3 -I - --vault <…> --data <…> --control <…> < scripts/check-backup-preflight.py
加 --json 输出机器可读的结果；--uid 换一个比对属主的账号号码（默认是运行账号）。

只读：只列目录、取文件元数据、读清单和暂存正文（各有大小上限）；控制库按只读打开，不建 -wal、-shm。
规则照抄封存代码，每条检查旁边写了出处。代码改了规则，这里和 scripts/test-check-backup-preflight.py
的对照用例要一起改；SQL 和几个名字常量由那份用例逐字比对 Rust 源码，漂了会红。

退出码：0 没有会挡封存的问题；1 有（逐条列出）；2 用法错，或者有一部分没核成（给的路径读不了、账号不对、
中途出了没预料到的错）。没核成的部分逐条写明原因，别的部分照样核完、查出的问题照样列出；退出码 2 的结论
不完整，不能当成「没问题」。
"""
import argparse
from dataclasses import asdict, dataclass, field
import functools
import json
import os
from pathlib import Path
import sqlite3
import stat
import sys
import unicodedata
import urllib.parse

# crates/vault-broker/src/fs/rooted.rs：原子写的临时文件 `.<原名>.studiolo-tmp-<进程号>-<序号>`。
TEMPORARY_MARKER = ".studiolo-tmp-"
# crates/vault-broker/src/residue.rs：开库清扫的隔离区；封存跳过它（只在这两个位置、只跳过真目录）。
QUARANTINE = "residue-quarantine"
WORKBENCH = ".workbench"
# crates/vault-broker/src/artifact_stage.rs：暂存正文 `artifact-candidate-<id>.json`，只放在写者数据根这一层。
STAGED_PREFIX = "artifact-candidate-"
STAGED_SUFFIX = ".json"
# crates/vault-broker/src/trusted_export.rs：尝试目录、清单名、各自的大小上限、导出扫描的文件数上限。
ATTEMPTS = "sources/attempts/"
MANIFEST_NAME = "manifest.json"
MANIFEST_LIMIT = 1024 * 1024
STAGED_LIMIT = 9 * 1024 * 1024
EXPORT_MAX_FILES = 100_000
# 导出扫描从导出根算层数（`canonical/` 是第 1 层），超过 64 层拒绝；所以 Vault 里目录最多套 63 层。
MAX_DIRECTORY_NESTING = 63
# crates/vault-broker/src/checkpoint/tree.rs：目录项数和字节数上限。`check_pair` 把 Vault 根和写者数据根
# 两棵树合起来比，所以这里也合起来比；走查数到上限就停（封存同样走到上限就拒绝）。
TREE_MAX_NODES = 1_000_000
TREE_MAX_BYTES = 512 * 1024 * 1024 * 1024
# 隔离区里的文件只数给人看（封存不带它们），数到这么多就停，写「至少」。
QUARANTINE_COUNT_LIMIT = 100_000
# crates/core/src/vault_path.rs `is_unicode_combining_mark`：路径里不许出现的组合字符。
COMBINING_MARKS = ((0x0300, 0x036F), (0x1AB0, 0x1AFF), (0x1DC0, 0x1DFF), (0x20D0, 0x20FF), (0xFE20, 0xFE2F))
# 每类问题最多列这么多条，其余只给总数。
LIST_LIMIT = 50

# crates/control-db/src/checkpoint/validate.rs `snapshot`：每条计数都必须是 0，否则控制库检查点拒绝导出。
# 顺序和原文一致；（类别, 说明, SQL）。
CONTROL_ZERO_CHECKS = (
    ("control_binding", "Vault 绑定表不止一行",
     "SELECT count(*) FROM vault_binding WHERE singleton!=1"),
    ("control_snapshot_vault", "有快照属于别的 Vault",
     "SELECT count(*) FROM snapshots WHERE vault_uuid!=(SELECT vault_uuid FROM vault_binding WHERE singleton=1)"),
    ("control_projection_outbox", "投影发件箱有没投递的或坏的行（没投递的可能是瞬时的，过一分钟再跑一次）",
     "SELECT count(*) FROM projection_outbox WHERE delivered_at_ms IS NULL OR sequence<=0 OR delivered_at_ms<0"),
    ("control_terminal_outbox", "终态审计发件箱有没投递的或坏的行（没投递的可能是瞬时的，过一分钟再跑一次）",
     "SELECT count(*) FROM terminal_audit_outbox WHERE delivered_at_ms IS NULL OR delivered_at_ms<0"),
    ("control_runtime_outbox", "运行审计发件箱有没确认的或坏的行（没确认的可能是瞬时的，过一分钟再跑一次）",
     "SELECT count(*) FROM runtime_audit_outbox WHERE delivered!=1 OR sequence<=0"),
    ("control_committing", "停在「提交中」的 attempt（要主人在驾驶舱丢掉过期提交，或等它提交完）",
     "SELECT count(*) FROM attempts WHERE phase='committing'"),
    ("control_attempt_shape", "attempt 的阶段、fence 或时间不合法",
     "SELECT count(*) FROM attempts WHERE phase NOT IN ('claimed','running','cancel_requested','committing','completed','failed','cancelled','abandoned') OR fence<=0 OR updated_at_ms<created_at_ms"),
    ("control_task_shape", "任务的状态或时间不合法",
     "SELECT count(*) FROM tasks WHERE status NOT IN ('draft','queued','claimed','running','committing','cancel_requested','needs_reconfirm','needs_review','needs_revision','approved','succeeded','failed','cancelled','abandoned') OR updated_at_ms<created_at_ms"),
    ("control_active_dangling", "任务指着的在跑 attempt 不存在或已经不在跑",
     "SELECT count(*) FROM tasks t WHERE active_attempt IS NOT NULL AND NOT EXISTS(SELECT 1 FROM attempts a WHERE a.attempt_id=t.active_attempt AND a.task_id=t.task_id AND a.phase IN ('claimed','running','cancel_requested'))"),
    ("control_active_orphan", "在跑的 attempt 没有被它的任务指着",
     "SELECT count(*) FROM attempts a WHERE a.phase IN ('claimed','running','cancel_requested') AND NOT EXISTS(SELECT 1 FROM tasks t WHERE t.task_id=a.task_id AND t.active_attempt=a.attempt_id)"),
    ("control_last_dangling", "任务指着的上一次 attempt 不存在",
     "SELECT count(*) FROM tasks t WHERE last_attempt IS NOT NULL AND NOT EXISTS(SELECT 1 FROM attempts a WHERE a.attempt_id=t.last_attempt AND a.task_id=t.task_id)"),
    ("control_commit_unaudited", "产物提交没有对应的终态审计",
     "SELECT count(*) FROM artifact_commits c WHERE NOT EXISTS(SELECT 1 FROM terminal_audit_outbox o WHERE o.attempt_id=c.attempt_id)"),
    ("control_terminal_shape", "旧格式的终态：旧内联预留（产物是数组）或报告格式不是第 3 版",
     "SELECT count(*) FROM attempts WHERE terminal_json IS NOT NULL AND (NOT json_valid(terminal_json) OR json_type(terminal_json,'$.artifacts_json') IS NOT 'object' OR json_extract(terminal_json,'$.report_json.format') IS NOT 3)"),
)
# crates/control-db/src/artifact_policies.rs `validate`：批准过的产物审计都要有对应的原批准。
CONTROL_POLICY_ORPHAN = (
    "control_policy_orphan", "有「产物已批准」的运行审计找不到原批准",
    "SELECT count(*) FROM runtime_audit_outbox o WHERE o.action='task.artifacts.authorized' AND NOT EXISTS(SELECT 1 FROM artifact_policies p WHERE p.approval_event_id=o.event_id)",
)
# validate.rs：fence 计数器要恰好一行，而且比所有 attempt 的 fence 都大。
CONTROL_FENCE = (
    "control_fence", "fence 计数器不比已有 attempt 的 fence 大",
    "SELECT count(*) FROM fence_counter WHERE singleton=1 AND next_fence>COALESCE((SELECT max(fence) FROM attempts),0)",
)
# 把计数语句改成列出是哪几行；只认这几种开头，认不出的只给总数。
LISTING_PREFIXES = (
    ("SELECT count(*) FROM attempts a WHERE", "SELECT a.task_id,a.attempt_id FROM attempts a WHERE"),
    ("SELECT count(*) FROM attempts WHERE", "SELECT task_id,attempt_id FROM attempts WHERE"),
    ("SELECT count(*) FROM tasks t WHERE", "SELECT t.task_id FROM tasks t WHERE"),
    ("SELECT count(*) FROM tasks WHERE", "SELECT task_id FROM tasks WHERE"),
    ("SELECT count(*) FROM artifact_commits c WHERE", "SELECT c.attempt_id FROM artifact_commits c WHERE"),
)


class InputError(Exception):
    """这一部分不能核查（路径不存在、不是目录、读不了，账号不对）：记成没核成，退出码 2。"""


@dataclass
class Finding:
    kind: str
    where: str
    detail: str


@dataclass
class Report:
    uid: int
    checked: list = field(default_factory=list)
    findings: list = field(default_factory=list)
    notes: list = field(default_factory=list)
    # 没核成的部分和原因：别的部分照样核完、照样输出，退出码 2（复审 R2-F5）。
    incomplete: list = field(default_factory=list)

    def add(self, kind, where, detail):
        self.findings.append(Finding(kind, where, detail))


# ---------- 纯判断：名字和路径 ----------

def is_temporary_name(name):
    """rooted.rs `is_temporary_name`：宽的判断，封存见到就拒绝。"""
    return name.startswith(".") and TEMPORARY_MARKER in name


def is_exact_temporary_name(name):
    """rooted.rs `is_exact_temporary_name`：开库清扫只挪逐段对得上的。"""
    if not name.startswith("."):
        return False
    original, marker, tail = name[1:].rpartition(TEMPORARY_MARKER)
    if not marker or not original:
        return False
    process, dash, sequence = tail.partition("-")

    def decimal(value):
        return 0 < len(value) <= 20 and all("0" <= c <= "9" for c in value)

    return bool(dash) and decimal(process) and decimal(sequence)


def vault_path_problem(raw):
    """core/src/vault_path.rs `VaultPath::parse`；合法返回 None，否则返回原因。"""
    if not raw:
        return "路径为空"
    lower = raw.lower()
    if ("\\" in raw
            or any(unicodedata.category(c) == "Cc" for c in raw)
            or any(lo <= ord(c) <= hi for c in raw for lo, hi in COMBINING_MARKS)
            or not unicodedata.is_normalized("NFC", raw)
            or (len(raw) >= 3 and raw[0].isascii() and raw[0].isalpha() and raw[1:3] == ":/")
            or any(code in lower for code in ("%2f", "%5c", "%00", "%2e"))):
        return "含反斜杠、控制字符、组合字符、非 NFC 拼写、盘符或编码过的分隔符"
    if raw.startswith("/") or any(part in ("", ".", "..") for part in raw.split("/")):
        return "不是规整的相对路径"
    return None


def attempt_manifest_directory(logical):
    """trusted_export.rs `attempt_manifest_directory`：清单只在「任务/尝试」这一层。"""
    if not logical.startswith(ATTEMPTS) or not logical.endswith("/" + MANIFEST_NAME):
        return None
    directory = logical[: -len(MANIFEST_NAME)]
    parts = directory[len(ATTEMPTS):].split("/")
    if len(parts) == 3 and parts[0] and parts[1] and parts[2] == "":
        return directory
    return None


def attempt_directory_of(logical):
    """尝试目录里的文件属于哪个 `sources/attempts/<任务>/<尝试>/`；不在这一层以下返回 None。"""
    parts = logical[len(ATTEMPTS):].split("/")
    if len(parts) >= 3 and parts[0] and parts[1]:
        return f"{ATTEMPTS}{parts[0]}/{parts[1]}/"
    return None


def manifest_claims(directory, document):
    """trusted_export.rs `attempt_approvals` 对一份清单的要求；返回（认领的逻辑路径, 问题）。"""
    if not isinstance(document, dict) or not isinstance(document.get("format_version"), int) \
            or isinstance(document.get("format_version"), bool):
        return None, ("manifest_unreadable", "清单读不出来")
    artifacts = document.get("artifacts", [])
    provenance = document.get("provenance")
    if not isinstance(artifacts, list) or (provenance is not None and not isinstance(provenance, dict)):
        return None, ("manifest_unreadable", "清单读不出来")
    paths = []
    for item in artifacts:
        path = item.get("path") if isinstance(item, dict) else None
        if not isinstance(path, str) or vault_path_problem(path):
            return None, ("manifest_unreadable", "清单里有不合法的产物路径")
        paths.append(path)
    if document["format_version"] != 2 or provenance is None:
        return None, ("old_manifest", "旧清单（不是第 2 版或没有原批准），每一次整批封存都会因它失败")
    claims = [directory + MANIFEST_NAME]
    for path in paths:
        if not path.startswith(directory):
            return None, ("manifest_outside", f"清单列了本目录以外的文件：{path}")
        claims.append(path)
    if len(set(claims)) != len(claims):
        return None, ("double_claim", "同一个文件被清单认领了两次")
    return claims, None


def staged_problem(name, document):
    """artifact_stage.rs `staged_provenance` 里能在这一侧核的部分：名字形状和有没有原批准。"""
    stem = name[len(STAGED_PREFIX):]
    if not stem.endswith(STAGED_SUFFIX) or not stem[: -len(STAGED_SUFFIX)] \
            or any(c not in "0123456789abcdef" for c in stem[: -len(STAGED_SUFFIX)]):
        return "staged_unreadable", "名字不是 artifact-candidate-<十六进制>.json"
    if not isinstance(document, dict):
        return "staged_unreadable", "内容读不出来"
    if document.get("provenance") is None:
        return "staged_legacy", "暂存正文没有原批准（升级前留下的），每一次整批封存都会因它失败"
    return None


# ---------- 读文件系统（只读） ----------

def read_limited(path, limit):
    """不跟随符号链接，按上限读；超过上限、不是普通文件、打不开或读的时候出错（如磁盘 I/O 错）都返回 None，
    调用方报「读不出来」：封存同样读不了它，而且一份读不出的文件不能让别的文件没核（2026-11-16 第二意见）。"""
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_CLOEXEC", 0)
    try:
        fd = os.open(path, flags)
    except OSError:
        return None
    try:
        with os.fdopen(fd, "rb") as handle:
            info = os.fstat(handle.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
                return None
            data = handle.read(limit + 1)
    except OSError:
        return None
    return data if len(data) <= limit else None


def decode_json(data):
    """读不出来一律返回 None：不是 UTF-8、不是 JSON、整数位数超过解释器上限（ValueError）、嵌套太深（RecursionError）。"""
    if data is None:
        return None
    try:
        return json.loads(data.decode("utf-8"))
    except (ValueError, RecursionError):
        return None


def root_metadata(path, label):
    try:
        info = os.lstat(path)
    except OSError as error:
        raise InputError(f"{label} 读不了：{path}（{error.strerror}）") from None
    if not stat.S_ISDIR(info.st_mode):
        raise InputError(f"{label} 不是真目录（符号链接也不行）：{path}")
    return info


@dataclass
class Tree:
    """一棵树的走查结果：要细看的普通文件（逻辑路径到绝对路径，只留 keep 选中的），加上计数。"""
    root: str
    files: dict = field(default_factory=dict)
    file_count: int = 0
    nodes: int = 0
    total_bytes: int = 0
    # 目录项数走到上限就停了：计数是「至少」，后面的没核。
    stopped: bool = False
    # 列出来之后、取元数据之前就不见了的（主服务在写）；不算问题，只提一句。
    vanished: int = 0


# 残留临时文件的三种处理说法。
RESIDUE_SWEPT = "原子写崩溃留下的临时文件；主服务下次开库会把它挪进隔离区（重启主服务即可）"
RESIDUE_NOT_SWEPT = "原子写崩溃留下的临时文件，开库清扫不管这个位置；要人看过再删"
RESIDUE_UNSAFE_QUARANTINE = ("原子写崩溃留下的临时文件；隔离区目录不是本账号私有的（属主不对，或组、其他人有权限），"
                             "开库清扫不会往里挪，要人看过再处理")


def quarantine_usable(path, uid):
    """residue.rs `ensure_directory`：隔离区还没有（开库时新建），或者是本账号私有的真目录，开库清扫才会往里挪。"""
    try:
        info = os.lstat(path)
    except FileNotFoundError:
        return True
    except OSError:
        return False
    return stat.S_ISDIR(info.st_mode) and info.st_uid == uid and not info.st_mode & 0o077


def count_quarantined(path, limit):
    """隔离区里有多少个目录项（文件和目录都算）：封存不带它们，只给人看。返回（个数, 是不是「至少」）。
    数到上限就停，每层也只读到上限为止：空目录再多也不会整棵走完（复审 R2-F4）。不跟随链接；
    里面有读不了的目录也算「至少」，不静默少数。"""
    count = 0
    at_least = False
    pending = [path]
    while pending:
        directory = pending.pop()
        try:
            entries = listed_entries(directory, limit - count)
        except OSError:
            at_least = True
            continue
        for entry in entries:
            count += 1
            if count >= limit:
                return count, True
            try:
                if entry.is_dir(follow_symlinks=False):
                    pending.append(entry.path)
            except OSError:
                at_least = True
    return count, at_least


def listed_entries(directory, room):
    """一层目录的目录项，按名字排序；最多取 room 个（再多就超上限了，不必全读进来）。"""
    entries = []
    with os.scandir(directory) as iterator:
        for entry in iterator:
            entries.append(entry)
            if len(entries) >= room:
                break
    entries.sort(key=lambda entry: entry.name)
    return entries


def walk_tree(root, prefix, skip, report, sweep_hint, keep, node_budget, tree=None):
    """照 tree.rs 和 trusted_export.rs 走一棵树。prefix 是导出里的顶层目录（`canonical/`、`writer-data/`），
    skip 是封存跳过的隔离区路径；sweep_hint(逻辑路径) 给开库清扫会挪走的位置返回处理说法，不管的位置返回 None；
    keep(逻辑路径, 名字) 选出后面还要细看的普通文件；node_budget 是这棵树还能有多少个目录项，超了就停。
    tree 由调用方给时就地填：后面哪一步出错没核成，已经走过的计数也留得住（2026-11-16 第二意见）。"""
    root_info = root_metadata(root, prefix.rstrip("/"))
    tree = Tree(root=str(root)) if tree is None else tree
    if root_info.st_uid != report.uid:
        report.add("foreign_owner", str(root), f"根目录属主是 {root_info.st_uid}，不是 {report.uid}")
    elif missing_access(root_info):
        report.add("unreadable", str(root), f"属主没有{missing_access(root_info)}权限，封存读不了，下面的没核")
        return tree
    pending = [(Path(root), "", tuple(skip), 0)]
    while pending and not tree.stopped:
        directory, relative, skip_here, nesting = pending.pop()
        if nesting > MAX_DIRECTORY_NESTING:
            report.add("too_deep", str(directory), f"目录套了 {nesting} 层，封存最多 {MAX_DIRECTORY_NESTING} 层")
            continue
        try:
            # 多取两个：一个可能是隔离区（不计数），再多一个就证明超了上限。
            entries = listed_entries(directory, node_budget - tree.nodes + 2)
        except OSError as error:
            report.add("unreadable", str(directory), f"列不出目录（{error.strerror}），封存同样读不了")
            continue
        for entry in entries:
            name = entry.name
            absolute = Path(entry.path)
            if len(skip_here) == 1 and name == skip_here[0] and entry.is_dir(follow_symlinks=False):
                count, at_least = count_quarantined(absolute, QUARANTINE_COUNT_LIMIT)
                if count:
                    report.notes.append(f"{absolute} 里有{'至少 ' if at_least else ' '}{count} 个目录项（文件和目录都算）："
                                        "封存不带它们，人看过可以删")
                continue
            tree.nodes += 1
            if tree.nodes > node_budget:
                tree.stopped = True
                break
            # tree.rs `names`：目录和文件的名字都要是 UTF-8、不含控制字符，否则整棵树封存不了。
            try:
                name.encode("utf-8")
            except UnicodeEncodeError:
                report.add("bad_name", str(absolute), "名字不是 UTF-8，封存扫描认不了")
                continue
            if any(unicodedata.category(c) == "Cc" for c in name):
                report.add("bad_name", str(absolute), "名字里有控制字符，封存扫描认不了")
                continue
            logical = relative + name
            try:
                info = entry.stat(follow_symlinks=False)
            except FileNotFoundError:
                tree.vanished += 1
                continue
            except OSError as error:
                report.add("unreadable", str(absolute), f"取不了元数据（{error.strerror}），封存同样读不了")
                continue
            if stat.S_ISDIR(info.st_mode):
                if node_problem(info, root_info, report, absolute, check_links=False):
                    continue
                below = skip_here[1:] if len(skip_here) > 1 and skip_here[0] == name else ()
                pending.append((absolute, logical + "/", below, nesting + 1))
                continue
            if not stat.S_ISREG(info.st_mode):
                report.add("special_node", str(absolute), "不是普通文件也不是目录（符号链接、设备、套接字或管道），封存只收这两种")
                continue
            if node_problem(info, root_info, report, absolute, check_links=True):
                continue
            problem = vault_path_problem(prefix + logical)
            if problem:
                report.add("bad_name", str(absolute), f"路径不合法：{problem}")
            if is_temporary_name(name):
                hint = sweep_hint(logical) if is_exact_temporary_name(name) else None
                report.add("residue", str(absolute), hint or RESIDUE_NOT_SWEPT)
                continue
            tree.file_count += 1
            tree.total_bytes += info.st_size
            if keep(logical, name):
                tree.files[logical] = absolute
    if tree.vanished:
        report.notes.append(f"{root} 里有 {tree.vanished} 个目录项在扫描时不见了（主服务在写），结论按扫描那一刻")
    return tree


def missing_access(info):
    """files.rs 以只读打开每个文件，tree.rs 要列目录、进目录：属主缺的那几种权限，不缺返回空串。
    按权限位判断，不看是谁在跑脚本（root 跑也一样准）。"""
    if stat.S_ISDIR(info.st_mode):
        needed = ((stat.S_IRUSR, "读"), (stat.S_IXUSR, "进入"))
    else:
        needed = ((stat.S_IRUSR, "读"),)
    return "和".join(label for bit, label in needed if not info.st_mode & bit)


def node_problem(info, root_info, report, absolute, check_links):
    """tree.rs、files.rs：每个目录和文件都要和根同一个文件系统、属于运行账号、属主读得了，文件只许一个硬链接。"""
    if info.st_dev != root_info.st_dev:
        report.add("foreign_device", str(absolute), "不在根目录那个文件系统上（挂载点），封存不跨文件系统")
        return True
    if info.st_uid != report.uid:
        report.add("foreign_owner", str(absolute), f"属主是 {info.st_uid}，不是 {report.uid}")
        return True
    if missing_access(info):
        report.add("unreadable", str(absolute),
                   f"属主没有{missing_access(info)}权限（{stat.filemode(info.st_mode)}），封存读不了")
        return True
    if check_links and info.st_nlink != 1:
        report.add("hard_link", str(absolute), f"有 {info.st_nlink} 个硬链接，封存只收单链接的文件")
        return True
    return False


def check_vault(vault, report, node_budget=None, tree=None):
    """Vault 根：结构、临时文件、尝试目录的清单。开库清扫只管 `.workbench/` 底下。tree 见 `walk_tree`。"""
    report.checked.append(f"Vault 根 {vault}")
    usable = quarantine_usable(vault / WORKBENCH / QUARANTINE, report.uid)

    def sweep_hint(logical):
        if not logical.startswith(WORKBENCH + "/"):
            return None
        return RESIDUE_SWEPT if usable else RESIDUE_UNSAFE_QUARANTINE

    tree = walk_tree(vault, "canonical/", (WORKBENCH, QUARANTINE), report, sweep_hint,
                     lambda logical, name: logical.startswith(ATTEMPTS),
                     TREE_MAX_NODES if node_budget is None else node_budget, tree)
    check_attempts(tree.files, report)
    return tree


def check_attempts(files, report):
    """trusted_export.rs `attempt_approvals` + `classify`：尝试目录里的文件只认第 2 版清单列出的。"""
    claimed = set()
    broken = set()
    for logical in sorted(files):
        directory = attempt_manifest_directory(logical)
        if directory is None:
            continue
        document = decode_json(read_limited(files[logical], MANIFEST_LIMIT))
        claims, problem = (None, ("manifest_unreadable", "清单读不出来或超过 1 MiB")) if document is None \
            else manifest_claims(directory, document)
        if problem:
            report.add(problem[0], str(files[logical]), problem[1])
            broken.add(directory)
            continue
        duplicate = claimed.intersection(claims)
        if duplicate:
            report.add("double_claim", str(files[logical]), f"和别的清单认领了同一个文件：{sorted(duplicate)[0]}")
        claimed.update(claims)
    missing = {}
    for logical in sorted(files):
        if not logical.startswith(ATTEMPTS) or logical in claimed:
            continue
        directory = attempt_directory_of(logical)
        if directory in broken:
            continue
        if directory is not None and directory + MANIFEST_NAME not in files:
            missing.setdefault(directory, 0)
            missing[directory] += 1
            continue
        report.add("unlisted_attempt_file", str(files[logical]), "尝试目录里没被清单列出的文件，封存归不了类")
    for directory, count in sorted(missing.items()):
        report.add("missing_manifest", directory, f"尝试目录没有清单（里面 {count} 个文件都归不了类）")


def check_data(data, report, node_budget=None, tree=None):
    """写者数据根：结构、临时文件、暂存正文。开库清扫管整个数据根。tree 见 `walk_tree`。"""
    report.checked.append(f"写者数据根 {data}")
    usable = quarantine_usable(data / QUARANTINE, report.uid)
    tree = walk_tree(data, "writer-data/", (QUARANTINE,), report,
                     lambda logical: RESIDUE_SWEPT if usable else RESIDUE_UNSAFE_QUARANTINE,
                     lambda logical, name: name.startswith(STAGED_PREFIX),
                     TREE_MAX_NODES if node_budget is None else node_budget, tree)
    for logical, absolute in sorted(tree.files.items()):
        name = logical.rsplit("/", 1)[-1]
        if name != logical:
            report.add("staged_misplaced", str(absolute), "名字像暂存正文，却不在写者数据根这一层")
            continue
        document = decode_json(read_limited(absolute, STAGED_LIMIT))
        problem = staged_problem(name, document) if document is not None \
            else ("staged_unreadable", "内容读不出来或超过 9 MiB")
        if problem:
            report.add(problem[0], str(absolute), problem[1])
    return tree


# ---------- 控制库（只读） ----------

def sidecar_size(path):
    """控制库旁文件（-wal、-shm）的大小，不跟随链接；不存在返回 None。"""
    try:
        return os.lstat(path).st_size
    except FileNotFoundError:
        return None
    except OSError as error:
        raise InputError(f"控制库旁文件读不了：{path}（{error.strerror}）") from None


def open_control_readonly(path):
    """只读打开，保证不建 -wal、-shm：主服务开着时 -shm 在，按只读模式共享它；
    关着时两者都不在，按不可变方式读主文件。只有 -wal 没有 -shm 是异常状态，不碰。

    -shm 在的时候只许控制库的属主账号跑：判断完到连上之间主服务可能正好关库、收掉 -wal 和 -shm，
    只读的 WAL 连接这时会以当前账号新建 -shm；账号不对，主服务之后就打不开库了（复审 A3-F7）。
    属主账号建出来的 -shm 和主服务自己建的一样，无害。"""
    try:
        info = os.lstat(path)
    except OSError as error:
        raise InputError(f"控制库读不了：{path}（{error.strerror}）") from None
    if not stat.S_ISREG(info.st_mode):
        raise InputError(f"控制库不是普通文件（符号链接也不行）：{path}")
    wal = Path(f"{path}-wal")
    shm = Path(f"{path}-shm")
    wal_size = sidecar_size(wal)
    if sidecar_size(shm) is not None:
        if info.st_uid != os.geteuid():
            raise InputError(f"主服务开着（{shm.name} 在）时只能用控制库的属主账号（{info.st_uid}）核查："
                             "换别的账号，它关库的那一刻可能以这个账号建出 -shm，之后主服务打不开库")
        mode = "mode=ro"
    elif wal_size:
        raise InputError(f"控制库有没合并的 {wal.name} 却没有 {shm.name}：先让主服务正常开一次库再核查")
    else:
        mode = "mode=ro&immutable=1"
    uri = f"file:{urllib.parse.quote(str(Path(path).resolve()))}?{mode}"
    try:
        connection = sqlite3.connect(uri, uri=True, timeout=5)
        connection.execute("PRAGMA query_only=ON")
        connection.execute("SELECT count(*) FROM sqlite_master").fetchone()
    except sqlite3.Error as error:
        raise InputError(f"控制库打不开：{path}（{error}）") from None
    return connection


def listing_query(count_query):
    for prefix, replacement in LISTING_PREFIXES:
        if count_query.startswith(prefix):
            return replacement + count_query[len(prefix):] + f" LIMIT {LIST_LIMIT}"
    return None


def check_control(path, report):
    report.checked.append(f"控制库 {path}")
    connection = open_control_readonly(path)
    try:
        connection.execute("BEGIN")
        for kind, detail, query in (*CONTROL_ZERO_CHECKS, CONTROL_POLICY_ORPHAN):
            count = connection.execute(query).fetchone()[0]
            if count == 0:
                continue
            listed = listing_query(query)
            rows = connection.execute(listed).fetchall() if listed else []
            where = "；".join("/".join(str(value) for value in row) for row in rows) or "（只有总数）"
            report.add(kind, where, f"{detail}：{count} 条")
        kind, detail, query = CONTROL_FENCE
        if connection.execute(query).fetchone()[0] != 1:
            report.add(kind, str(path), detail)
        integrity = connection.execute("PRAGMA quick_check(1)").fetchone()[0]
        if integrity != "ok":
            report.add("control_integrity", str(path), f"库自检不通过：{integrity}")
        if connection.execute("PRAGMA foreign_key_check").fetchone() is not None:
            report.add("control_integrity", str(path), "有外键指向不存在的行")
        version = connection.execute("SELECT max(version) FROM schema_migrations").fetchone()[0]
        report.notes.append(f"控制库结构版本 {version}（主服务开库时迁移，这里不判断）")
        connection.execute("ROLLBACK")
    except sqlite3.Error as error:
        raise InputError(f"控制库查不了：{path}（{error}）") from None
    finally:
        connection.close()


# ---------- 汇总与输出 ----------

def check_totals(trees, report, missing=0):
    """tree.rs `check_pair`：Vault 根和写者数据根两棵树的目录项数、字节数合起来比上限（复审 A3-F1）；
    trusted_export.rs `walk`：导出扫描插入前文件数已经到上限才拒绝，所以超过上限才算（复审 A3-F3）。
    missing 是给了却没核成的树有几棵：它们走过的那部分也在 trees 里、照样算进合计，超了照样报（实际只会更多），
    没超不能说没超。"""
    if missing:
        report.notes.append(f"有 {missing} 棵树没核成：两棵树合计的目录项数、字节数、文件数只算了走到的部分，"
                            "实际只会更多")
    if not trees:
        return
    where = " + ".join(tree.root for tree in trees)
    stopped = any(tree.stopped for tree in trees)
    at_least = "至少 " if stopped or missing else ""
    nodes = sum(tree.nodes for tree in trees)
    total_bytes = sum(tree.total_bytes for tree in trees)
    files = sum(tree.file_count for tree in trees)
    if nodes > TREE_MAX_NODES:
        report.add("too_many_nodes", where,
                   f"{at_least}{nodes} 个目录项，封存两棵树合计最多 {TREE_MAX_NODES} 个"
                   + ("（数到上限就停了，后面的没核）" if stopped else ""))
    if total_bytes > TREE_MAX_BYTES:
        report.add("too_many_bytes", where, f"{at_least}{total_bytes} 字节，封存两棵树合计最多 {TREE_MAX_BYTES} 字节")
    if files > EXPORT_MAX_FILES:
        report.add("too_many_files", where,
                   f"{at_least}{files} 个文件，导出扫描整份最多 {EXPORT_MAX_FILES} 个（这里只数了这两棵树）")
    elif files >= EXPORT_MAX_FILES * 9 // 10:
        report.notes.append(f"{where} 共 {files} 个文件，离导出扫描上限 {EXPORT_MAX_FILES} 不远；上限按整份导出算，"
                            "控制库、审计这些 owner 的文件这里没数，可能已经超了")
    if len(trees) == 1 and not missing:
        report.notes.append("只给了一棵树：封存按 Vault 根和写者数据根两棵合计比目录项数和字节数，另一棵也要算进去")


def attempt(report, label, check):
    """跑一部分核查。核不成（InputError，或者没预料到的错）就记下是哪部分、为什么，返回 None：别的部分照样核，
    已经查出的问题照样输出（复审 R2-F5）；没预料到的错不能以退出码 1 被读成「有问题」（复审 A3-F2）。"""
    try:
        return check()
    except InputError as error:
        report.incomplete.append(f"{label}：{error}")
    except Exception as error:  # noqa: BLE001 —— 见上面的说明
        report.incomplete.append(f"{label}：意外错误 {type(error).__name__}：{error}")
    return None


def run(arguments):
    report = Report(uid=arguments.uid if arguments.uid is not None else os.geteuid())
    sections = []
    if arguments.vault:
        sections.append((f"Vault 根 {arguments.vault}", Path(arguments.vault), check_vault))
    if arguments.data:
        sections.append((f"写者数据根 {arguments.data}", Path(arguments.data), check_data))
    # 两棵树共用一份目录项预算：前一棵走过的，后一棵就少走这么多。计数对象在这里建、交给检查就地填，
    # 一棵树走完之后哪一步出错没核成，走过的部分也照样占预算、照样进合计（2026-11-16 第二意见）。
    trees = []
    missing = 0
    for label, root, check in sections:
        budget = max(TREE_MAX_NODES - sum(tree.nodes for tree in trees), 0)
        tree = Tree(root=str(root))
        if attempt(report, label, functools.partial(check, root, report, budget, tree)) is None:
            missing += 1
        trees.append(tree)
    check_totals(trees, report, missing)
    if arguments.control:
        attempt(report, f"控制库 {arguments.control}", lambda: check_control(arguments.control, report))
    return report


def render(report):
    lines = [f"核查了：{'、'.join(report.checked)}；属主按 {report.uid} 比。"]
    if report.findings:
        lines.append(f"有 {len(report.findings)} 处会让每一次整批封存都失败，先别启用备份，报告再定：")
        by_kind = {}
        for finding in report.findings:
            by_kind.setdefault(finding.kind, []).append(finding)
        for kind, findings in by_kind.items():
            lines.append(f"- {kind}（{len(findings)} 处）")
            for finding in findings[:LIST_LIMIT]:
                lines.append(f"    {finding.where}：{finding.detail}")
            if len(findings) > LIST_LIMIT:
                lines.append(f"    ……另有 {len(findings) - LIST_LIMIT} 处没列")
    elif report.incomplete:
        lines.append("核成的部分没发现会挡封存的现存数据；下面几部分没核成，结论不完整。")
    else:
        lines.append("没发现会挡封存的现存数据。")
    for item in report.incomplete:
        lines.append(f"没核成：{item}")
    for note in report.notes:
        lines.append(f"留意：{note}")
    lines.append("没核的：历史目录对历史正文的认领、暂存正文的内容和名字是否对得上、清单里原批准的内部字段、"
                 "控制库的表结构清单和产物批准的逐条核对、ACL 和安全模块对读权限的限制（这里只看权限位）、"
                 "导出里别的 owner（控制库、审计、认证……）的文件数；这些由封存本身把关，第一次封存失败会写明原因。")
    return "\n".join(lines)


def parse_arguments(argv):
    parser = argparse.ArgumentParser(description="上线前的备份只读核查（不写任何东西）")
    parser.add_argument("--vault", help="Vault 根（Broker 的 canonical 根）")
    parser.add_argument("--data", help="写者数据根（Broker 的 writer-data 根）")
    parser.add_argument("--control", help="控制库文件 control.sqlite3")
    parser.add_argument("--uid", type=int, help="比对属主用的账号号码，默认是运行账号")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    arguments = parser.parse_args(argv)
    if not (arguments.vault or arguments.data or arguments.control):
        parser.error("--vault、--data、--control 至少给一项")
    return arguments


def main(argv=None):
    arguments = parse_arguments(sys.argv[1:] if argv is None else argv)
    try:
        report = run(arguments)
        output = json.dumps({"uid": report.uid, "checked": report.checked,
                             "findings": [asdict(finding) for finding in report.findings],
                             "notes": report.notes, "incomplete": report.incomplete},
                            ensure_ascii=False, indent=1) if arguments.json else render(report)
    except Exception as error:  # noqa: BLE001 —— 汇总、输出出错时什么结论都没有，同样不能以退出码 1 被读成「有问题」
        print(f"核查没做完：意外错误 {type(error).__name__}：{error}", file=sys.stderr)
        return 2
    print(output)
    for item in report.incomplete:
        print(f"核查没做完：{item}", file=sys.stderr)
    if report.incomplete:
        return 2
    return 1 if report.findings else 0


if __name__ == "__main__":
    sys.exit(main())
