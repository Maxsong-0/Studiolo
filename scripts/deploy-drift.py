#!/usr/bin/env python3
"""部署漂移对账（T-256）：哪些提交合了 main、线上没有；哪些 BUG 的修复已经上线、该验收了。

三个子命令：
  command  打印要用户敲的那条只读命令（在 169 上取发行记录，不带 sudo）
  probe    打印在 169 上跑的只读程序，由上面那条命令经 ssh 送过去
  report   拿记录和本机仓库的 origin/main 对账

本脚本只读：只起 git（本机仓库、只读子命令），不 ssh、不 fetch、不写任何文件。
退出码：0 没有漂移也没有要看的；1 有漂移、看不清、或有该验收／要人看的 BUG；2 输入读不懂（不出结论）。
用法、归类依据和局限见 docs/m2/deploy-drift.md。
"""
import argparse
import json
import os
import plistlib
import re
import subprocess
import sys

if sys.version_info < (3, 11):
    print("deploy-drift: 需要 Python 3.11 以上（要用 tomllib 读 Cargo.toml）", file=sys.stderr)
    sys.exit(2)

import tomllib  # noqa: E402  低版本要先给出上面那句话

# 169 上跑的只读程序：读两个单元指到的发行目录和告警包的链接，印一份 JSON。哪一项读不出来，那一项只给 error。
PROBE = r'''import json, os, re, subprocess, time

NAME = r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}"
SHARE = r"/home/maxsong/\.local/share/"
HUMAN = re.compile("(" + SHARE + "studiolo-human-releases/(" + NAME + r"))/launch-human\.py")
AGENT = re.compile("(" + SHARE + "studiolo-agent-releases/(" + NAME + r"))/bin/studiolo-agent")
ALERT = "/opt/studiolo-alert/current"
COMMIT = re.compile("[0-9a-f]{40}")
CLEAN = "与 base-commit 一致的干净工作区快照"


def read(root, path):
    with open(root + path, "rb") as handle:
        data = handle.read(4 * 1024 * 1024 + 1)
    if len(data) > 4 * 1024 * 1024:
        raise ValueError
    return data.decode("utf-8")


def commit(value):
    if not isinstance(value, str) or not COMMIT.fullmatch(value):
        raise ValueError
    return value


def human(root, show):
    found = HUMAN.search(show("studiolo-human.service"))
    manifest = json.loads(read(root, found[1] + "/manifest.json"))
    value = {"release": found[2], "build": manifest["build"]}
    for key in ("human_program_source_commit", "web_source_commit", "bootstrap_source_commit"):
        value[key] = commit(manifest[key])
    if "sync_program_source_commit" in manifest:
        value["sync_program_source_commit"] = commit(manifest["sync_program_source_commit"])
    return value


def agent(root, show):
    found = AGENT.search(show("studiolo-agent.service"))
    if CLEAN not in read(root, found[1] + "/provenance.txt"):
        raise ValueError
    return {"release": found[2], "commit": commit(read(root, found[1] + "/base-commit.txt").strip())}


def alert(root, show):
    return {"commit": os.path.basename(os.readlink(root + ALERT))}


def collect(root, show, now):
    value = {"schema": 1, "collected_at": now}
    for key, part, reason in (
            ("human", human, "人侧单元没指到发行目录，或者读不了它的 manifest"),
            ("agent", agent, "agent 单元没指到发行目录、读不了来源提交，或者包出自有未提交改动的工作区"),
            ("alert", alert, "读不到告警包的 current 链接")):
        try:
            value[key] = part(root, show)
        except (OSError, ValueError, KeyError, TypeError):
            value[key] = {"error": reason}
    return value


def show(unit):
    try:
        result = subprocess.run(["/usr/bin/systemctl", "show", "-p", "ExecStart", "--value", unit],
                                capture_output=True, text=True, timeout=20)
    except (OSError, subprocess.SubprocessError):
        return ""
    return result.stdout


if __name__ == "__main__":
    print(json.dumps(collect("", show, time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())), indent=1))
'''

SCHEMA = 1
DEFAULT_HOST = "studiolo-169"
RECORD_PATH = "/tmp/deploy-drift-169.json"
LAUNCH_AGENT = "~/Library/LaunchAgents/me.studiolo.agent-sync.plist"
VAULT = "~/Documents/Max's Secret Place"
BUGS = "项目/Studiolo-Docs/06-BUG"
MAX_RECORD_BYTES = 64 * 1024
MAX_NOTE_BYTES = 2 * 1024 * 1024
MAX_RANGE = 500

