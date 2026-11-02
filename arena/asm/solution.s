/* Handwritten x86-64 assembly solution: counting sort, then two pointers.
 *
 * Same idea as the reference, written without a compiler and without libc:
 * input is read with the read(2) syscall, values are sorted with a counting
 * sort (the problem bounds a[i] to [0, 2n]), and the pair count comes from a
 * two-pointer scan. Output goes through write(2).
 *
 * The counters for this profile are simulated by Cachegrind, so the number is a
 * model of the instruction stream, not a hardware measurement. That is the same
 * kind label every language gets for these metrics.
 */
    .section .bss
    .align 8
input:      .skip 8388608        /* whole stdin */
values:     .skip 8388608        /* up to n 8-byte values */
counts:     .skip 3200016        /* histogram, one slot per value in [0, 2n] */
outbuf:     .skip 64

    .text
    .file "solution.s"
    .globl main

/* next_int: parse a non-negative integer from (%r14), advance %r14.
 * Returns the value in %rax. Clobbers %rcx, %rdx. */
next_int:
    xor %eax, %eax
ni_skip:
    movzbl (%r14), %ecx
    cmpb $'0', %cl
    jb ni_advance
    cmpb $'9', %cl
    ja ni_advance
    jmp ni_parse
ni_advance:
    testb %cl, %cl
    jz ni_ret
    inc %r14
    jmp ni_skip
ni_parse:
    xor %eax, %eax
ni_digit:
    movzbl (%r14), %edx
    sub $'0', %edx
    cmp $9, %edx
    ja ni_ret
    imul $10, %rax, %rax
    add %rdx, %rax
    inc %r14
    jmp ni_digit
ni_ret:
    ret

main:
    /* Read all of stdin into `input`. */
    xor %r12, %r12
read_loop:
    xor %eax, %eax               /* SYS_read */
    xor %edi, %edi               /* fd 0 */
    lea input(%rip), %rsi
    add %r12, %rsi
    mov $8388608, %rdx
    sub %r12, %rdx
    syscall
    test %rax, %rax
    jle read_done
    add %rax, %r12
    jmp read_loop
read_done:
    lea input(%rip), %r14
    add %r12, %r14
    movb $0, (%r14)

    /* Parse n and the threshold. */
    lea input(%rip), %r14
    call next_int
    mov %rax, %rbx               /* n */
    call next_int
    mov %rax, %rbp               /* threshold */

    /* Parse the n values. */
    lea values(%rip), %r15
    xor %r12, %r12
parse_values:
    cmp %rbx, %r12
    jge parsed
    call next_int
    mov %rax, (%r15, %r12, 8)
    inc %r12
    jmp parse_values
parsed:

    /* Zero the histogram. */
    lea counts(%rip), %rdi
    mov $400002, %rcx
    xor %eax, %eax
    rep stosq

    /* Count occurrences of each value. */
    lea values(%rip), %r15
    lea counts(%rip), %rdi
    xor %r12, %r12
hist_loop:
    cmp %rbx, %r12
    jge hist_done
    mov (%r15, %r12, 8), %rax
    incq (%rdi, %rax, 8)
    inc %r12
    jmp hist_loop
hist_done:

    /* Rebuild the sorted array in place. */
    lea values(%rip), %rsi
    lea counts(%rip), %rdi
    xor %r12, %r12               /* value */
    xor %r13, %r13               /* write index */
recon_outer:
    cmp $400002, %r12
    jge recon_done
    mov (%rdi, %r12, 8), %rcx
    test %rcx, %rcx
    jz recon_next
recon_inner:
    mov %r12, (%rsi, %r13, 8)
    inc %r13
    dec %rcx
    jnz recon_inner
recon_next:
    inc %r12
    jmp recon_outer
recon_done:

    /* Two pointers over the sorted array. */
    xor %r12, %r12               /* lo */
    lea -1(%rbx), %r13           /* hi */
    xor %r14, %r14               /* count */
two_pointer:
    cmp %r13, %r12
    jge count_done
    mov (%rsi, %r12, 8), %rax
    mov (%rsi, %r13, 8), %rdx
    add %rdx, %rax
    cmp %rbp, %rax
    jg shrink_hi
    mov %r13, %rax
    sub %r12, %rax
    add %rax, %r14
    inc %r12
    jmp two_pointer
shrink_hi:
    dec %r13
    jmp two_pointer
count_done:

    /* Format the count as decimal into outbuf, newline-terminated. */
    lea outbuf(%rip), %rsi
    add $31, %rsi
    movb $'\n', (%rsi)
    mov %r14, %rax
    mov $10, %rcx
itoa_loop:
    xor %rdx, %rdx
    div %rcx
    add $'0', %dl
    dec %rsi
    mov %dl, (%rsi)
    test %rax, %rax
    jnz itoa_loop

    /* length = (outbuf + 31) - rsi + 1 */
    lea outbuf(%rip), %r9
    add $31, %r9
    sub %rsi, %r9
    inc %r9
    mov $1, %rax                 /* SYS_write */
    mov $1, %rdi                 /* fd 1 */
    mov %r9, %rdx
    syscall

    mov $60, %rax                /* SYS_exit */
    xor %edi, %edi
    syscall
