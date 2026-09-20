"""Durable notification receipts, never a second task authority or prompt store."""
from contextlib import contextmanager
import os
from pathlib import Path
import re
import sqlite3
import stat
import time
import uuid

from .mcp import McpError, _private_open, _private_parent


STATES = frozenset({"pending", "dispatching", "sent", "uncertain", "cancelled", "completed"})
RESULT_DETAILS = frozenset({"", *STATES, "restart_uncertain", "transport_error", "dispatch_failed", "user_cancelled"})
_TRANSITIONS = {
    "pending": {"dispatching", "cancelled"},
    "dispatching": {"sent", "uncertain"},
    "sent": {"completed", "cancelled"},
    "uncertain": {"completed", "cancelled"},
    "cancelled": set(),
    "completed": set(),
}


def _identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,159}", value):
        raise ValueError("invalid route identifier")
    return value


class Store:
    """One connection per transaction permits concurrent HTTP request threads.

    The daemon must hold its single-instance lock before calling
    recover_dispatching(). Merely constructing a Store never interrupts another
    live sender. Neither uncertain nor sent notifications are retried.
    """

    def __init__(self, path):
        self.path = _private_parent(Path(path))
        os.close(_private_open(self.path, os.O_RDWR, create=True))
        with self._connection() as db:
            db.executescript("""
                CREATE TABLE IF NOT EXISTS notifications (
                    id TEXT PRIMARY KEY,
                    request_id TEXT UNIQUE NOT NULL,
                    client_id TEXT NOT NULL,
                    task_id TEXT NOT NULL,
                    state TEXT NOT NULL CHECK(state IN
                        ('pending','dispatching','sent','uncertain','cancelled','completed')),
                    detail TEXT NOT NULL DEFAULT '',
                    created_ms INTEGER NOT NULL,
                    updated_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS notifications_pending
                    ON notifications(state, client_id, created_ms);
            """)

    @contextmanager
    def _connection(self):
        # A private parent plus inode checks protects SQLite's path-based open.
        # DELETE journaling avoids persistent WAL/SHM files with different ACLs.
        os.close(_private_open(self.path, os.O_RDWR))
        for suffix in ("-journal", "-wal", "-shm"):
            sidecar = self.path.with_name(self.path.name + suffix)
            try:
                fd = os.open(sidecar, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
            except FileNotFoundError:
                continue  # Another completed transaction may remove its journal.
            except OSError:
                raise McpError("unsafe_file") from None
            try:
                info = os.fstat(fd)
                if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                        or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1):
                    raise McpError("unsafe_file")
            finally:
                os.close(fd)
        db = sqlite3.connect(self.path, timeout=20, isolation_level=None)
        db.row_factory = sqlite3.Row
        try:
            db.execute("PRAGMA busy_timeout=20000")
            db.execute("PRAGMA journal_mode=DELETE")
            db.execute("PRAGMA synchronous=FULL")
            yield db
        finally:
            db.close()

    def enqueue(self, request_id, client_id, task_id):
        request_id, client_id, task_id = map(_identifier, (request_id, client_id, task_id))
        now = time.time_ns() // 1_000_000
        with self._connection() as db:
            db.execute("BEGIN IMMEDIATE")
            try:
                old = db.execute("SELECT * FROM notifications WHERE request_id = ?", (request_id,)).fetchone()
                if old is not None:
                    if old["client_id"] != client_id or old["task_id"] != task_id:
                        raise ValueError("idempotency conflict")
                    db.execute("COMMIT")
                    return dict(old)
                item_id = uuid.uuid4().hex
                db.execute("""INSERT INTO notifications
                    (id, request_id, client_id, task_id, state, created_ms, updated_ms)
                    VALUES (?, ?, ?, ?, 'pending', ?, ?)""",
                           (item_id, request_id, client_id, task_id, now, now))
                result = dict(db.execute("SELECT * FROM notifications WHERE id = ?", (item_id,)).fetchone())
                db.execute("COMMIT")
                return result
            except BaseException:
                db.execute("ROLLBACK")
                raise

    def list(self, project=None):
        """Project authorization/filtering belongs to the daemon's registry."""
        with self._connection() as db:
            return [dict(row) for row in db.execute("SELECT * FROM notifications ORDER BY created_ms, id")]

    def get(self, item_id):
        _identifier(item_id)
        with self._connection() as db:
            row = db.execute("SELECT * FROM notifications WHERE id = ?", (item_id,)).fetchone()
            return dict(row) if row else None

    def pending(self, client_id=None):
        with self._connection() as db:
            if client_id is None:
                rows = db.execute("SELECT * FROM notifications WHERE state = 'pending' ORDER BY created_ms, id")
            else:
                _identifier(client_id)
                rows = db.execute("SELECT * FROM notifications WHERE state = 'pending' AND client_id = ? ORDER BY created_ms, id", (client_id,))
            return [dict(row) for row in rows]

    def transition(self, item_id, expected_state, new_state, detail=""):
        _identifier(item_id)
        if (not isinstance(expected_state, str) or not isinstance(new_state, str)
                or expected_state not in STATES or new_state not in _TRANSITIONS[expected_state]):
            raise ValueError("invalid notification transition")
        if not isinstance(detail, str) or detail not in RESULT_DETAILS:
            raise ValueError("invalid notification result code")
        with self._connection() as db:
            cursor = db.execute("""UPDATE notifications SET state = ?, detail = ?, updated_ms = ?
                WHERE id = ? AND state = ?""", (new_state, detail, time.time_ns() // 1_000_000, item_id, expected_state))
            return cursor.rowcount == 1

    def recover_dispatching(self):
        """Called once at daemon startup, under the daemon's exclusive lock."""
        with self._connection() as db:
            result = db.execute("""UPDATE notifications SET state = 'uncertain',
                detail = 'restart_uncertain', updated_ms = ? WHERE state = 'dispatching'""",
                                (time.time_ns() // 1_000_000,))
            return result.rowcount