FULL = re.compile("[0-9a-f]{40}")
SHORT = re.compile("[0-9a-f]{7,40}")
NAME = re.compile("[A-Za-z0-9][A-Za-z0-9._-]{0,99}")
REF = re.compile("[A-Za-z0-9][A-Za-z0-9._/-]{0,199}")
HOST = re.compile("[A-Za-z0-9][A-Za-z0-9._@-]{0,199}")
MOMENT = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z")

# 整个工作区共用的 Rust 输入：动了它们，每个 Rust 程序都可能变。
RUST = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
# 编进程序、但放在包目录外面的文件（include_str!／include_bytes!）：包目录名 → 路径。用例核对一个不漏。
EMBEDDED = {"runner-adapter": ("deploy/runner/launch.py", "deploy/runner/rg_probe.py")}
TEST = re.compile(r"(^|/)(tests?|__tests__|benches|examples)/|(^|/)(test[-_][^/]*|tests?\.rs)$|(_tests?|\.test|\.spec)\.\w+$")
# 只管格式和检查的设置文件，不进任何产物。
SETTINGS = {".gitignore", ".prettierignore", ".prettierrc.json", "eslint.config.js", ".testbox.toml"}
# 不进这五个发行的路径。third 列：None 不进任何产物；"装配" 只影响怎么装；"另装" 是单独部署的东西，上没上线这里看不出。
OUTSIDE = (
    ("docs/", "文档", None),
    ("examples/", "示例库", None),
    ("scripts/", "打包、验收、测试用的脚本", None),
    ("web/dev/", "网页的开发假数据", None),
    ("crates/studiolo/", "架构守卫与测试替身", None),
    ("deploy/blackboard/", "人侧发行的装配工具", "装配"),
    ("deploy/human/", "人侧发行的装配工具", "装配"),
    ("deploy/agent/", "agent 的安装工具", "装配"),
    ("crates/backup-worker/", "备份 worker", "另装"),
    ("deploy/backup-worker/", "备份 worker", "另装"),
    ("deploy/doorbell/", "门铃", "另装"),
    ("deploy/sync-bridge/", "Bridge 的单元与配置", "另装"),
    ("deploy/runner/", "Runner 的安装", "另装"),
    ("deploy/preview/", "预览站", "另装"),
    ("deploy/replica-push/", "169 上收副本的端口", "另装"),
    ("crates/auth-admin/", "认证管理程序（随人侧发行原样沿用，manifest 不记它的来源）", "另装"),
    ("crates/vault-admin/", "一次性首建工具（来源提交冻结）", "另装"),
    ("bottega/", "Bottega（Mac App）", "另装"),
)


class Bad(Exception):
    """输入读不懂：不出结论，退 2。"""


# ---- 路径归到哪个发行 ----

def is_test(path):
    return bool(TEST.search(path))


def is_inert(path):
    """怎么改都进不了产物：测试、说明文字、格式与检查的设置。"""
    return is_test(path) or path.endswith(".md") or path.rsplit("/", 1)[-1] in SETTINGS


class Part:
    """一个发行（或发行里单独记来源的一块）由哪些路径组成。"""

    def __init__(self, title, packages=(), paths=(), without=()):
        self.title = title
        self.packages = packages
        self.paths = paths
        self.without = without
        self.crates = set()

    def owns(self, path):
        if is_inert(path) or path.startswith(tuple(self.without)):
            return False
        if self.packages and path in RUST:
            return True
        pieces = path.split("/")
        if len(pieces) > 2 and pieces[0] == "crates" and pieces[1] in self.crates:
            return True
        if any(path in EMBEDDED.get(crate, ()) for crate in self.crates):
            return True
        return any(path == own or (own.endswith(("/", "_")) and path.startswith(own)) for own in self.paths)


HUMAN, WEB, TOOLS, AGENT, MAC, ALERT, WORKER = "人侧程序", "人侧网页", "人侧随包工具", "agent", "Mac 副本包", "告警包", "Worker"


