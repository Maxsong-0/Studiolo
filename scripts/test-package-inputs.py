#!/usr/bin/env python3
"""打包白名单的 Python 本地依赖守卫：清单带进包里的 scripts/ 下每个 .py，其本地依赖也必须在同一份
STUDIOLO_INPUTS 里（等于某个条目，或落在某个目录条目之下）。只用 ast 读源码，不执行、不构建、不联网。
起因（2026-09-23 T-157）：T-139 新增 scripts/client_projection_policy_pipeline.py 时差点漏进 package-agent.sh；
CI 整仓检出一路全绿，运行包却只带白名单里的文件，包里的管线一跑就 ModuleNotFoundError，要到真打包才发现。

口径（缺一个就红，读不懂也红）：
- 清单：唯一一处独占一行的 `STUDIOLO_INPUTS=(` 多行数组；其余提到它的地方（整行注释除外）只许
  `"${STUDIOLO_INPUTS[@]}"` 展开。追加、改元素、单行写法、引号、通配、变量、花括号一律判红。
- 核对对象：清单带进包里的 scripts/ 下全部 .py（显式条目，及 scripts/ 内目录条目下的文件）。crates/、
  deploy/ 等目录里的 .py 按各自目录或别的部署通道解析依赖，不在 scripts/ 这个导入命名空间里。
- 本地依赖：import／from-import／import_module 的顶层名在 scripts/ 下有同名 .py 或包目录（包按整目录算）；
  spec_from_file_location、SourceFileLoader、runpy.run_path 路径参数里完整的 "*.py" 字面量，以所在目录、
  scripts/、仓库根为基准落到 scripts/ 下已有文件的。路径参数不是字面量（经辅助函数、变量或 f-string
  传入）时退回该文件全部 "*.py" 字面量，一个都没有就判红。依赖的依赖一路递归；标准库与第三方模块不算。
  子进程按路径执行的脚本不在本守卫范围。

另一类同样只有真打包才发现的缺口（2026-11-15，T-187）：两个运行包都带着架构守卫的测试程序到服务器上跑，
守卫在运行时按仓库根读文件。源码里点名的、crates/ 以外的仓库文件必须在两份清单里；漏带时那条守卫在包里
读不到文件。口径：架构守卫源码（crates/studiolo/tests 下的 architecture.rs、architecture_cases、
architecture_support）里的字符串字面量，恰好是仓库里一个已有的普通文件、且不在 crates/ 下的，都算。

第三类（2026-11-17，T-207 部署）：vault-broker 的 preflight_script 用例运行时按
`concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/check-backup-preflight.py")` 调脚本，两份清单都没带；
测试箱整仓检出一路全绿，agent 运行包到 169 原生自检才红。口径：打包脚本随包带测试的 crate（`cargo test --workspace`
就是全部 crate，否则按 STUDIOLO_TEST_PACKAGES 的 `-p`）里，代码位置（注释、字符串里的不算）每一处
`env!("CARGO_MANIFEST_DIR")` 都必须写成 `concat!(env!(…), "/相对路径")` 或 `Path|PathBuf::new|from(env!(…))`
后接零到多个 `.join("字面量")`；拼出的路径要留在仓库里、真实存在、落在 STUDIOLO_INPUTS 里。别的写法一律判红。
"""
import ast
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
HEADER = "STUDIOLO_INPUTS=("
EXPANSION = '"${STUDIOLO_INPUTS[@]}"'
SEGMENT = r"[A-Za-z0-9_.][A-Za-z0-9_.-]*"
ENTRY = re.compile(rf"{SEGMENT}(?:/{SEGMENT})*")
# 按路径加载 Python 文件的调用：函数名 -> (路径参数的位置, 关键字名)。
LOADERS = {"spec_from_file_location": (1, "location"), "SourceFileLoader": (1, "path"),
           "run_path": (0, "path_name")}
IMPORTERS = ("import_module", "__import__")
# 真实源码里 import、spec_from_file_location、经辅助函数 load(name, filename) 传入文件名三种依赖各举一例：
# 守卫必须认得出（一条都认不出的「全绿」不算数）；从清单删掉被依赖的一方，守卫必须点名引用方。
EDGES = {"scripts/package-agent.sh": [
             ("scripts/check-client-projection-pipeline.py", "scripts/client_projection_policy_pipeline.py"),
             ("scripts/test-check-agent.py", "scripts/check-agent.py"),
             ("scripts/test-check-agent-replica.py", "scripts/test-check-agent.py")],
         "scripts/package-preview.sh": [("scripts/test-check-preview-exercise.py", "scripts/test-check-preview.py")]}


