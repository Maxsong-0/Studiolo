# T-094 因子 portable 密文管线

本工具只迁移调用方指定的 `totp-next` systemd 凭据内的 40 字节小写十六进制因子。
它不读 Auth，不检查这个因子是否已 enrolled，也不证明它与快照、恢复账本、设备或用户绑定。
恢复后的 Auth 绑定必须由另行实现的 `Totp::attach` 完成。格式测试通过不等于完整灾备通过。

## 可信工具组与命令

运行时工具组为 `portable_factor.py`、同目录 `portable_factor_security.py`、独立编译的
`factor-pipe`，以及受信任的 `/usr/bin/systemd-creds` 和 `age`。不得从待解密包选择这些工具。
入口从自身固定 root 目录读取配套模块，不搜索当前目录、用户站点或环境导入路径。

仅支持 Linux、真实及有效 UID 都为 0、Python `-I -B -S`。工具文件须 root 所有、普通单链接、
不可组/其他写，二进制须 owner 可执行。所有路径必须绝对、规范、NFC、无控制字符、无符号链接；
所有祖先须 root 所有且不可组/其他写，root sticky 中间祖先允许作为临时目录。
输出父目录额外要求精确 `0700`，已有目标始终拒绝。

```sh
python3 -I -B -S /root/trusted/portable_factor.py export \
  --credential /root/private/totp-next \
  --recipient age1... \
  --output /root/private/portable-factor.age \
  --filter-bin /root/trusted/factor-pipe \
  --age-bin /root/trusted/age \
  --host-key /root/private/source-host-key

python3 -I -B -S /root/trusted/portable_factor.py import \
  --output /root/private/restored-totp-next \
  --filter-bin /root/trusted/factor-pipe \
  --host-key /root/private/target-host-key
```

第二条命令的 stdin 必须由受信任解密进程通过 FIFO 管道提供；不得在终端粘贴种子。
上例只展示参数，不能直接在终端执行 import 来输入秘密。调用方先对不可变的完整 age 密文
认证到 `/dev/null`，成功后再第二次解密并连接 import，同时核对两个子进程的退出码。
import 本身只验证种子格式，不能知道上游 age 的最终认证或退出状态。

export 的 `--age-bin` 默认 `/usr/bin/age`。export 的 `--host-key` 可省略，此时先校验
`/var/lib/systemd/credential.secret` 已存在；不会为不存在的默认密钥启动 systemd-creds。
import 强制显式 `--host-key`，禁止隐式改变共享主机默认密钥。import 密钥路径须位于输出父目录
之内的私有目录树，其直接父目录必须 `0700`；可使用本机已有密钥或在本次新建。

## 文件和进程边界

输入凭据须 root `0600`、普通单链接、1–65536 字节。已有 host-key 须 root `0400`、普通单链接、
1–16384 字节；Python 只核对元数据，不读取密钥内容。发布前再次检查工具、凭据及已有 key
描述符和路径绑定；新 key 在成功加密后也必须满足上述约束。

`SYSTEMD_CREDENTIAL_SECRET` 只传给 systemd-creds 子进程，值仅为路径，不含密钥。
要求 systemd 版本至少 259；[systemd v259 源码](https://raw.githubusercontent.com/systemd/systemd/v259/src/shared/creds-util.c)
说明此路径对应的主机密钥还绑定 machine-id。已有异机 key 可能被 systemd 删除重建，因此只允许
操作方提供本机新建或本机复用的私有 key；不要把主机 key 随 portable 因子迁移。
包装器的事后绑定检查能拒绝这种变更后的密文发布，但不读 key 就不能提前证明其 machine-id。

明文只经过 `systemd-creds → factor-pipe → age` 或 `stdin → factor-pipe → systemd-creds`。
Python 不读取、缓存、`communicate` 或放入 argv/env 中间种子流；其唯一捕获的产品子进程输出
是无凭据参数的 `systemd-creds --version` 元数据。所有业务子进程 stderr 封闭为 `/dev/null`。

过滤器无参数，stdin/stdout 都须 FIFO 非 TTY。它将 core 软硬限额置零、关闭并回读 dumpable、
设置并回读 no_new_privs，`mlockall(CURRENT|FUTURE|ONFAULT)` 失败即拒绝，15 秒闹钟覆盖等待。
至 EOF 最多读取 41 字节，只有恰好 40 字节小写 hex 才输出；短写继续写完，退出前 volatile
清零 41 字节。SIGKILL/机器掉电无法执行用户态清零，锁页和禁 core 仍是独立边界。

Python 同样关闭 core/dumpable，子进程组只由本次管线创建。管线最多等待 30 秒，失败只 kill/reap
本次直接子进程。极端不可中断内核等待不保证三秒内回收，绝不因此发布成功。
密文先写本次 `O_EXCL|O_NOFOLLOW` 的 `0600` partial，全部子进程零退出、校验、fsync 后才
通过不覆盖 hard-link 发布并 fsync 目录。清理只碰本次 partial；已有目标从不覆盖。
若发布后的目录 fsync 失败，最终名字可能已存在，调用方须核对状态，不能把失败当作肯定未发布。
失败期间 systemd 可能已创建本次私有 host-key；它不会被包装器删除或用于共享默认位置。

正常 stdout 仅 `portable_factor_complete`，失败 stderr 仅 `portable_factor_failed`。
过滤器失败仅 `factor_pipe_failed`；不输出输入、工具错误正文、凭据或路径。

## 分层验证与复现

本地仅做 AST；下列 Linux 命令必须由主线安排在隔离测试目录串行执行。不要自行提权、安装依赖、
从生产复制凭据或复用真实 Auth。`CASE` 必须是主线创建并接管的 root `0700` 私有目录，祖先满足
上述限制；其中工具和源码也由主线复制为 root 普通文件。完整回路的私有 host-key 均随虚构 case 清理。

```sh
cc -std=c11 -Wall -Wextra -Werror -O2 "$CASE/factor_pipe.c" -o "$CASE/factor-pipe"
python3 -I -B -S "$CASE/test_portable_factor.py" \
  --filter-bin "$CASE/factor-pipe" --filter-source "$CASE/factor_pipe.c" --root-flow -v
python3 -I -B -S "$CASE/test_portable_factor_crypto.py" \
  --filter-bin "$CASE/factor-pipe" --age-bin "$CASE/age" \
  --age-keygen "$CASE/age-keygen" --work-parent "$CASE" -v
```

第一组覆盖过滤器 40/39/41 字节、换行、大小写/非 hex、EOF、闹钟、非 FIFO、锁页失败、
短写/零写、下游关闭，以及模拟管线子进程失败/超时回收、目标存在、链接/模式、版本和固定错误。
不带 `--root-flow` 可以仅运行过滤器测试，其余测试明确 skip；过滤器成功仍要求足够锁页额度。
第二组使用真实 systemd/age，仅生成虚构种子和本次 age 身份、两个本机独立 host-key；验证导出/
导入、原凭据不能直接使用另一把 key 解密、恢复结果相等、截断 age 密文拒绝、错误输入不发布。
测试中的正常明文只在测试进程内做布尔比较，不打印；生成的 age 身份文件不读取到 Python。