def blank_parts():
    return [
        Part(HUMAN, packages=("studiolo-local-server",)),
        Part(WEB, paths=("web/",), without=("web/dev/fixture",)),
        # studiolo-maintain 是 local-server 包里的程序，和人侧程序共用全部依赖。
        Part(TOOLS, packages=("studiolo-local-server", "studiolo-sync-launcher")),
        Part(AGENT, packages=("studiolo-agent-server",), paths=("deploy/agent/studiolo-agent-system.service.in",)),
        Part(MAC, paths=("scripts/sync-agent-vault.sh", "scripts/replica_")),
        # studiolo-alert 在 local-server 包里，但只用这两个库。
        Part(ALERT, packages=("studiolo-audit-ledger", "studiolo-observability"),
             paths=("crates/local-server/src/alert_main.rs", "deploy/alerts/", "scripts/check-agent.py",
                    "scripts/check-agent-replica.py"), without=("deploy/alerts/worker/",)),
        Part(WORKER, paths=("deploy/alerts/worker/",)),
    ]


def closure(manifests, roots):
    """这几个包（按包名）连同正式依赖和构建依赖的包目录名；开发依赖不进产物，不算。"""
    directories, depends = {}, {}
    for path, text in manifests.items():
        try:
            value = tomllib.loads(text)
            package = value["package"]["name"]
        except (tomllib.TOMLDecodeError, KeyError, TypeError) as error:
            raise Bad(f"读不懂 {path}") from error
        directories[package] = path.split("/")[1]
        tables = [value, *value.get("target", {}).values()]
        names = set()
        for table in tables:
            for kind in ("dependencies", "build-dependencies"):
                for name, spec in table.get(kind, {}).items():
                    names.add(spec.get("package", name) if isinstance(spec, dict) else name)
        depends[package] = names
    found, queue = set(), [root for root in roots if root in directories]
    while queue:
        package = queue.pop()
        if directories[package] not in found:
            found.add(directories[package])
            queue += [name for name in depends[package] if name in directories]
    return found


def parts_for(manifest_sets):
    """各发行的路径；依赖闭包取几个提交的并集（线上那份和 main 那份可能不一样）。"""
    parts = blank_parts()
    for part in parts:
        for manifests in manifest_sets:
            part.crates |= closure(manifests, part.packages)
        # 包改名、搬走或者 Cargo.toml 没读到，闭包就是空的；空闭包对不出漂移，不能当成没有漂移。
        if part.packages and not all(any(closure(one, [package]) for one in manifest_sets) for package in part.packages):
            raise Bad(f"在 crates/*/Cargo.toml 里找不到{part.title}要的包（{'、'.join(part.packages)}），先改 scripts/deploy-drift.py")
    return parts


def outside(path):
    return next(((title, kind) for prefix, title, kind in OUTSIDE if path.startswith(prefix)), None)


def home(path, parts):
    """这个路径归哪儿；归不到任何一处回 None，由报告单列，不吞。"""
    for part in parts:
        if part.owns(path):
            return part.title
    if is_inert(path):
        return "测试、说明或设置"
    found = outside(path)
    return found[0] if found else None


# ---- 本机仓库 ----

