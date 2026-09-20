#define _GNU_SOURCE
#include <errno.h>
#include <signal.h>
#include <stddef.h>
#include <sys/mman.h>
#include <sys/prctl.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <unistd.h>

/* 种子只在本进程的有界、已锁页缓冲中暂存；任何输出前先收齐并验证 EOF。 */
static volatile unsigned char secret[41];
static const char error_message[] = "factor_pipe_failed\n";

static void clear_secret(void) {
    for (size_t i = 0; i < sizeof(secret); ++i)
        secret[i] = 0;
}

static void fail(int signal_number) {
    (void)signal_number;
    clear_secret();
    ssize_t written = write(STDERR_FILENO, error_message, sizeof(error_message) - 1);
    (void)written;
    _exit(1);
}

static void require(int condition) {
    if (!condition)
        fail(0);
}

static int pipe_only(int fd) {
    struct stat info;
    return fstat(fd, &info) == 0 && S_ISFIFO(info.st_mode) && !isatty(fd);
}

int main(int argc, char **argv) {
    (void)argv;
    struct rlimit limit = {0, 0};
    require(setrlimit(RLIMIT_CORE, &limit) == 0);
    require(getrlimit(RLIMIT_CORE, &limit) == 0 && limit.rlim_cur == 0 && limit.rlim_max == 0);
    require(prctl(PR_SET_DUMPABLE, 0L, 0L, 0L, 0L) == 0);
    require(prctl(PR_GET_DUMPABLE, 0L, 0L, 0L, 0L) == 0);
    require(prctl(PR_SET_NO_NEW_PRIVS, 1L, 0L, 0L, 0L) == 0);
    require(prctl(PR_GET_NO_NEW_PRIVS, 0L, 0L, 0L, 0L) == 1);
    require(argc == 1 && pipe_only(STDIN_FILENO) && pipe_only(STDOUT_FILENO));

    struct sigaction action = {0};
    action.sa_handler = fail;
    require(sigemptyset(&action.sa_mask) == 0);
    const int handled[] = {SIGALRM, SIGINT, SIGTERM, SIGHUP, SIGPIPE};
    for (size_t i = 0; i < sizeof(handled) / sizeof(handled[0]); ++i)
        require(sigaction(handled[i], &action, NULL) == 0);
    alarm(15);
    require(mlockall(MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT) == 0);

    size_t used = 0;
    for (;;) {
        ssize_t count = read(STDIN_FILENO, (void *)(secret + used), sizeof(secret) - used);
        if (count < 0 && errno == EINTR)
            continue;
        require(count >= 0);
        if (count == 0)
            break;
        used += (size_t)count;
        require(used < sizeof(secret));
    }
    require(used == 40);
    for (size_t i = 0; i < used; ++i)
        require((secret[i] >= '0' && secret[i] <= '9') || (secret[i] >= 'a' && secret[i] <= 'f'));

    size_t sent = 0;
    while (sent < used) {
        ssize_t count = write(STDOUT_FILENO, (const void *)(secret + sent), used - sent);
        if (count < 0 && errno == EINTR)
            continue;
        require(count > 0);
        sent += (size_t)count;
    }
    clear_secret();
    alarm(0);
    return 0;
}
