#!/usr/bin/env bash
# T-063：把已验收的运行包安装到服务器发布目录。只做校验、落盘与登记三件事。
# 用法：bash install-runtime-package.sh <已解压运行包 payload 的绝对路径> <发布名>
#
# 硬边界（与 deploy/agent 既有安装器一致，且比它更保守）：
#   * 绝不 systemctl start/stop/restart/reload、绝不 daemon-reload、绝不写 /etc；
#     需要重启才生效的改动只追加到待重启登记，由用户自己批量执行。
#   * 只以普通用户身份运行，拒绝 root；私有目录 0700、文件 0600、可执行 0700。
#   * 安装前逐项校验整包：条目类型、权限位、SHA256SUMS、生产程序与生产特征回执。
#   * 不接受任何秘密参数；秘密只经 systemd EnvironmentFile 由用户自行注入。
set -Eeuo pipefail
umask 077
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
unset CDPATH ENV BASH_ENV PYTHONPATH PYTHONHOME

# 发布根目录与待重启登记可由参数覆盖，便于在本机以虚构目录做契约测试。
STUDIOLO_RELEASES_ROOT="${STUDIOLO_RELEASES_ROOT:-$HOME/.local/share/studiolo-preview/releases}"
STUDIOLO_RESTART_REGISTRY="${STUDIOLO_RESTART_REGISTRY:-$HOME/.local/share/studiolo-preview/pending-restart.log}"

# BEGIN_LOOSE_MODES
# 任何组或其他用户可读、可写、可执行的条目都算越界。
# 逐位判定而不用 `-perm /0077`：GNU find 与 BSD find 对 `/` 与 `+` 的支持并不一致。
loose_entries() {
    find "$1" \( -perm -0040 -o -perm -0020 -o -perm -0010 \
        -o -perm -0004 -o -perm -0002 -o -perm -0001 \) -print -quit
}
writable_entries() {
    find "$1" \( -perm -0020 -o -perm -0002 \) -print -quit
}
# END_LOOSE_MODES