class Repo:
    """只用 git 的只读子命令；环境从头给，不继承 GIT_*，也不许它去补取缺的对象。"""

    def __init__(self, path):
        self.path = path
        self.changed = {}
        self.manifest_cache = {}

    def git(self, *args, accept=(0,)):
        environment = {"PATH": os.environ.get("PATH", os.defpath), "HOME": os.path.expanduser("~"), "LC_ALL": "C.UTF-8",
                       "GIT_OPTIONAL_LOCKS": "0", "GIT_NO_LAZY_FETCH": "1", "GIT_TERMINAL_PROMPT": "0"}
        command = ["git", "--no-pager", "-c", "core.quotepath=off", "-c", "core.fsmonitor=false",
                   "-c", "core.hooksPath=/dev/null", "-C", self.path, *args]
        try:
            result = subprocess.run(command, capture_output=True, env=environment, timeout=120, stdin=subprocess.DEVNULL)
        except (OSError, subprocess.SubprocessError) as error:
            raise Bad("git 跑不起来") from error
        if result.returncode not in accept:
            raise Bad(f"git {args[0]} 失败（退出码 {result.returncode}）")
        return result

    def text(self, *args):
        return self.git(*args).stdout.decode("utf-8", "replace")

    def names(self, *args):
        return [name.decode("utf-8", "surrogateescape") for name in self.git(*args, "-z").stdout.split(b"\0") if name]

    def find(self, revision):
        """全长提交号；没有这个提交（或者短号有歧义）回 None。"""
        result = self.git("rev-parse", "--verify", "--quiet", "--end-of-options", revision + "^{commit}", accept=(0, 1, 128))
        value = result.stdout.decode("ascii", "replace").strip()
        return value if result.returncode == 0 and FULL.fullmatch(value) else None

    def commit(self, value, what):
        found = self.find(value)
        if found is None:
            raise Bad(f"{what}的提交 {value[:12]} 不在这个仓库里（先 git fetch，再核对记录）")
        return found

    def is_ancestor(self, older, newer):
        return self.git("merge-base", "--is-ancestor", older, newer, accept=(0, 1)).returncode == 0

    def describe(self, commit):
        """短号、提交日期、标题；标题里的控制字符换成空格，免得搅乱报告。"""
        date, _, subject = self.text("log", "-1", "--no-show-signature", "--format=%cs%x1f%s", commit).strip("\n").partition("\x1f")
        return f"{commit[:7]} {date} {''.join(one if one.isprintable() else ' ' for one in subject)}"

    def commits(self, older, newer):
        return self.text("rev-list", "--no-merges", f"{older}..{newer}").split()

    def is_merge(self, commit):
        return len(self.text("rev-list", "--parents", "-n", "1", commit).split()) > 2

    def files(self, commit):
        """这个提交改了哪些路径；合并提交取它相对第一个父提交带进来的。"""
        if commit not in self.changed:
            flags = ("diff-tree", "-r", "--no-renames", "--name-only", "--no-ext-diff")
            if self.is_merge(commit):
                self.changed[commit] = self.names(*flags, commit + "^1", commit)
            else:
                self.changed[commit] = self.names(*flags, "--root", "--no-commit-id", commit)
        return self.changed[commit]

    def between(self, older, newer):
        return self.names("diff-tree", "-r", "--no-renames", "--name-only", "--no-ext-diff", older, newer)

    def blob(self, commit, path):
        result = self.git("cat-file", "blob", f"{commit}:{path}", accept=(0, 128))
        return result.stdout if result.returncode == 0 else None

    def tree(self, commit, directory):
        return self.names("ls-tree", "-r", "--full-tree", "--name-only", commit, directory)

    def manifests(self, commit):
        if commit not in self.manifest_cache:
            paths = [path for path in self.tree(commit, "crates") if re.fullmatch(r"crates/[^/]+/Cargo\.toml", path)]
            self.manifest_cache[commit] = {path: self.blob(commit, path).decode("utf-8", "replace") for path in paths}
        return self.manifest_cache[commit]


# ---- 发行记录 ----

def no_duplicates(pairs):
    value = dict(pairs)
    if len(value) != len(pairs):
        raise ValueError("duplicate key")
    return value


def expect(condition, message):
    if not condition:
        raise Bad("169 的发行记录读不懂：" + message)


def check_text(value, pattern, what):
    expect(isinstance(value, str) and pattern.fullmatch(value) is not None, what + "的形状不对")
    return value


def check_item(value, what, required, optional=()):
    """记录里的一项：要么只有 error，要么键一个不多一个不少。"""
    expect(isinstance(value, dict), what + "不是对象")
    if set(value) == {"error"}:
        reason = value["error"]
        expect(isinstance(reason, str) and 0 < len(reason) <= 200 and reason.isprintable(), what + "的 error 不是一句话")
        return value
    expect(set(required) <= set(value) <= set(required) | set(optional), what + "的键对不上")
    for key, pattern in {**required, **dict(optional)}.items():
        if key in value:
            check_text(value[key], pattern, f"{what}的 {key}")
    return value


def load_record(path):
    """读并逐项核 169 的记录；一处不对就失败关闭。"""
    try:
        with open(path, "rb") as handle:
            data = handle.read(MAX_RECORD_BYTES + 1)
    except OSError as error:
        raise Bad("打不开 169 的发行记录") from error
    expect(len(data) <= MAX_RECORD_BYTES, "太大")
    try:
        value = json.loads(data.decode("utf-8"), object_pairs_hook=no_duplicates)
    except (ValueError, RecursionError) as error:
        raise Bad("169 的发行记录读不懂：不是没有重复键的 JSON") from error
    expect(isinstance(value, dict) and set(value) == {"schema", "collected_at", "human", "agent", "alert"}, "顶层的键对不上")
    expect(type(value["schema"]) is int and value["schema"] == SCHEMA, "版本不认识")
    check_text(value["collected_at"], MOMENT, "取记录的时刻")
    check_item(value["human"], "人侧", {"release": NAME, "build": NAME, "human_program_source_commit": FULL,
                                        "web_source_commit": FULL, "bootstrap_source_commit": FULL},
               {"sync_program_source_commit": FULL})
    check_item(value["agent"], "agent", {"release": NAME, "commit": FULL})
    check_item(value["alert"], "告警包", {"commit": SHORT})
    return value


