#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

/*
 * 验证 FUSE_INTERRUPT 的最小原生客户端。
 *
 * Python 会对部分被 EINTR 打断的系统调用做自动重试，无法可靠证明内核已经
 * 中止 F_SETLKW。这里明确不设置 SA_RESTART；收到 SIGUSR1 后，进程仍保持 fd
 * 打开，测试端因此能区分“waiter 被 interrupt 取消”和“close/release 兜底”。
 */
static volatile sig_atomic_t interrupted = 0;

static void handle_interrupt(int signal_number) {
    (void)signal_number;
    interrupted = 1;
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s PATH\n", argv[0]);
        return 2;
    }

    struct sigaction action = {0};
    action.sa_handler = handle_interrupt;
    sigemptyset(&action.sa_mask);
    action.sa_flags = 0;
    if (sigaction(SIGUSR1, &action, NULL) != 0) {
        perror("sigaction");
        return 2;
    }

    int fd = open(argv[1], O_RDWR);
    if (fd < 0) {
        perror("open");
        return 2;
    }

    struct flock lock = {
        .l_type = F_WRLCK,
        .l_whence = SEEK_SET,
        .l_start = 0,
        .l_len = 0,
    };
    puts("waiting");
    fflush(stdout);
    if (fcntl(fd, F_SETLKW, &lock) == 0) {
        puts("unexpected-acquire");
        fflush(stdout);
        close(fd);
        return 3;
    }
    if (errno != EINTR || !interrupted) {
        perror("fcntl(F_SETLKW)");
        close(fd);
        return 4;
    }

    puts("interrupted");
    fflush(stdout);
    /* 保持 fd 打开，直到测试端确认 Meta 中没有残留 waiter。 */
    (void)getchar();
    close(fd);
    return 0;
}
