"""从仓库根运行；使用自建短命本机进程验证虚构/自身白名单，绝不执行被读源码。"""

from collections import Counter
from hashlib import sha256
import json
from pathlib import Path
import socket
import subprocess
import time
import urllib.parse
import urllib.request


ROOT = Path(__file__).resolve().parents[2]
EXAMPLE = ROOT / "examples/m3-rust-call-links"
OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))
FIXTURE_PATHS = [
    "src/catalog/src/lib.rs",
    "src/search/src/lib.rs",
    "tools/src/lib.rs",
]
SELF_PATHS = [
    "crates/workbench/src/rust_names.rs",
    "crates/workbench/src/rust_extract.rs",
]


def fingerprint(root):
    return {
        str(path.relative_to(root)): sha256(path.read_bytes()).hexdigest()
        for path in root.rglob("*")
        if path.is_file()
    }


def batch_id(root, paths):
    digest = sha256(b"studiolo-rust-call-links-v1\0")
    digest.update(len(paths).to_bytes(8, "big"))
    for path in sorted(paths):
        for value in (path.encode(), (root / path).read_bytes()):
            digest.update(len(value).to_bytes(8, "big"))
            digest.update(value)
    return digest.hexdigest()


def check_graph(root, paths, observation):
    assert observation["status"] == "ready", observation
    graph, links = observation["graph"], observation["links"]
    assert links["version"] == 1
    assert links["batch_id"] == batch_id(root, paths)
    assert [item["path"] for item in graph["files"]] == sorted(paths)
    contexts = {item["id"]: item for item in graph["contexts"]}
    associations = links["associations"]
    assert [item["call_id"] for item in associations] == [
        item["id"] for item in graph["calls"]
    ]
    for field in ("contexts", "calls"):
        for item in graph[field]:
            location = item["source"]
            raw = (root / location["path"]).read_bytes()
            start = 0
            if not item["id"].endswith(":root"):
                _, _, start, end = item["id"].split(":")
                start, end = int(start), int(end)
                assert 0 <= start < end <= len(raw)
            prefix = raw[:start]
            assert location["line"] == prefix.count(b"\n") + 1
            assert location["column"] == len(prefix.rsplit(b"\n", 1)[-1]) + 1
    reasons = {
        "callee_form", "context", "opaque_scope", "shadowed_name",
        "ambiguous_name", "generic_target", "not_found",
    }
    incoming = Counter()
    for call, link in zip(graph["calls"], associations):
        if link["status"] == "linked":
            assert link["reason"] is None
            target = contexts[link["target"]]
            assert target["kind"] == "function"
            assert target["source"]["path"] == call["source"]["path"]
            incoming[target["id"]] += 1
        else:
            assert link["status"] == "unresolved"
            assert link["target"] is None and link["reason"] in reasons
    return graph, associations, contexts, incoming


def capture(repository, paths):
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    command = [
        str(ROOT / "target/debug/studiolo-local-server"),
        "--vault", str(EXAMPLE / "vault"),
        "--assets", str(ROOT / "web/dist"),
        "--port", str(port),
        "--project-repository", f"项目/示例工坊={repository}",
        "--rust-call-sites", json.dumps(paths),
    ]
    process = subprocess.Popen(
        command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    try:
        query = urllib.parse.urlencode({"project": "项目/示例工坊", "generation": 1})
        url = f"http://127.0.0.1:{port}/api/project-operations?{query}"
        for attempt in range(100):
            try:
                with OPENER.open(url, timeout=8) as response:
                    data = json.load(response)
                break
            except OSError:
                if process.poll() is not None or attempt == 99:
                    raise
                time.sleep(0.05)
        assert data["cargo_dependencies"]["status"] == "disabled"
        return check_graph(repository, paths, data["rust_call_sites"])
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def main():
    before = fingerprint(EXAMPLE)
    graph, links, contexts, incoming = capture(EXAMPLE / "repository", FIXTURE_PATHS)
    catalog = "src/catalog/src/lib.rs"
    collect = next(
        item for item in contexts.values()
        if item["name"] == "collect" and item["source"] == {
            "path": catalog, "line": 15, "column": 1,
        }
    )
    assert incoming[collect["id"]] == 4
    unused = next(item for item in contexts.values() if item["name"] == "unused")
    assert incoming[unused["id"]] == 0
    seen_reasons = {item["reason"] for item in links}
    assert {"callee_form", "opaque_scope", "shadowed_name", "generic_target", "not_found"} <= seen_reasons
    print("fixture:", len(graph["contexts"]), "contexts,", len(links), "calls,",
          sum(incoming.values()), "linked; collect has 4 references, unused has 0")

    self_before = {path: (ROOT / path).read_bytes() for path in SELF_PATHS}
    graph, links, contexts, incoming = capture(ROOT, SELF_PATHS)
    targets = {contexts[target]["name"] for target in incoming}
    assert {"identifier", "path"} <= targets, targets
    print("self:", len(graph["contexts"]), "contexts,", len(links), "calls,",
          sum(incoming.values()), "linked; identifier/path definitions verified")
    assert self_before == {path: (ROOT / path).read_bytes() for path in SELF_PATHS}
    assert before == fingerprint(EXAMPLE)
    print("All batch hashes, references, byte spans, line/columns and read-only fingerprints passed.")


if __name__ == "__main__":
    main()