# ---- 线上各发行的来源提交 ----

class Live:
    """一个发行线上的来源：有提交；或者不知道（写明为什么）；或者这个发行里本来就没有这一块。"""

    def __init__(self, commit=None, label="", unknown=None, absent=None, problems=()):
        self.commit = commit
        self.label = label
        self.unknown = unknown
        self.absent = absent
        self.problems = list(problems)


def live_from_record(repo, record):
    result = {}
    human, agent, alert = record["human"], record["agent"], record["alert"]
    for title, key in ((HUMAN, "human_program_source_commit"), (WEB, "web_source_commit"),
                       (TOOLS, "sync_program_source_commit")):
        if "error" in human:
            result[title] = Live(unknown=human["error"])
        elif key not in human:
            result[title] = Live(absent="这个发行没有随包工具。")
        else:
            result[title] = Live(repo.commit(human[key], title), f"发行 {human['release']}")
    result[AGENT] = (Live(unknown=agent["error"]) if "error" in agent
                     else Live(repo.commit(agent["commit"], AGENT), f"发布名 {agent['release']}"))
    result[ALERT] = (Live(unknown=alert["error"]) if "error" in alert
                     else Live(repo.commit(alert["commit"], ALERT), "169 那一份；108 那一份不在这次记录里"))
    return result


def live_from_argument(repo, value, title):
    if not SHORT.fullmatch(value):
        raise Bad(f"{title}的提交号要写 7～40 位小写十六进制")
    return Live(repo.commit(value, title), "由命令行给出")


def live_mac(repo):
    """本机 LaunchAgent 指到的同步包：目录名里带提交号，再逐字节核内容。"""
    try:
        with open(os.path.expanduser(LAUNCH_AGENT), "rb") as handle:
            script = plistlib.load(handle)["ProgramArguments"][1]
        package = os.path.dirname(os.path.dirname(script))
        name = os.path.basename(package)
        present = {one: os.path.join(package, "scripts", one) for one in os.listdir(os.path.join(package, "scripts"))
                   if one == "sync-agent-vault.sh" or re.fullmatch(r"replica_\w+\.py", one)}
        contents = {}
        for one, path in present.items():
            with open(path, "rb") as handle:
                contents["scripts/" + one] = handle.read()
    except (OSError, ValueError, KeyError, IndexError, TypeError, AttributeError, plistlib.InvalidFileException):
        return Live(unknown="本机没有装同步包，或者读不了它；在装着的那台 Mac 上跑，或者用 --mac 给提交号")
    commits = [repo.find(one) for one in re.findall(r"(?:^|-)([0-9a-f]{7,40})(?=-|$)", name)]
    commits = [one for one in commits if one]
    if len(commits) != 1 or not NAME.fullmatch(name):
        return Live(unknown="同步包的目录名里认不出唯一的提交号；用 --mac 给提交号")
    commit = commits[0]
    own = {path for path in repo.tree(commit, "scripts")
           if path == "scripts/sync-agent-vault.sh" or re.fullmatch(r"scripts/replica_\w+\.py", path)}
    problems = [f"{path} 和这个提交里的不一样" for path in sorted(own | set(contents))
                if contents.get(path) is None or contents[path] != repo.blob(commit, path)]
    return Live(commit, f"本机 {name}", problems=problems)


# ---- 对账 ----

class Drift:
    """一个发行的对账结果。"""

    def __init__(self, part, live):
        self.part = part
        self.live = live
        self.commits = []
        self.shared = 0
        self.net = []
        self.merged = []
        self.off_main = False

    @property
    def unclear(self):
        return self.live.unknown is not None

    @property
    def drifted(self):
        return bool(self.commits or self.shared or self.net or self.off_main or self.live.problems)


def compare(repo, part, live, main):
    drift = Drift(part, live)
    if live.commit is None:
        return drift
    if not repo.is_ancestor(live.commit, main):
        drift.off_main = True
        return drift
    drift.commits = [one for one in repo.commits(live.commit, main) if any(map(part.owns, repo.files(one)))]
    drift.net = [path for path in repo.between(live.commit, main) if part.owns(path)]
    # 普通提交里一个都没改过、两头却不一样：改动夹在合并提交里（解冲突时带的），不能漏。
    listed = {path for one in drift.commits for path in repo.files(one)}
    drift.merged = [path for path in drift.net if path not in listed]
    return drift