def manifest_entries(text):
    """按 bash 数组字面量读出 STUDIOLO_INPUTS 的条目；返回 (条目, 读不懂的写法)。"""
    lines = text.splitlines()
    code = [line for line in lines if not line.lstrip().startswith("#")]
    mentions = sum(len(re.findall(r"\bSTUDIOLO_INPUTS\b", line)) for line in code)
    expansions = sum(line.count(EXPANSION) for line in code)
    starts = [index for index, line in enumerate(lines) if line == HEADER]
    if len(starts) != 1 or mentions != 1 + expansions:
        return [], [f"STUDIOLO_INPUTS 必须恰好一处独占一行的 `{HEADER}` 定义，其余只许 {EXPANSION} 展开"]
    entries, problems = [], []
    for line in lines[starts[0] + 1:]:
        if line.strip() == ")":
            return entries, problems
        for word in line.split():
            if word.startswith("#"):
                break  # 词首的 # 起注释，本行余下都是注释
            if ENTRY.fullmatch(word) and not {".", ".."} & set(word.split("/")):
                entries.append(word)
            else:
                problems.append(f"读不懂的清单写法：{word}")
    return [], ["STUDIOLO_INPUTS 缺少独占一行的右括号"]


def python_literals(node):
    """node 之下完整的 "*.py" 字符串字面量；f-string 的片段不算，那是运行时才拼出的名字。"""
    parts = {id(part) for joined in ast.walk(node) if isinstance(joined, ast.JoinedStr) for part in joined.values}
    return {item.value for item in ast.walk(node) if isinstance(item, ast.Constant) and id(item) not in parts
            and isinstance(item.value, str) and PurePosixPath(item.value).suffix == ".py"}


