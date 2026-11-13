"""有界读取可信命令的标准输出；拒绝超限、失败或超时，不把 stderr 写进异常。"""
import os
import selectors
import subprocess
import time


def output(command, timeout, environment=None, limit=4096):
    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=environment) as child:
        collected = bytearray()
        deadline = time.monotonic() + timeout
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            try:
                while True:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError("command_timeout")
                    if not selector.select(remaining):
                        raise TimeoutError("command_timeout")
                    block = os.read(child.stdout.fileno(), min(4096, limit + 1 - len(collected)))
                    if not block:
                        break
                    collected.extend(block)
                    if len(collected) > limit:
                        raise ValueError("command_output_too_large")
                if child.wait(timeout=max(0, deadline - time.monotonic())) != 0:
                    raise ValueError("command_failed")
                return bytes(collected)
            except BaseException:
                child.kill()
                child.wait()
                raise