def hide_shared(drifts):
    """随包工具和人侧程序共用代码：已经列在人侧程序下面的提交不重复列。"""
    by_title = {drift.part.title: drift for drift in drifts}
    above = set(by_title[HUMAN].commits)
    tools = by_title[TOOLS]
    own = [one for one in tools.commits if one not in above]
    tools.shared = len(tools.commits) - len(own)
    tools.commits = own


def unplaced(repo, drifts, parts, main):
    """线上到 main 之间改过、却哪儿都归不到的路径。"""
    paths = set()
    for drift in drifts:
        if drift.live.commit and not drift.off_main:
            for commit in repo.commits(drift.live.commit, main):
                paths.update(repo.files(commit))
    return sorted(path for path in paths if home(path, parts) is None)


# ---- BUG 档 ----

BUG_FILE = re.compile(r"BUG-(\d+)-.*\.md")
MERGED = re.compile(r"合(?:入|进|并到|并进|并入)?\s*main")
NOT_MERGED = re.compile(r"(?:没|未|不|待|等)(?:有|着)?合")
RANGE = re.compile(r"([0-9a-f]{7,40})\.\.([0-9a-f]{7,40})")


class Bug:
    def __init__(self, number, title, status, note, body):
        self.number = number
        self.title = title
        self.status = status
        self.note = note
        self.body = body
        self.reason = ""

    def tokens(self):
        """状态行反引号里的提交号；没有才看正文里说「合 main」的那些句子（说没合的不算）。"""
        found = commit_tokens(self.note)
        if not found:
            for sentence in re.split(r"[。；;\n]", self.body):
                if MERGED.search(sentence) and not NOT_MERGED.search(sentence):
                    found += commit_tokens(sentence)
        return found


def commit_tokens(text):
    return [one for one in re.findall(r"`([^`\n]+)`", text) if SHORT.fullmatch(one) or RANGE.fullmatch(one)]


def load_bugs(directory):
    """「修复中」的 BUG 档；读不了就失败关闭。"""
    bugs = []
    try:
        names = sorted(os.listdir(directory))
        for name in names:
            if not BUG_FILE.fullmatch(name):
                continue
            with open(os.path.join(directory, name), "rb") as handle:
                data = handle.read(MAX_NOTE_BYTES + 1)
            if len(data) > MAX_NOTE_BYTES:
                raise Bad(f"06-BUG 里的 {name[:7]} 太大")
            text = data.decode("utf-8")
            status = re.search(r"\A﻿?---\r?\n(?:.*\r?\n)*?状态:[ \t]*(\S+)(.*)", text)
            heading = re.search(r"^# (.*)$", text, re.M)
            number = int(BUG_FILE.fullmatch(name)[1])
            title = heading[1].strip() if heading else name
            title = "".join(one if one.isprintable() else " " for one in title)
            if status and status[1].startswith("修复中"):
                bugs.append(Bug(number, re.sub(r"^BUG-\d+\s*", "", title), status[1], status[2], text))
    except (OSError, UnicodeDecodeError) as error:
        raise Bad("读不了 06-BUG") from error
    return bugs


def fixes(repo, bug):
    """这个 BUG 的修复提交（全长）；看不明白就回空并写明原因。"""
    found, lost = [], []
    for token in bug.tokens():
        span = RANGE.fullmatch(token)
        if span:
            ends = [repo.find(one) for one in span.groups()]
            commits = repo.commits(*ends) if all(ends) else []
            if not commits or len(commits) > MAX_RANGE:
                lost.append(token)
            found += commits
            continue
        commit = repo.find(token)
        if commit:
            found.append(commit)
        elif not token.isdigit():  # 纯数字多半是日期或编号
            lost.append(token)
    if lost:
        bug.reason = f"这个仓库里没有这个提交：{'、'.join(lost)}"
        return []
    if not found:
        bug.reason = "看不出修复提交"
    return list(dict.fromkeys(found))


def reached(repo, commit, parts):
    """这个提交碰了哪些发行，另外碰了哪些单独部署的东西、哪些装配工具。"""
    paths = [path for path in repo.files(commit) if not is_inert(path)]
    # 锁文件跟着任何一个包变；只要还有别的产品路径，就不拿它当「每个 Rust 程序都改了」。
    narrow = [path for path in paths if path not in RUST] or paths
    owners = [part.title for part in parts if any(map(part.owns, narrow))]
    beside = {}
    for path in narrow:
        found = None if any(part.owns(path) for part in parts) else outside(path)
        if found and found[1]:
            beside.setdefault(found[1], set()).add(found[0])
    return owners, beside