def references(path):
    """一个 Python 文件的本地依赖候选：(导入的顶层名, 按路径加载的 .py 字面量, 读不懂的写法)。"""
    try:
        tree = ast.parse(path.read_text(encoding="utf-8"), str(path))
    except (SyntaxError, UnicodeDecodeError, ValueError) as error:
        return set(), set(), [f"无法按 Python 源码解析（{type(error).__name__}）"]
    modules, literals, problems, dynamic = set(), set(), [], False
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            modules.update(alias.name.split(".")[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
            # 相对导入只在包内有效，已含在整包依赖里；scripts/ 顶层文件用它本身就跑不起来。
            modules.add(node.module.split(".")[0])
        elif isinstance(node, ast.Call):
            name = getattr(node.func, "attr", getattr(node.func, "id", None))
            if name in LOADERS:
                index, keyword = LOADERS[name]
                argument = node.args[index] if len(node.args) > index else next(
                    (item.value for item in node.keywords if item.arg == keyword), None)
                found = python_literals(argument) if argument is not None else set()
                literals |= found
                dynamic = dynamic or not found
            elif name in IMPORTERS and node.args:
                if isinstance(node.args[0], ast.Constant) and isinstance(node.args[0].value, str):
                    modules.add(node.args[0].value.split(".")[0])
                else:
                    problems.append(f"{name} 的模块名不是字面量，无法静态确定")
    if dynamic:
        literals |= python_literals(tree)
        if not literals:
            problems.append("按路径加载的目标不是字面量，文件里也没有可退回的 .py 字面量")
    return modules, literals, problems


def python_files(directory):
    return sorted(path for path in directory.rglob("*.py") if path.is_file() and "__pycache__" not in path.parts)


def local_dependencies(path, root):
    """把候选落到 scripts/ 下真实存在的文件；返回 (依赖文件, 读不懂的写法)。"""
    scripts = root / "scripts"
    modules, literals, problems = references(path)
    found = set()
    for module in filter(str.isidentifier, modules):
        if (scripts / f"{module}.py").is_file():
            found.add(scripts / f"{module}.py")
        elif (scripts / module).is_dir():
            found.update(python_files(scripts / module))  # 包按整目录算
    for literal in literals:
        for base in (path.parent, scripts, root):
            target = Path(os.path.normpath(base / literal))
            if target.is_file() and target.is_relative_to(scripts):
                found.add(target)
    found.discard(path)
    return found, problems


def scan(root, script):
    """核对一个打包脚本；返回 (依赖边 {(引用方, 依赖)}, 缺口)。路径都相对仓库根。"""
    root = Path(root).resolve()
    entries, problems = manifest_entries((root / script).read_text(encoding="utf-8"))
    if problems:
        return set(), [f"{script}: {problem}" for problem in problems]
    relative = lambda path: path.relative_to(root).as_posix()
    covered = lambda name: any(name == entry or name.startswith(entry + "/") for entry in entries)
    gaps, pending = [], []
    for entry in entries:
        if entry != "scripts" and not entry.startswith("scripts/"):
            continue
        target = root / entry
        if target.is_dir():
            pending += python_files(target)
        elif entry.endswith(".py"):
            if target.is_file():
                pending.append(target)
            else:
                gaps.append(f"{script}: 清单里的 {entry} 不存在，核对不了它的依赖")
    edges, seen = set(), set()
    while pending:
        path = pending.pop()
        if path in seen:
            continue
        seen.add(path)
        found, trouble = local_dependencies(path, root)
        gaps += [f"{script}: {relative(path)} {item}" for item in trouble]
        for dependency in sorted(found):
            edges.add((relative(path), relative(dependency)))
            if not covered(relative(dependency)):
                gaps.append(f"{script}: {relative(path)} -> {relative(dependency)}，后者不在 STUDIOLO_INPUTS 里")
            pending.append(dependency)
    return edges, sorted(gaps)


ARCHITECTURE = ("crates/studiolo/tests/architecture.rs", "crates/studiolo/tests/architecture_cases",
                "crates/studiolo/tests/architecture_support")
LITERAL = re.compile(rf'"({SEGMENT}(?:/{SEGMENT})*)"')


def architecture_inputs(root):
    """架构守卫源码点名的、crates/ 以外的仓库文件；返回 {仓库相对路径: 点名它的源码文件}。"""
    root = Path(root).resolve()
    sources = []
    for entry in ARCHITECTURE:
        target = root / entry
        sources += sorted(target.rglob("*.rs")) if target.is_dir() else [target]
    named = {}
    for source in sources:
        for literal in LITERAL.findall(source.read_text(encoding="utf-8")):
            if literal.startswith("crates/") or {".", ".."} & set(literal.split("/")):
                continue
            if (root / literal).is_file():
                named.setdefault(literal, source.relative_to(root).as_posix())
    return named


def architecture_gaps(root, script):
    """一个打包脚本漏带的、架构守卫运行时要读的文件。"""
    entries, problems = manifest_entries((Path(root) / script).read_text(encoding="utf-8"))
    if problems:
        return [f"{script}: {problem}" for problem in problems]
    covered = lambda name: any(name == entry or name.startswith(entry + "/") for entry in entries)
    return sorted(f"{script}: 架构守卫 {source} 要读 {name}，它不在 STUDIOLO_INPUTS 里"
                  for name, source in architecture_inputs(root).items() if not covered(name))


class ArchitectureRuntimeInputsTests(unittest.TestCase):
    """架构守卫运行时读的仓库文件必须随两个运行包走；从清单删掉其中一个，必须红并点名。"""

    # 守卫必须认得出这些（一个都认不出的「全绿」不算数）。
    KNOWN = (".config/nextest.toml", ".testbox.toml", "scripts/test-linux.sh", "Cargo.toml")

    def test_both_manifests_carry_every_file_the_architecture_guards_read(self):
        named = architecture_inputs(ROOT)
        for name in self.KNOWN:
            self.assertIn(name, named, "守卫没认出架构测试点名的文件，扫描逻辑失效")
        for script in EDGES:
            with self.subTest(script=script):
                self.assertEqual(architecture_gaps(ROOT, script), [],
                                 "打包清单漏了架构守卫要读的文件，随包测试在服务器上会读不到")

    def test_dropping_a_named_file_from_either_manifest_is_red(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for entry in ARCHITECTURE:
                source, target = ROOT / entry, root / entry
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copytree(source, target) if source.is_dir() else shutil.copy(source, target)
            named = architecture_inputs(ROOT)
            for name in named:
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                shutil.copy(ROOT / name, root / name)
            for script in EDGES:
                original = (ROOT / script).read_text(encoding="utf-8")
                (root / script).parent.mkdir(parents=True, exist_ok=True)
                (root / script).write_text(original, encoding="utf-8")
                self.assertEqual(architecture_gaps(root, script), [], "复制出的副本在变异前就不绿")
                for name in self.KNOWN:
                    if name == "Cargo.toml":
                        continue  # 清单里另有 Cargo.lock 等以它开头的词，单独变异它不干净；另三个足够
                    with self.subTest(script=script, name=name):
                        mutated, count = re.subn(rf"(?<!\S){re.escape(name)}(?!\S)", "", original)
                        self.assertEqual(count, 1, f"{script} 里没有唯一一处 {name}，变异没生效")
                        (root / script).write_text(mutated, encoding="utf-8")
                        gaps = architecture_gaps(root, script)
                        self.assertEqual(len(gaps), 1, gaps)
                        self.assertIn(f"要读 {name}，", gaps[0])
                (root / script).write_text(original, encoding="utf-8")


# 第三类守卫的词法：注释和字符串、字符字面量整段认出来，落在里面的 env! 不是真依赖。
# 转义写 \\[\s\S] 而不是 \\.：字符串里的 `\` 接换行（续行）也是转义，`.` 不匹配换行会让后面的引号全部错位。
RUST_MASKED = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|r(#*)"[\s\S]*?"\1|b?"(?:\\[\s\S]|[^"\\])*"'
                         r"|b?'(?:\\[\s\S]|[^'\\])'")
MANIFEST_DIR = re.compile(r'env!\(\s*"CARGO_MANIFEST_DIR"\s*\)')
CONCAT_FORM = re.compile(r'concat!\(\s*env!\(\s*"CARGO_MANIFEST_DIR"\s*\)\s*,\s*"([^"\\]*)"\s*,?\s*\)')
JOIN_FORM = re.compile(r'\b(?:Path|PathBuf)::(?:new|from)\(\s*env!\(\s*"CARGO_MANIFEST_DIR"\s*\)\s*\)'
                       r'((?:\s*\.join\(\s*"[^"\\]*"\s*\))*)')
JOIN_PART = re.compile(r'\.join\(\s*"([^"\\]*)"\s*\)')


def tested_crates(root, text):
    """打包脚本随包带哪些 crate 的测试；返回 (crate 目录列表, 读不懂的写法)。"""
    names = {}
    for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
        package = manifest.read_text(encoding="utf-8").split("[package]", 1)[-1].split("\n[", 1)[0]
        found = re.search(r'^name\s*=\s*"([^"]+)"', package, re.M)
        if found:
            names[found.group(1)] = manifest.parent
    if re.search(r"\bcargo test --workspace\b", text):
        return sorted(names.values()), []
    block = re.search(r"^STUDIOLO_TEST_PACKAGES=\(\n(.*?)\n\)", text, re.S | re.M)
    if block is None:
        return [], ["既没有 `cargo test --workspace` 也没有 STUDIOLO_TEST_PACKAGES，认不出随包带哪些 crate 的测试"]
    picked = re.findall(r"-p\s+(\S+)", block.group(1))
    unknown = [f"STUDIOLO_TEST_PACKAGES 里的 {name} 不是工作区 crate" for name in picked if name not in names]
    if not picked:
        unknown.append("STUDIOLO_TEST_PACKAGES 是空的")
    return sorted({names[name] for name in picked if name in names}), unknown


def manifest_dir_inputs(root, crates):
    """随包 crate 的代码按 CARGO_MANIFEST_DIR 拼出的仓库路径；返回 ({仓库相对路径: [引用它的源码]}, 读不懂的写法)。"""
    named, problems = {}, []
    for crate in crates:
        for source in sorted(path for path in crate.rglob("*.rs") if "target" not in path.relative_to(crate).parts):
            text = source.read_text(encoding="utf-8")
            where = source.relative_to(root).as_posix()
            masked = [match.span() for match in RUST_MASKED.finditer(text)]
            in_code = lambda start: not any(low <= start < high for low, high in masked)
            resolved = []  # (位置, 路径段或 None, 拼不出时的原因)
            for match in CONCAT_FORM.finditer(text):
                if in_code(match.start()):
                    literal = match.group(1)
                    resolved.append((match.span(), literal[1:].split("/"), None) if literal.startswith("/") else
                                    (match.span(), None, "concat! 里的路径字面量要以 / 开头（它直接接在 crate 目录后面）"))
            for match in JOIN_FORM.finditer(text):
                if in_code(match.start()):
                    parts = JOIN_PART.findall(match.group(1))
                    resolved.append((match.span(), None, "join 了绝对路径，会丢掉 crate 目录") if any(
                        part.startswith("/") for part in parts) else (match.span(), parts, None))
            for match in MANIFEST_DIR.finditer(text):
                if in_code(match.start()) and not any(low <= match.start() < high for (low, high), _, _ in resolved):
                    line = text.count("\n", 0, match.start()) + 1
                    problems.append(f"{where}:{line} 的 env!(\"CARGO_MANIFEST_DIR\") 写法认不出，"
                                    "改成 concat!(env!(…), \"/相对路径\") 或 Path::new(env!(…)).join(\"字面量\")")
            for _, parts, reason in resolved:
                target = Path(os.path.normpath(crate.joinpath(*parts))) if parts is not None else None
                if target is None:
                    problems.append(f"{where}：{reason}")
                elif not target.is_relative_to(root):
                    problems.append(f"{where} 按 CARGO_MANIFEST_DIR 拼出的路径跑出了仓库")
                elif not target.exists():
                    problems.append(f"{where} 按 CARGO_MANIFEST_DIR 拼出的 {target.relative_to(root).as_posix()} 不存在")
                else:
                    named.setdefault(target.relative_to(root).as_posix(), []).append(where)
    return named, problems


def manifest_dir_gaps(root, script, text=None):
    """一个打包脚本漏带的、随包测试运行时按 CARGO_MANIFEST_DIR 要读的文件；返回 (引用表, 缺口)。"""
    root = Path(root).resolve()
    text = (root / script).read_text(encoding="utf-8") if text is None else text
    entries, problems = manifest_entries(text)
    crates, trouble = tested_crates(root, text)
    named, unreadable = manifest_dir_inputs(root, crates)
    covered = lambda name: any(name == entry or name.startswith(entry + "/") for entry in entries)
    gaps = [f"{script}: {item}" for item in problems + trouble + unreadable]
    gaps += [f"{script}: {'、'.join(sorted(set(sources)))} 运行时按 CARGO_MANIFEST_DIR 读 {name}，它不在 STUDIOLO_INPUTS 里"
             for name, sources in named.items() if not problems and not covered(name)]
    return named, sorted(gaps)


class ManifestDirRuntimeInputsTests(unittest.TestCase):
    """真实仓库：两份清单都带齐随包测试按 CARGO_MANIFEST_DIR 读的文件；从清单删掉预检脚本，必须红并点名。"""

    # 守卫必须认得出这些（一个都认不出的「全绿」不算数）：跑出 crate 的 concat、crate 内的 concat、Path::new 后 join。
    KNOWN = {"scripts/package-agent.sh": ("scripts/check-backup-preflight.py",
                                          "crates/agent-server/tests/support/admin_terminal.py",
                                          "crates/agent-server/src"),
             "scripts/package-preview.sh": ("scripts/check-backup-preflight.py",
                                            "crates/auth-admin/tests/support/terminal_pty.py")}

    def test_both_manifests_carry_every_file_read_through_cargo_manifest_dir(self):
        for script, known in self.KNOWN.items():
            with self.subTest(script=script):
                named, gaps = manifest_dir_gaps(ROOT, script)
                self.assertEqual(gaps, [], "随包测试运行时要读的文件不在打包清单里，到服务器原生自检才会红")
                for name in known:
                    self.assertIn(name, named, "守卫没认出应有的引用，解析逻辑失效")
        # agent 运行包只带 STUDIOLO_TEST_PACKAGES 的测试：control-db、auth-admin 不随包，它们的引用不该算进来。
        agent = manifest_dir_gaps(ROOT, "scripts/package-agent.sh")[0]
        self.assertNotIn("crates/auth-admin/tests/support/terminal_pty.py", agent)
        self.assertEqual(agent["scripts/check-backup-preflight.py"],
                         ["crates/vault-broker/tests/broker/preflight_script.rs"])

    def test_dropping_the_preflight_script_from_either_manifest_is_red(self):
        name = "scripts/check-backup-preflight.py"
        for script in self.KNOWN:
            with self.subTest(script=script):
                original = (ROOT / script).read_text(encoding="utf-8")
                mutated, count = re.subn(rf"(?<!\S){re.escape(name)}(?!\S)", "", original)
                self.assertEqual(count, 1, f"{script} 里没有唯一一处 {name}，变异没生效")
                gaps = manifest_dir_gaps(ROOT, script, mutated)[1]
                self.assertEqual(len(gaps), 1, gaps)
                self.assertIn(f"读 {name}，", gaps[0])
                self.assertIn("crates/vault-broker/tests/broker/preflight_script.rs", gaps[0])


class SyntheticManifestDirTests(unittest.TestCase):
    """虚构的小仓库逐条钉住第三类口径：两种写法、注释与字符串不算、认不出就红、只看随包的 crate。"""

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name).resolve()
        self.write("crates/demo/Cargo.toml", '[package]\nname = "studiolo-demo"\n')
        self.write("crates/other/Cargo.toml", '[package]\nname = "studiolo-other"\n')
        self.write("crates/demo/tests/data.json", "{}\n")
        self.write("scripts/tool.py", "print('虚构工具')\n")

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def gaps(self, body, *entries, packages=None):
        self.write("crates/demo/tests/probe.rs", body)
        block = "\n".join(f"  {entry}" for entry in entries)
        tests = ("cargo test --workspace --no-run\n" if packages is None
                 else f"STUDIOLO_TEST_PACKAGES=(\n  {packages}\n)\n")
        text = f'STUDIOLO_INPUTS=(\n{block}\n)\ntar -cf - "${{STUDIOLO_INPUTS[@]}}"\n{tests}'
        return manifest_dir_gaps(self.root, "scripts/package.sh", text)[1]

    def test_concat_and_join_forms_resolve_and_must_be_covered(self):
        concat = 'const S: &str = concat!(\n    env!("CARGO_MANIFEST_DIR"),\n    "/../../scripts/tool.py"\n);\n'
        join = 'fn d() -> std::path::PathBuf { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data.json") }\n'
        bare = 'fn r() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }\n'
        self.assertEqual(self.gaps(concat + join + bare, "crates", "scripts/tool.py"), [])
        gaps = self.gaps(concat, "crates")
        self.assertEqual(len(gaps), 1, gaps)
        self.assertIn("crates/demo/tests/probe.rs 运行时按 CARGO_MANIFEST_DIR 读 scripts/tool.py，", gaps[0])
        # 前面有转义引号的字符串、字符字面量 '"'、带续行（`\` 接换行）的字符串，后面的引用照样认得出。
        # 续行那段放最后：转义写成 \\. 时它的收尾引号会被当成新字符串的开头，把后面的 concat! 吞进去。
        prelude = 'fn m() { let _ = "转义 \\" 引号"; let _q = \'"\'; let _ = "第一行\\\n  第二行"; }\n'
        gaps = self.gaps(prelude + concat, "crates")
        self.assertEqual(len(gaps), 1, gaps)
        self.assertIn("读 scripts/tool.py，", gaps[0])
        self.assertEqual(len(self.gaps(join, "scripts/tool.py")), 1)

    def test_comments_and_string_literals_do_not_count(self):
        body = ('// concat!(env!("CARGO_MANIFEST_DIR"), "/missing.txt")\n'
                '/* Path::new(env!("CARGO_MANIFEST_DIR")).join("nowhere") */\n'
                'const DOC: &str = "env!(\\"CARGO_MANIFEST_DIR\\")";\n'
                'const RAW: &str = r#"concat!(env!("CARGO_MANIFEST_DIR"), "/x")"#;\n')
        self.assertEqual(self.gaps(body, "crates"), [])

    def test_unreadable_missing_or_escaping_paths_are_red(self):
        for body in ['fn a() { let d = env!("CARGO_MANIFEST_DIR"); let _ = Path::new(d).join("x"); }\n',
                     'fn b() -> String { format!("{}/x", env!("CARGO_MANIFEST_DIR")) }\n',
                     'fn c() { let _ = Path::new(env!("CARGO_MANIFEST_DIR")).join("/etc/passwd"); }\n',
                     'const D: &str = concat!(env!("CARGO_MANIFEST_DIR"), "tests/data.json");\n',
                     'const E: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/missing.txt");\n',
                     'const F: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../outside");\n']:
            with self.subTest(body=body):
                gaps = self.gaps(body, "crates", "scripts")
                self.assertEqual(len(gaps), 1, gaps)

    def test_only_crates_whose_tests_ship_are_scanned(self):
        body = 'const S: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/tool.py");\n'
        self.assertEqual(self.gaps(body, "crates", packages="-p studiolo-other"), [])
        self.assertEqual(len(self.gaps(body, "crates", packages="-p studiolo-demo")), 1)
        self.assertEqual(len(self.gaps(body, "crates", packages="-p studiolo-missing")), 1)
        self.write("crates/demo/tests/probe.rs", body)
        text = 'STUDIOLO_INPUTS=(\n  crates\n)\ntar -cf - "${STUDIOLO_INPUTS[@]}"\n'
        self.assertEqual(len(manifest_dir_gaps(self.root, "scripts/package.sh", text)[1]), 1)


class RealManifestTests(unittest.TestCase):
    """真实仓库两份清单现状必须全绿；再把 scripts/ 复制进临时目录，从清单删掉 EDGES 里的被依赖方，必须红。"""

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        shutil.copytree(SCRIPTS, self.root / "scripts", ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))

    def test_both_manifests_carry_every_local_python_dependency(self):
        for script, edges in EDGES.items():
            with self.subTest(script=script):
                found, gaps = scan(ROOT, script)
                self.assertEqual(gaps, [], "打包清单漏了本地 Python 依赖，运行包里会 ModuleNotFoundError")
                for edge in edges:
                    self.assertIn(edge, found, "守卫没认出应有的依赖，解析逻辑失效")

    def test_dropping_a_known_dependency_from_either_manifest_is_red(self):
        for script, edges in EDGES.items():
            path = self.root / script
            original = path.read_text(encoding="utf-8")
            self.assertEqual(scan(self.root, script)[1], [], "复制出的副本在变异前就不绿")
            for importer, entry in edges:
                with self.subTest(entry=entry):
                    mutated, count = re.subn(rf"(?<!\S){re.escape(entry)}(?!\S)", "", original)
                    self.assertEqual(count, 1, f"{script} 里没有唯一一处 {entry}，变异没生效")
                    try:
                        path.write_text(mutated, encoding="utf-8")
                        gaps = scan(self.root, script)[1]
                        self.assertTrue(any(f"{importer} -> {entry}，" in gap for gap in gaps), gaps)
                    finally:
                        path.write_text(original, encoding="utf-8")


class SyntheticManifestTests(unittest.TestCase):
    """虚构的小仓库逐条钉住口径：标准库与第三方不算、递归、整包、各种按路径加载、读不懂就红。"""

    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / "scripts").mkdir()

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def gaps(self, *entries, block=None, script=None):
        block = "\n".join(f"  {entry}" for entry in entries) if block is None else block
        script = f'STUDIOLO_INPUTS=(\n{block}\n)\ntar -cf - "${{STUDIOLO_INPUTS[@]}}"\n' if script is None else script
        self.write("scripts/package.sh", "#!/usr/bin/env bash\n" + script)
        return scan(self.root, "scripts/package.sh")[1]

    def missing(self, importer, dependency):
        return f"scripts/package.sh: {importer} -> {dependency}，后者不在 STUDIOLO_INPUTS 里"

    def test_only_local_modules_count_and_dependencies_of_dependencies_are_followed(self):
        # json、os.path 是标准库，yaml、requests 是第三方：scripts/ 下没有同名文件，都不算。
        self.write("scripts/entry.py", "import json, os.path\nimport yaml\nfrom requests import get\n"
                   "from middle import run\n")
        self.write("scripts/middle.py", "import importlib.util\nfrom pathlib import Path\nimportlib.util."
                   "spec_from_file_location('leaf', Path(__file__).with_name('leaf-tool.py'))\n")
        self.write("scripts/leaf-tool.py", "print('虚构叶子脚本')\n")
        self.assertEqual(self.gaps("scripts/entry.py"), [self.missing("scripts/entry.py", "scripts/middle.py"),
                                                        self.missing("scripts/middle.py", "scripts/leaf-tool.py")])
        self.assertEqual(self.gaps("scripts/entry.py", "scripts/middle.py", "scripts/leaf-tool.py"), [])

    def test_package_import_requires_the_whole_package(self):
        self.write("scripts/entry.py", "from kit.store import Store\n")
        for name, text in [("__init__", ""), ("store", "from . import util\n"), ("util", "")]:
            self.write(f"scripts/kit/{name}.py", text)
        self.assertEqual(len(self.gaps("scripts/entry.py")), 3)
        self.assertEqual(self.gaps("scripts/entry.py", "scripts/kit"), [])

    def test_path_loaders_and_dynamic_imports_are_followed(self):
        prelude = ("import importlib, importlib.util, runpy\nfrom pathlib import Path\n"
                   "SCRIPTS = Path(__file__).parent\n")
        spec = "importlib.util.spec_from_file_location"
        bodies = {"keyword": f"{spec}('m', location=SCRIPTS / 'tool.py')\n",
                  "root-relative": f"{spec}('m', SCRIPTS.parent / 'scripts/tool.py')\n",
                  "source-loader": "import importlib.machinery as machinery\n"
                                   "machinery.SourceFileLoader('m', str(SCRIPTS / 'tool.py'))\n",
                  "run-path": "runpy.run_path(str(SCRIPTS / 'tool.py'))\n",
                  "helper": f"def load(name, filename):\n    return {spec}(name, SCRIPTS / filename)\n"
                            "load('m', 'tool.py')\n",
                  "import-module": "importlib.import_module('tool')\n"}
        self.write("scripts/tool.py", "print('虚构工具')\n")
        for label, body in bodies.items():
            with self.subTest(label=label):
                self.write("scripts/entry.py", prelude + body)
                self.assertEqual(self.gaps("scripts/entry.py"), [self.missing("scripts/entry.py", "scripts/tool.py")])
                self.assertEqual(self.gaps("scripts/entry.py", "scripts/tool.py"), [])

    def test_unknown_load_target_or_unparseable_or_absent_python_is_red(self):
        for body in ["import runpy, sys\nrunpy.run_path(sys.argv[1])\n",
                     # f-string 片段 "-tool.py" 不是完整文件名，不能当字面量放行。
                     "import importlib.util\nNAME = 'x'\n"
                     "importlib.util.spec_from_file_location('m', f'{NAME}-tool.py')\n",
                     "import importlib, sys\nimportlib.import_module(sys.argv[1])\n",
                     "def broken(:\n"]:
            with self.subTest(body=body):
                self.write("scripts/entry.py", body)
                gaps = self.gaps("scripts/entry.py")
                self.assertEqual(len(gaps), 1, gaps)
                self.assertTrue(gaps[0].startswith("scripts/package.sh: scripts/entry.py "), gaps)
        self.assertEqual(self.gaps("scripts/absent.py"),
                         ["scripts/package.sh: 清单里的 scripts/absent.py 不存在，核对不了它的依赖"])

    def test_manifest_syntax_the_guard_cannot_read_is_red(self):
        self.write("scripts/entry.py", "")
        self.assertEqual(self.gaps(block="  scripts/entry.py  # 行尾注释 scripts/*.py\n  # 整行注释"), [])
        for block in ['  "scripts/entry.py"', "  scripts/*.py", "  $EXTRA", "  scripts/{entry,other}.py",
                      "  ../scripts/entry.py", "  scripts/./entry.py", "  -v", "  scripts/entry.py)"]:
            with self.subTest(block=block):
                self.assertNotEqual(self.gaps(block=block), [])
        use = 'tar -cf - "${STUDIOLO_INPUTS[@]}"\n'
        for script in ["STUDIOLO_INPUTS=(scripts/entry.py)\n" + use,
                       "STUDIOLO_INPUTS=(\n  scripts/entry.py\n)\nSTUDIOLO_INPUTS+=(scripts/tool.py)\n" + use,
                       "STUDIOLO_INPUTS=(\n  scripts/entry.py\n)\nSTUDIOLO_INPUTS=(\n  scripts/tool.py\n)\n" + use,
                       "STUDIOLO_INPUTS=(\n  scripts/entry.py\n)\ntar -cf - ${STUDIOLO_INPUTS[*]}\n",
                       "STUDIOLO_INPUTS=(\n  scripts/entry.py\n" + use]:
            with self.subTest(script=script):
                self.assertNotEqual(self.gaps(script=script), [])


if __name__ == "__main__":
    unittest.main()
