"""核对既有 Broker 与黑板目录身份；不打开写者、不初始化目录。"""
from pathlib import Path
import uuid

# 黑板数据目录只允许出现控制库本体及其 WAL/SHM；停服时 SQLite 可能收掉后两个。
BOARD_FILES = {'control.sqlite3', 'control.sqlite3-wal', 'control.sqlite3-shm'}


def board_files(c, board):
    """起点是已在跑黑板的发行：目录里必须有控制库、不得有别的东西，每个文件私有且单链接。

    r8→r2 那一次是首建，当时要求这个目录为空；r2 起黑板已接单，库里是真实的认领与回执，
    因此「为空」会把每一次后续发行挡在预检，而「随便有什么都行」又会放过被篡改的目录。
    本函数在预检与装单元前各跑一次（binding 被调用两次），停服后 WAL/SHM 消失同样通过。
    """
    names = {path.name for path in board.iterdir()}
    c.require('control.sqlite3' in names and names <= BOARD_FILES)
    for name in sorted(names):
        c.inspect(board / name, c.UID, 0o600)


def binding(c, a):
    """逐项核验 Broker 的不可变配对；不打开写者，不补写任何目录。"""
    vault, writer, board = [Path(p) for p in (a.VAULT, a.WRITER, a.BOARD)]
    roots = []
    for path in (vault, writer, board):
        roots.append(c.inspect(path, c.UID, 0o700, True))
    c.require(len({(s.st_dev, s.st_ino) for s in roots}) == 3)
    for left in (vault, writer, board):
        for right in (vault, writer, board):
            c.require(left == right or not left.is_relative_to(right))
    board_files(c, board)
    control = vault / '.workbench'
    left = c.document(c.read(control / 'broker-binding.json', c.UID, 0o600))
    right = c.document(c.read(writer / 'broker-binding.json', c.UID, 0o600))
    fields = {'format_version', 'pair_id', 'vault_uuid', 'canonical', 'data', 'control',
              'marker', 'staging', 'history', 'idempotency', 'snapshots'}
    c.require(isinstance(left, dict) and set(left) == fields and left == right)
    c.require(type(left['format_version']) is int and left['format_version'] == 1)
    for name in ('pair_id', 'vault_uuid'):
        c.require(str(uuid.UUID(left[name])) == left[name])
    paths = {'canonical': vault, 'data': writer, 'control': control,
             'marker': control / 'vault.json', 'staging': control / 'staging',
             **{name: writer / name for name in ('history', 'idempotency', 'snapshots')}}
    for key, path in paths.items():
        s = c.inspect(path, c.UID, 0o600 if key == 'marker' else 0o700, key != 'marker')
        c.require(left[key] == {'dev': s.st_dev, 'ino': s.st_ino})
        c.require(all(type(v) is int for v in left[key].values()))
    marker = c.document(c.read(control / 'vault.json', c.UID, 0o600))
    c.require(marker == {'format_version': 1, 'vault_uuid': left['vault_uuid']})
    c.read(writer / 'writer-state.json', c.UID, 0o600)
    return left


