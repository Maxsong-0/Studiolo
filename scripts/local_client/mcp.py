"""Small, serialized MCP HTTP transport for a local Studiolo Consumer.

Only Studiolo's bearer and MCP session are handled here. Requests, credentials,
and upstream error bodies are deliberately never logged. Mutations are never
retried: a lost reply has an unknown outcome and must be reconciled by reading
the authoritative blackboard.
"""
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request


MAX_BYTES = 1024 * 1024
PROTOCOL = "2025-11-25"
_LOCKS = {}
_LOCKS_GUARD = threading.Lock()


class McpError(RuntimeError):
    """Safe transport failure, with a fixed code and no upstream text."""

    def __init__(self, code):
        self.code = code
        super().__init__("Studiolo MCP: " + code)


def _private_parent(path):
    """Create a private immediate parent; never chmod an existing directory."""
    path = Path(path).absolute()
    try:
        # Checking before mkdir avoids following a pre-existing symlink parent.
        for parent in path.parents:
            if parent.is_symlink():
                raise McpError("unsafe_file")
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        info = path.parent.lstat()
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
                or stat.S_IMODE(info.st_mode) & 0o077):
            raise McpError("unsafe_file")
    except OSError:
        raise McpError("unsafe_file") from None
    return path


def _private_open(path, flags, create=False):
    """Check the opened inode, including hard links, rather than just its path."""
    path = _private_parent(path)
    try:
        fd = os.open(path, flags | os.O_NOFOLLOW | os.O_CLOEXEC
                     | (os.O_CREAT if create else 0), 0o600)
        info = os.fstat(fd)
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1):
            os.close(fd)
            raise McpError("unsafe_file")
        return fd
    except OSError:
        raise McpError("unsafe_file") from None


def _read_private(path, limit=MAX_BYTES):
    fd = _private_open(path, os.O_RDONLY)
    try:
        with os.fdopen(fd, "rb") as stream:
            value = stream.read(limit + 1)
        if len(value) > limit:
            raise McpError("file_too_large")
        return value
    except OSError:
        raise McpError("unsafe_file") from None


def _write_private(path, value):
    path = _private_parent(path)
    temporary = None
    try:
        # Refuse a hostile existing target; os.replace must not hide the issue.
        if os.path.lexists(path):
            os.close(_private_open(path, os.O_RDONLY))
        fd, temporary = tempfile.mkstemp(prefix=".session-", dir=path.parent)
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(value, stream, ensure_ascii=False, separators=(",", ":"))
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        temporary = None
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except (OSError, ValueError, TypeError):
        raise McpError("state_write_failed") from None
    finally:
        if temporary is not None:
            try:
                os.unlink(temporary)
            except OSError:
                pass


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def _response_message(raw, content_type, request_id):
    """Select this call's response; ignore SSE notifications and empty events."""
    try:
        text = raw.decode("utf-8")
        if content_type.split(";", 1)[0].strip().lower() == "text/event-stream":
            values = []
            data = []
            for line in text.replace("\r\n", "\n").replace("\r", "\n").split("\n") + [""]:
                if line == "":
                    if data:
                        payload = "\n".join(data)
                        if payload.strip():
                            values.append(json.loads(payload))
                    data = []
                elif line.startswith("data:"):
                    value = line[5:]
                    data.append(value[1:] if value.startswith(" ") else value)
                elif line == "data":
                    data.append("")
        elif content_type.split(";", 1)[0].strip().lower() == "application/json":
            values = [json.loads(text)]
        else:
            raise McpError("invalid_response")
        responses = [value for value in values if isinstance(value, dict)
                     and "id" in value and type(value["id"]) is type(request_id)
                     and value["id"] == request_id]
        if len(responses) != 1:
            raise McpError("response_id_mismatch")
        response = responses[0]
        if response.get("jsonrpc") != "2.0":
            raise McpError("invalid_response")
        if "error" in response:
            raise McpError("rpc_error")
        if "result" not in response:
            raise McpError("invalid_response")
        return response["result"]
    except (UnicodeError, ValueError, TypeError):
        raise McpError("invalid_response") from None


