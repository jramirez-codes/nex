//! Keeping glibc's heap small: few arenas, and freed memory handed back to the kernel.

/// Call first thing in `main`, before any thread starts. glibc gives each thread that
/// allocates its own arena (up to 8 per core), and memory freed in an arena stays in the
/// process. nex's startup threads (parallel font lookups, the PTY readers) would each leave
/// one behind; two arenas are plenty for a UI thread plus readers.
pub fn tune_allocator() {
    #[cfg(target_env = "gnu")]
    // SAFETY: plain call into glibc, made before other threads exist.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

/// Return heap pages freed so far (font loading at startup, a font change) to the kernel.
/// glibc otherwise keeps them, and they count toward the process's resident memory.
pub fn trim_heap() {
    #[cfg(target_env = "gnu")]
    // SAFETY: plain call into glibc; safe from any thread.
    unsafe {
        libc::malloc_trim(0);
    }
}
