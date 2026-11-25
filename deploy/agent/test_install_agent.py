#!/usr/bin/env python3
"""t237-install-agent.sh 的回归：替身命令、临时目录，不碰真的 systemd、单元和发布目录。

    python3 -I -B deploy/agent/test_install_agent.py             用例（ci 套跑这个）
    python3 -I -B deploy/agent/test_install_agent.py --mutants   再做改错核对：改坏脚本一处，指定的用例必须红
"""

from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import signal
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from install_cases.bench import SCRIPT, TEXT
from install_cases.cases import CASES
from install_cases.mutants import mutants

WORKERS = 8


def judged(name, judge, text):
    """(名字, 过没过, 出的错)。用例自己抛的异常另算，不当成「拦住了」。"""
    try:
        return name, bool(judge(text)), ""
    except Exception as error:
        return name, False, repr(error)[:300]


@unittest.skipUnless(sys.platform == "linux", "要 setsid、flock 和 GNU 的 tail、stat")
class InstallAgentTests(unittest.TestCase):
    def test_syntax(self):
        subprocess.run(["bash", "-n", str(SCRIPT)], check=True)

    def test_cases(self):
        self.assertEqual(len(dict(CASES)), len(CASES))
        with ThreadPoolExecutor(WORKERS) as pool:
            outcomes = list(pool.map(lambda case: judged(*case, TEXT), CASES))
        for name, passed, error in outcomes:
            with self.subTest(name):
                self.assertTrue(passed, error)

    def test_mutants_match_the_script(self):
        names = dict(CASES)
        for mutant, (_, case) in mutants().items():
            with self.subTest(mutant):
                self.assertTrue(case is None or case in names)


def check_mutants():
    cases = dict(CASES)
    found = mutants()
    real = {name: (text, case) for name, (text, case) in found.items() if case is not None}
    with ThreadPoolExecutor(WORKERS) as pool:
        outcomes = list(pool.map(lambda item: judged(item[0], cases[item[1][1]], item[1][0]), real.items()))
    missed = [(name, real[name][1], error or "存活") for name, passed, error in outcomes if passed or error]
    for name, case, why in missed:
        print(f"没拦住：{name}（{case}）{why}")
    print(f"改错 {len(found)} 处，等价 {len(found) - len(real)}，没拦住 {len(missed)}")
    return not missed


if __name__ == "__main__":
    # 从后台起的进程会忽略这几个信号，子进程照着忽略，「收到信号」的用例就测不到了。
    for name in (signal.SIGHUP, signal.SIGINT, signal.SIGQUIT):
        if signal.getsignal(name) is signal.SIG_IGN:
            signal.signal(name, signal.SIG_DFL)
    wants_mutants = "--mutants" in sys.argv
    program = unittest.main(argv=[argument for argument in sys.argv if argument != "--mutants"], exit=False)
    ok = program.result.wasSuccessful() and (not wants_mutants or check_mutants())
    sys.exit(0 if ok else 1)
