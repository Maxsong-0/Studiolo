#!/usr/bin/env python3
"""Repository entry point for the Studiolo local Consumer companion."""
from local_client.__main__ import main
import sys

if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError):
        print("本机配置或连接不可用；请核对登记与私有文件权限。", file=sys.stderr)
        raise SystemExit(1)