def judge(repo, bug, parts, drifts, main):
    """该验收了／还没上线／要人看，连同一句说明。"""
    commits = fixes(repo, bug)
    if not commits:
        return "manual", bug.reason
    off = [one[:7] for one in commits if not repo.is_ancestor(one, main)]
    if off:
        return "manual", f"修复提交不在 main：{'、'.join(off)}"
    # 合并提交带进来的东西可能多过这个 BUG；有普通提交就只看普通提交。
    plain = [one for one in commits if not repo.is_merge(one)] or commits
    by_title = {drift.part.title: drift for drift in drifts}
    missing, unclear, touched, elsewhere, tooling, behind = {}, set(), set(), set(), set(), set()
    for commit in plain:
        owners, beside = reached(repo, commit, parts)
        elsewhere |= beside.get("另装", set())
        tooling |= beside.get("装配", set())
        for title in owners:
            live = by_title[title].live
            if live.absent:
                continue
            touched.add(title)
            if live.commit is None:
                unclear.add(title)
            elif not repo.is_ancestor(commit, live.commit):
                # 随包工具和人侧程序共用代码：人侧程序已经有了，工具里用不用得上交人看，不算没上线。
                if title == TOOLS and HUMAN in owners:
                    behind.add(commit[:7])
                else:
                    missing.setdefault(title, []).append(commit[:7])
    if elsewhere:
        return "manual", f"修复还碰了不归这五个发行的：{'、'.join(sorted(elsewhere))}"
    if not touched:
        return "manual", "修复提交没有碰五个发行的代码"
    if unclear:
        return "manual", f"{'、'.join(sorted(unclear))} 线上是哪个提交未知"
    if missing:
        return "waiting", "；".join(f"{title} 还没有 {'、'.join(commits)}" for title, commits in missing.items())
    note = f"已在 {'、'.join(part.title for part in parts if part.title in touched)}：{'、'.join(one[:7] for one in plain)}"
    if behind:
        note += f"；随包工具还没有 {'、'.join(sorted(behind))}，用不用得上要人看"
    if tooling:
        note += f"；另改了{'、'.join(sorted(tooling))}，装没装进去要人看"
    return "ready", note


# ---- 报告 ----

def render_part(repo, drift):
    live = drift.live
    title = drift.part.title
    if live.unknown is not None:
        return [f"## {title}", f"未知：{live.unknown}。"]
    if live.absent is not None:
        return [f"## {title}", live.absent]
    lines = [f"## {title} · 线上 {live.commit[:7]}（{live.label}）"]
    lines += [f"- 内容对不上：{problem}" for problem in live.problems]
    if drift.off_main:
        return lines + ["线上源提交不在 main：没法按提交对账，先弄清它从哪条分支出的。"]
    if drift.commits:
        lines.append(f"{len(drift.commits)} 个提交已合 main、不在线上（净改动 {len(drift.net)} 个文件）：")
        lines += [f"- {repo.describe(one)}" for one in drift.commits]
    if drift.shared:
        lines.append(f"另有 {drift.shared} 个和人侧程序共有的提交不在线上，见人侧程序那一节。")
    if drift.merged and not drift.shared:
        lines.append("合并提交里带的改动（没有普通提交改过这些文件）：")
        lines += [f"- {path}" for path in drift.merged]
    if not drift.drifted:
        lines.append("没有漂移。")
    return lines


def render_bugs(title, entries):
    lines = [f"## {title}"]
    lines += [f"- BUG-{bug.number:03d} {bug.title}：{note}" for bug, note in entries] or ["没有。"]
    return lines


def conclude(drifts, lost, bugs):
    titles = [drift.part.title for drift in drifts if drift.drifted]
    unknown = [drift.part.title for drift in drifts if drift.unclear]
    ready = f"有 {len(bugs['ready'])} 个 BUG 该验收了" if bugs["ready"] else "没有该验收的 BUG"
    if titles:
        text = f"结论：有漂移（{'、'.join(titles)}）"
    elif unknown or lost:
        text = "结论：看不清"
    else:
        text = "结论：五个发行都没有漂移"
    if unknown:
        text += f"，{'、'.join(unknown)} 未知"
    if lost:
        text += f"，有 {len(lost)} 个路径没归类"
    text += f"，{ready}。"
    clean = not (titles or unknown or lost or bugs["ready"] or bugs["manual"])
    return text, 0 if clean else 1


