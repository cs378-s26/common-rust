/* Copyright (C) 2025 Ahmed Gheith and contributors.
 *
 * Use restricted to classroom projects.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY
 * SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION
 * OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN
 * CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

	.global outb
outb:
	mov %di, %dx
	mov %sil, %al
	outb %al, %dx
	ret


	.global inb
inb:
	mov %di, %dx
	xor %rax, %rax
	inb %dx, %al
	ret

	.global get_cr0
get_cr0:
	mov %cr0, %rax
	ret

	.global get_cr4
get_cr4:
	mov %cr4, %rax
	ret

	.global set_cr4
set_cr4:
	mov %rdi, %rax
	mov %rax, %cr4
	ret

	.global enable_sse
enable_sse:
	mov %cr4, %rax
	and $0xfffb, %ax
	or $0x602, %ax
	mov %rax, %cr4
	ret

	.global rdfsbase
rdfsbase:
	rdfsbase %rax
	ret

	.global wrfsbase
wrfsbase:
	wrfsbase %rdi
	ret

	.global rdgsbase
rdgsbase:
	rdgsbase %rax
	ret

	.global wrgsbase
wrgsbase:
	wrgsbase %rdi
	ret

	.global memcpy
memcpy:
	cld
	mov %rdi, %rax
	mov %rdx, %rcx
	rep movsb
	ret

	.global memset
memset:
	mov %rdi, %rax
	mov %rdx, %rcx
	rep stosb
	ret

	.global context_switch // (uintptr_t fs, void (*func)(void *), void *arg)
context_switch:
	mov %rbp, %fs:saved_rbp@tpoff
	mov %rbx, %fs:saved_rbx@tpoff
	mov %r12, %fs:saved_r12@tpoff
	mov %r13, %fs:saved_r13@tpoff
	mov %r14, %fs:saved_r14@tpoff
	mov %r15, %fs:saved_r15@tpoff
	
	mov %rsp, %fs:saved_rsp@tpoff
	pushf
	pop %fs:saved_rflags@tpoff

	
	cli
	wrfsbase %rdi
	
	mov %fs:saved_rbp@tpoff, %rbp
	mov %fs:saved_rbx@tpoff, %rbx
	mov %fs:saved_r12@tpoff, %r12
	mov %fs:saved_r13@tpoff, %r13
	mov %fs:saved_r14@tpoff, %r14
	mov %fs:saved_r15@tpoff, %r15
	mov %fs:saved_rsp@tpoff, %rsp
	push %fs:saved_rflags@tpoff
	popf

	sub $8, %rsp
	mov %rdx, %rdi
	call *%rsi // after_switch()
	add $8, %rsp
	ret


	.global pause
pause:
	pause
	ret




	

	.section .note.GNU-stack,"",@progbits
	



