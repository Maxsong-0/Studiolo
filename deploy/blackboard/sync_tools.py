"""Bridge 随包程序的显式输入与来源门禁；不写文件、不装配或启动服务。"""
import re

TOOLS = frozenset({'bin/studiolo-maintain', 'bin/studiolo-sync-launcher'})
SOURCE = 'sync_program_source_commit'


def _hex(value, length):
    """摘要和来源仅接受定长小写十六进制，类型错误同样失败关闭。"""
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{' + str(length) + '}', value)


def read_inputs(args, c):
    """双工具四字段全给才读取；必须同时显式给出本次人侧程序与来源。"""
    try:
        names = ('maintain', 'maintain_sha256', 'sync_launcher', 'sync_launcher_sha256')
        values = tuple(getattr(args, name, None) for name in names)
        if all(value is None for value in values):
            return {}
        c.require(all(value is not None for value in values))
        program = tuple(getattr(args, name, None) for name in
                        ('local_server', 'local_server_sha256', 'human_source_commit'))
        c.require(all(value is not None for value in program))
        c.require(_hex(program[2], 40))
        c.require(_hex(values[1], 64) and _hex(values[3], 64))
        result = {}
        for name, path, digest in (
                ('bin/studiolo-maintain', values[0], values[1]),
                ('bin/studiolo-sync-launcher', values[2], values[3])):
            raw = c.read(c.absolute(path), c.UID)
            c.require(raw.startswith(b'\x7fELF') and c.sha(raw) == digest)
            result[name] = raw
        return result
    except (OSError, TypeError, AttributeError, ValueError):
        # 不把输入路径、程序正文或底层读取诊断带进拒绝消息。
        raise ValueError('sync tool inputs rejected') from None


def validate_metadata(manifest, c):
    """双工具与来源字段共同存在或共同缺省；其他 manifest 约束由发行门禁负责。"""
    c.require(isinstance(manifest, dict))
    files = manifest.get('files')
    c.require(isinstance(files, dict))
    present = TOOLS.intersection(files)
    c.require(not present or present == TOOLS)
    if present:
        c.require(_hex(manifest.get(SOURCE), 40))
    else:
        c.require(SOURCE not in manifest)


def validate_transition(old, new, c):
    """只给成对新增或同源升级放行；沿用工具不能伪报来自本次人侧构建。"""
    validate_metadata(old, c)
    validate_metadata(new, c)
    old_tools = TOOLS.intersection(old['files'])
    new_tools = TOOLS.intersection(new['files'])
    c.require(not old_tools or new_tools)
    if new_tools and (not old_tools or old[SOURCE] != new[SOURCE]):
        c.require(new[SOURCE] == new.get('human_program_source_commit'))
        return TOOLS
    # 空变更集合要求上层逐字节沿用；这里不重复总文件闭集与实际权限校验。
    return frozenset()