def report(arguments):
    record = load_record(arguments.record)
    repo = Repo(os.path.abspath(arguments.repo))
    if not REF.fullmatch(arguments.main):
        raise Bad("--main 不是一个引用名")
    main = repo.commit(arguments.main, "main ")
    live = live_from_record(repo, record)
    live[MAC] = live_from_argument(repo, arguments.mac, MAC) if arguments.mac is not None else live_mac(repo)
    live[WORKER] = (live_from_argument(repo, arguments.worker, WORKER) if arguments.worker is not None
                    else Live(unknown="Worker 没有机器可读的发行记录；用 --worker 给最后一次 wrangler 发布所在的提交"))
    commits = sorted({one.commit for one in live.values() if one.commit})
    parts = parts_for([repo.manifests(one) for one in [main, *commits]])
    drifts = [compare(repo, part, live[part.title], main) for part in parts]
    lost = unplaced(repo, drifts, parts, main)
    hide_shared(drifts)
    bugs = {"ready": [], "waiting": [], "manual": []}
    for bug in load_bugs(arguments.bugs):
        verdict, note = judge(repo, bug, parts, drifts, main)
        bugs[verdict].append((bug, note))

    text, code = conclude(drifts, lost, bugs)
    lines = ["# 部署漂移对账", f"main：{repo.describe(main)}（{arguments.main}；本脚本不 fetch，旧了就先 git fetch）",
             f"169 的记录：{record['collected_at']} 取"]
    if "error" not in record["human"]:
        lines.append(f"首建工具出自 {record['human']['bootstrap_source_commit'][:7]}，来源冻结，不算漂移。")
    lines += [text, ""]
    for drift in drifts:
        lines += [*render_part(repo, drift), ""]
    for title, key in (("该验收了（修复提交都已上线，档里还写着修复中）", "ready"), ("修复中、还没上线", "waiting"),
                       ("修复中、要人看", "manual")):
        lines += [*render_bugs(title, bugs[key]), ""]
    if lost:
        lines += ["## 没归类的路径（给 scripts/deploy-drift.py 补归类）", *[f"- {path}" for path in lost], ""]
    elsewhere = sorted({title for _, title, kind in OUTSIDE if kind == "另装"})
    lines += ["## 不对账的", "单独部署、这里看不出上没上线：" + "、".join(elsewhere) + "。"]
    print("\n".join(lines))
    return code


# ---- 入口 ----

def print_command(arguments):
    if not HOST.fullmatch(arguments.host):
        raise Bad("--host 只能是 SSH 名或 用户@地址")
    print("# 第 1 步：在 169 上只读取发行记录（不带特权，不写 169）")
    print(f"python3 scripts/deploy-drift.py probe | ssh {arguments.host} 'python3 -I -B -S -' > {RECORD_PATH}")
    print("# 第 2 步：对账（Worker 线上是哪个提交要另用 --worker 给）")
    print(f"python3 scripts/deploy-drift.py report --record {RECORD_PATH}")
    return 0


def print_probe(_):
    sys.stdout.write(PROBE)
    return 0


def parse(argv):
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    vault = os.environ.get("OBSIDIAN_VAULT_PATH") or os.path.expanduser(VAULT)
    parser = argparse.ArgumentParser(prog="deploy-drift", description="部署漂移对账：只读，不联网")
    commands = parser.add_subparsers(dest="command", required=True)
    one = commands.add_parser("command", help="打印要用户敲的那条只读命令")
    one.add_argument("--host", default=DEFAULT_HOST, help="169 的 SSH 名")
    one.set_defaults(run=print_command)
    one = commands.add_parser("probe", help="打印在 169 上跑的只读程序")
    one.set_defaults(run=print_probe)
    one = commands.add_parser("report", help="对账")
    one.add_argument("--record", required=True, help="169 的发行记录（command 那条命令的输出）")
    one.add_argument("--main", default="origin/main", help="拿哪个引用当 main")
    one.add_argument("--repo", default=here, help="本机仓库")
    one.add_argument("--bugs", default=os.path.join(vault, BUGS), help="06-BUG 目录")
    one.add_argument("--mac", help="Mac 副本包的提交号；不给就读本机的 LaunchAgent")
    one.add_argument("--worker", help="Worker 线上那一版的提交号；不给就报未知")
    one.set_defaults(run=report)
    return parser.parse_args(argv)


def main(argv=None):
    arguments = parse(argv)
    try:
        return arguments.run(arguments)
    except Bad as error:
        print(f"deploy-drift: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
