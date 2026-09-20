#!/usr/bin/env python3
"""旧 API 凭据入口已停用：本测试任务仅允许官方订阅额度。"""
import sys


def main():
    print("此 API 密钥录入命令已作废：仅允许订阅额度；未读取凭据、未写文件、未启动服务。", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
