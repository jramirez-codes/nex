//! Handing freed heap memory back to the system.

unsafe extern "C" {
    /// From `<malloc/malloc.h>`; a null zone means every zone, a goal of 0 means all it can.
    fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
}

/// Nothing to set up: the system allocator doesn't keep per-thread arenas around.
pub fn tune_allocator() {}

/// Return pages freed so far (font loading at startup, a font change) to the system. macOS
/// marks large freed blocks reusable but keeps counting them as resident until it is under
/// memory pressure.
pub fn trim_heap() {
    // SAFETY: documented to accept a null zone; safe from any thread.
    unsafe {
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}