# BEGIN_PACKAGE_VERIFY
# 只依赖两个参数：$1 = 运行包 payload 目录，$2 = 发布名。任一项不符即返回非零，安装不继续。
verify_package() {
    local payload=$1 release=$2 product crate expected
    if [[ "$payload" != /* || ! -d "$payload" || -L "$payload" ]]; then
        printf '%s\n' '运行包路径必须是已存在的绝对目录，且不能是符号链接。' >&2
        return 1
    fi
    if [[ ! "$release" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$ ]]; then
        printf '%s\n' '发布名须以字母数字开头，仅含字母数字点下划线短横线，最多80字符。' >&2
        return 1
    fi
    if [[ -n "$(find "$payload" ! -type f ! -type d -print -quit)" ]]; then
        printf '%s\n' '运行包含有普通文件与目录之外的条目（如符号链接），拒绝安装。' >&2
        return 1
    fi
    if [[ -n "$(writable_entries "$payload")" ]]; then
        printf '%s\n' '运行包存在组或其他用户可写的条目，拒绝安装。' >&2
        return 1
    fi
    if [[ ! -f "$payload/SHA256SUMS" || ! -s "$payload/SHA256SUMS" ]]; then
        printf '%s\n' '运行包缺少整包摘要清单。' >&2
        return 1
    fi
    if ! (cd "$payload" && sha256sum --check --status SHA256SUMS); then
        printf '%s\n' '运行包摘要与清单不一致，拒绝安装。' >&2
        return 1
    fi
    for product in studiolo-local-server studiolo-auth-admin studiolo-maintain; do
        if [[ ! -f "$payload/bin/$product" || ! -x "$payload/bin/$product" \
            || -L "$payload/bin/$product" ]]; then
            printf '%s\n' '运行包缺少可执行的生产程序，拒绝安装。' >&2
            return 1
        fi
    done
    if [[ ! -f "$payload/product-features.txt" || ! -s "$payload/product-features.txt" ]]; then
        printf '%s\n' '运行包缺少生产程序特征回执，拒绝安装。' >&2
        return 1
    fi
    for product in studiolo-local-server studiolo-maintain studiolo-auth-admin; do
        if ! grep -q "^product:$product crate:" "$payload/product-features.txt"; then
            printf '%s\n' '生产特征回执缺少本次应发布的程序，拒绝安装。' >&2
            return 1
        fi
    done
    for crate in backup observability sync-transport host-config; do
        if ! grep -q "^crate:$crate features:" "$payload/product-features.txt"; then
            printf '%s\n' '生产程序未链接本次应随包发布的运行时 crate，拒绝安装。' >&2
            return 1
        fi
    done
    if ! grep -q '^crate:auth features:production$' "$payload/product-features.txt" \
        || grep -q 'test-support' "$payload/product-features.txt"; then
        printf '%s\n' '生产程序未使用生产身份特征，或混入了测试验证器特征，拒绝安装。' >&2
        return 1
    fi
    expected="/home/maxsong/.local/share/studiolo-preview/releases/$release/source"
    if [[ ! -f "$payload/expected-source-root" \
        || "$(< "$payload/expected-source-root")" != "$expected" ]]; then
        printf '%s\n' '运行包记录的源码位置与发布名不匹配，拒绝安装。' >&2
        return 1
    fi
    return 0
}
# END_PACKAGE_VERIFY

# BEGIN_PRIVATE_LAYOUT
# 只依赖三个参数：$1 = 发布根目录，$2 = 发布名，$3 = 运行包 payload 目录。
# 先落到同一文件系统的 .partial 暂存目录，权限收紧并复验通过后才整体改名就位。
install_private_tree() {
    local root=$1 release=$2 payload=$3 target staging
    target=$root/$release
    staging=$root/.$release.partial
    if [[ -L "$root" ]]; then
        printf '%s\n' '发布根目录不能是符号链接。' >&2
        return 1
    fi
    mkdir -p -- "$root"
    chmod 0700 "$root"
    if [[ ! -O "$root" ]]; then
        printf '%s\n' '发布根目录不属于当前用户，拒绝安装；不跨身份写入他人目录。' >&2
        return 1
    fi
    if [[ -e "$target" || -L "$target" ]]; then
        printf '%s\n' '同名发布目录已存在；发布目录不可变，请换用新发布名。' >&2
        return 1
    fi
    if [[ -e "$staging" || -L "$staging" ]]; then
        printf '%s\n' '上一次安装留下了未清理的暂存目录，请先人工确认后再安装。' >&2
        return 1
    fi
    mkdir -- "$staging"
    chmod 0700 "$staging"
    cp -R -- "$payload/." "$staging/"
    find "$staging" -type d -exec chmod 0700 {} +
    find "$staging" -type f -exec chmod 0600 {} +
    # 原生验收要求生产程序与随包测试程序可执行；其余文件一律保持 0600。
    for directory in bin tests; do
        if [[ -d "$staging/$directory" ]]; then
            find "$staging/$directory" -type f -exec chmod 0700 {} +
        fi
    done
    if [[ -n "$(find "$staging" ! -type f ! -type d -print -quit)" ]]; then
        printf '%s\n' '落盘后出现普通文件与目录之外的条目，拒绝就位。' >&2
        return 1
    fi
    if [[ -n "$(loose_entries "$staging")" ]]; then
        printf '%s\n' '落盘后仍有条目对组或其他用户可见，拒绝就位。' >&2
        return 1
    fi
    # 落盘后按随包清单复验一次字节：磁盘写满、复制中断或介质损坏都会让副本与
    # 源包不一致，而发布目录一旦就位即不可变。就位前发现，好过带着半截包验收。
    if ! (cd "$staging" && sha256sum --check --status SHA256SUMS); then
        printf '%s\n' '落盘副本与随包摘要清单不一致，拒绝就位。' >&2
        return 1
    fi
    mv -- "$staging" "$target"
    printf '%s\n' "$target"
    return 0
}
# END_PRIVATE_LAYOUT

# BEGIN_RESTART_REGISTER
# 只依赖参数：$1 = 登记文件，$2 = 发布名，$3.. = 需要用户手动执行的单元名。
# 本函数只追加登记，绝不调用 systemctl；共享服务何时重启由用户决定。
register_pending_restart() {
    local registry=$1 release=$2 stamp unit
    shift 2
    if [[ $# == 0 ]]; then
        printf '%s\n' '待重启登记必须至少写明一个单元名。' >&2
        return 1
    fi
    if [[ -L "$registry" ]]; then
        printf '%s\n' '待重启登记文件不能是符号链接。' >&2
        return 1
    fi
    mkdir -p -- "$(dirname -- "$registry")"
    chmod 0700 "$(dirname -- "$registry")"
    : >> "$registry"
    chmod 0600 "$registry"
    stamp=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
    for unit in "$@"; do
        if [[ ! "$unit" =~ ^[A-Za-z0-9][A-Za-z0-9._@-]{0,79}\.service$ ]]; then
            printf '%s\n' '待重启单元名不合法，拒绝登记。' >&2
            return 1
        fi
        printf '%s 待重启 %s 发布 %s\n' "$stamp" "$unit" "$release" >> "$registry"
    done
    return 0
}
# END_RESTART_REGISTER

main() {
    local payload release target
    if [[ $# != 2 ]]; then
        printf '%s\n' '用法：bash install-runtime-package.sh <运行包 payload 绝对路径> <发布名>' >&2
        return 2
    fi
    if [[ "$EUID" == 0 ]]; then
        printf '%s\n' '请以普通用户执行；本脚本不写系统目录，也不需要 root。' >&2
        return 2
    fi
    payload=$1
    release=$2
    verify_package "$payload" "$release"
    target=$(install_private_tree "$STUDIOLO_RELEASES_ROOT" "$release" "$payload")
    register_pending_restart "$STUDIOLO_RESTART_REGISTRY" "$release" \
        studiolo-preview.service studiolo-access.service
    printf '运行包已安装到私有目录：%s\n' "$target"
    printf '%s\n' '尚未启停任何服务。请先在该目录按 docs/m3/release-packaging.md 完成原生验收，'
    printf '%s\n' '再由你自己启用并重启对应的 systemd 用户服务；待重启项已登记到：'
    printf '%s\n' "$STUDIOLO_RESTART_REGISTRY"
    return 0
}

main "$@"
