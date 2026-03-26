# Local Storage

There are two main types of local storage in the kernel: thread-local and core-local values. These require architectural support, such as the use of
`IA32_FS/GS_BASE` registers (or equivalent mechanisms on other architectures). These registers store a pointer to the base of the currently active
context-local storage region.

Local storage allows code to declare values that look like global variables, but which resolve to different memory depending on the currently
executing context. For example, each CPU core may have its own scheduler state, and each thread may have its own execution state. Local storage
provides an efficient mechanism for accessing such data without requiring explicit indexing or lookup.

This is implemented via a generic wrapper type `XLocal<T>`. A thread-local or core-local value is simply a `static` instance of `XLocal<T>` placed in
a special linker section (either `.cpu_local` or `.thread_local`). This should not be done manually. Instead, thread and core local values should be
declared using the provided macros. These macros allow declarations using normal variable syntax without exposing the internal `XLocal<T>` wrapper.

Example:

```
core_local! {
    pub CURRENT_CORE_ID: u32 = 0;
}
```

This expands to a `static` instance of `CoreLocal<T>` placed inside the `.cpu_local` linker section.

Similarly:

```
thread_local! {
    pub CURRENT_TASK: TaskState = TaskState::new();
}
```

declares a thread-local variable inside the `.thread_local` section.

All thread-local and core-local variables defined in this way are placed inside their respective template sections at link time.

## Template Regions

Each type of local storage has a corresponding template region:

- `.cpu_local` for core-local storage
- `.thread_local` for thread-local storage

These sections contain the initial instances of all declared local variables. They form a contiguous block of memory in the final binary. The start
and end of each region are provided by linker markers:

```
_marker_cpu_local_template_start
_marker_cpu_local_template_end

_marker_thread_local_template_start
_marker_thread_local_template_end
```

These markers allow the kernel to determine the exact byte range containing the template data.

The template region acts as a blueprint for new contexts. Whenever a new thread or CPU context is created, the kernel allocates memory for its local
storage block and copies the contents of the corresponding template section into it.

This ensures that all local variables begin with the initial values specified in their declarations.

## Creating Local Storage Instances

The creation of new local storage blocks is handled by `LocalStorageHandler::create()`. This is done by copying the template region into newly
allocated memory. For example, when a new thread is created, the kernel allocates a TLS block and copies the `.thread_local` template into it. The
thread's TLS base pointer then points to the start of that copy.

Similarly, each CPU core receives its own copy of the `.cpu_local` template.

## The `XLocal<T>` Wrapper

The wrapper types `CoreLocal<T>` and `ThreadLocal<T>` are thin wrappers around `T`:

```
#[repr(C)]
pub struct CoreLocal<T>(T);
```

The wrapper itself does not contain special logic or metadata. Its purpose is to:

- place the variable in the correct linker section
- allow computing the variable's offset inside the template region
- provide dereference behavior that resolves to the current context instance

Both wrappers implement `Deref<Target = T>`, allowing the contained value to be accessed as if it were a normal reference.

For example:

```
let id = *CURRENT_CORE_ID;
```

appears to dereference a global value, but actually accesses the core-local instance for the currently executing CPU.

## Instance Resolution

To obtain the actual instance of a local variable for the current execution context, the implementation combines the offset within the template block
with the context's base pointer. The address of the current instance is `context_base + offset`, where:

- `context_base` is obtained from architecture-specific code
- `offset` is the variable's position inside the template

The `Deref` implementation performs this computation and returns a reference to the resulting address.

## Core-Local Storage

Core-local storage provides one instance of each variable per CPU core. All threads running on the same CPU share the same core-local values.

The base pointer for core-local storage is retrieved using:

```
Arch::get_cpu_local_pointer()
```

Each CPU core receives its own copy of the `.cpu_local` template region during initialization.

Core-local storage is typically used for data such as:

- per-core schedulers
- interrupt state
- per-core statistics
- CPU-specific buffers

Since these values are shared by threads executing on the same CPU, they should only contain data that is safe to access concurrently within that
core's execution environment. Be warned of reentrancy if core-locals are used in irq handlers.

## Thread-Local Storage

Thread-local storage provides one instance of each variable per thread.

The base pointer for TLS is retrieved using:

```
Arch::get_thread_local_pointer()
```

Each thread receives its own copy of the `.thread_local` template region when it is created. The thread structure stores the address of this region
in `thread.tls_addr`. TLS access requires that execution be associated with a thread. The implementation asserts this condition before accessing the
thread-local base pointer.

Thread-local storage is typically used for:

- per-thread execution state
- scheduling metadata
- temporary buffers
- thread-specific caches

## Foreign Loads

Normally, local storage variables are accessed from the current context. That is, dereferencing a `CoreLocal<T>` or `ThreadLocal<T>` resolves the
instance belonging to the currently executing CPU or thread. In some cases, the kernel must inspect the local storage belonging to another context.
For example, the scheduler may need to inspect thread-local state belonging to a thread that is not currently running. This is referred to as a
foreign load. A foreign load accesses a local variable using the base pointer of another context rather than the current one.

Thread-local storage supports foreign loads via `ThreadLocal::read_for(&self, thread: &Thread)`.

Foreign loads must satisfy several conditions:

- The target context must still exist.
- The TLS region must still be valid.
- The contained type must be `Send + Sync`.

### Send and Sync Requirements

Foreign loads require the contained type `T` to implement both `Send` and `Sync`. This requirement is enforced on `ThreadLocal<T>::read_for`.

A foreign load returns a reference to data owned by another thread's TLS region. This means the returned reference may be observed concurrently with
execution on the target thread. In other words, the reference is effectively shared across threads. Because of this, the contained type must satisfy
the normal Rust rules for cross-thread access. For this reason the method is only implemented for:

```rs
impl<T: Send + Sync> ThreadLocal<T> {
    pub fn read_for(&self, thread: &Thread) -> &T
}
```

This ensures that it is not possible to obtain a reference to thread-local data belonging to another thread unless the contained type is safe to
access across threads. Note that this restriction only applies to foreign loads. Accessing thread-local data from the current thread through `Deref`
does not require `Send` or `Sync`, because the value is guaranteed to belong exclusively to the current thread's execution context. In practice,
this means that types stored in TLS can remain thread-confined unless they are intended to be inspected from other threads. Types that are not
`Send + Sync` may still be used in TLS, but they cannot be accessed using `read_for`. This restriction prevents unsound access patterns where a
thread could observe or interact with thread-local state that is not safe to share between threads.

## Safety Semantics

The `XLocal<T>` wrapper implements `Send` and `Sync` unsafely. This is valid because the wrapper itself does not contain the actual data. It only
describes the location of the data within a context-local region. The actual values reside inside per-context storage blocks.

Thread-local access requires that execution currently be associated with a thread. Accessing TLS outside of a thread context is invalid and will
trigger an assertion.

Core-local storage exists for the lifetime of the CPU core. Thread-local storage exists for the lifetime of the thread that owns the TLS block.