class McpClient:
    def __init__(self, endpoint: str, token_file: Path, state_file: Path):
        try:
            parsed = urllib.parse.urlsplit(endpoint)
            valid = (parsed.scheme == "https" and bool(parsed.hostname)
                     or parsed.scheme == "http" and parsed.hostname == "127.0.0.1")
            if (not valid or parsed.username is not None or parsed.password is not None
                    or parsed.fragment or parsed.query or not parsed.path
                    or any(ord(char) < 33 for char in endpoint)):
                raise ValueError()
            _ = parsed.port
        except (TypeError, ValueError):
            raise McpError("invalid_endpoint") from None
        self.endpoint = endpoint
        self.token_file = _private_parent(token_file)
        self.state_file = _private_parent(state_file)
        if self.token_file == self.state_file:
            raise McpError("unsafe_file")
        self.lock_file = self.state_file.with_name(self.state_file.name + ".lock")
        if self.token_file == self.lock_file:
            raise McpError("unsafe_file")
        with _LOCKS_GUARD:
            self._lock = _LOCKS.setdefault(str(self.state_file), threading.RLock())
        self._opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), _NoRedirect())

    @contextmanager
    def _serialized(self):
        with self._lock:
            fd = _private_open(self.lock_file, os.O_RDWR, create=True)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX)
                yield
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)
                os.close(fd)

    def _load(self, token):
        if os.path.lexists(self.state_file):
            try:
                state = json.loads(_read_private(self.state_file))
            except (ValueError, UnicodeError):
                raise McpError("invalid_state") from None
            if (not isinstance(state, dict) or type(state.get("counter")) is not int
                    or not 0 <= state["counter"] < 2 ** 53
                    or not isinstance(state.get("session_id", ""), str)
                    or len(state.get("session_id", "")) > 256
                    or any(not 33 <= ord(char) <= 126 for char in state.get("session_id", ""))):
                raise McpError("invalid_state")
        else:
            state = {"counter": 0}
        fingerprint = hashlib.sha256(token.encode()).hexdigest()
        if state.get("endpoint") != self.endpoint or state.get("token_sha256") != fingerprint:
            state.pop("session_id", None)
        state["endpoint"] = self.endpoint
        state["token_sha256"] = fingerprint
        return state

    def _request(self, state, token, method, params=None, notification=False):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if not notification:
            state["counter"] += 1
            message["id"] = state["counter"]
            # Reserve the ID durably before a possibly committing request.
            _write_private(self.state_file, state)
        try:
            body = json.dumps(message, ensure_ascii=False, allow_nan=False).encode()
        except (ValueError, TypeError, UnicodeError):
            raise McpError("invalid_request") from None
        if len(body) > MAX_BYTES:
            raise McpError("request_too_large")
        headers = {"Authorization": "Bearer " + token,
                   "Accept": "application/json, text/event-stream",
                   "Content-Type": "application/json", "MCP-Protocol-Version": PROTOCOL}
        if state.get("session_id"):
            headers["Mcp-Session-Id"] = state["session_id"]
        request = urllib.request.Request(self.endpoint, body, headers, method="POST")
        try:
            try:
                response = self._opener.open(request, timeout=20)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                status = response.status
                if status in {401, 403, 404}:
                    state.pop("session_id", None)
                    _write_private(self.state_file, state)
                    raise McpError("session_invalid")
                if 300 <= status < 400:
                    raise McpError("redirect_refused")
                if notification:
                    if status != 202:
                        raise McpError("notification_rejected")
                    return None, None
                if status != 200:
                    raise McpError("http_error")
                raw = response.read(MAX_BYTES + 1)
                if len(raw) > MAX_BYTES:
                    raise McpError("response_too_large")
                returned_session = response.headers.get("Mcp-Session-Id")
                if returned_session is not None and (
                        not returned_session or len(returned_session) > 256
                        or any(not 33 <= ord(char) <= 126 for char in returned_session)):
                    raise McpError("invalid_response")
                result = _response_message(raw, response.headers.get("Content-Type", ""), message["id"])
                if isinstance(result, dict) and result.get("isError"):
                    raise McpError("tool_error")
                return result, returned_session
        except (OSError, ValueError, urllib.error.URLError):
            raise McpError("transport_error") from None

    def call(self, method, params=None):
        if (not isinstance(method, str) or not method or len(method) > 128
                or method == "initialize" or method.startswith("notifications/")):
            raise McpError("invalid_method")
        with self._serialized():
            try:
                token = _read_private(self.token_file, 16 * 1024).decode().strip()
            except UnicodeError:
                raise McpError("invalid_token") from None
            if not token or any(not 33 <= ord(char) <= 126 for char in token):
                raise McpError("invalid_token")
            state = self._load(token)
            if not state.get("session_id"):
                result, session = self._request(state, token, "initialize", {
                    "protocolVersion": PROTOCOL, "capabilities": {},
                    "clientInfo": {"name": "studiolo-local", "version": "1.0.0"},
                })
                if (not isinstance(result, dict) or result.get("protocolVersion") != PROTOCOL
                        or session is None):
                    raise McpError("initialize_rejected")
                state["session_id"] = session
                try:
                    self._request(state, token, "notifications/initialized", notification=True)
                except McpError:
                    state.pop("session_id", None)
                    _write_private(self.state_file, state)
                    raise
                _write_private(self.state_file, state)
            result, _ = self._request(state, token, method, params)
            return result

    def tools_list(self):
        result = self.call("tools/list", {})
        if not isinstance(result, dict) or not isinstance(result.get("tools"), list):
            raise McpError("invalid_response")
        return result

    def board(self, project, operation, request):
        result = self.call("tools/call", {"name": "board", "arguments": {
            "project": project, "operation": operation, "request": request,
        }})
        try:
            contents = result["content"]
            if not isinstance(contents, list) or len(contents) != 1 or contents[0].get("type") != "text":
                raise ValueError()
            return json.loads(contents[0]["text"])
        except (KeyError, IndexError, TypeError, ValueError, AttributeError):
            raise McpError("invalid_board_response") from None
