//! Process memory. The system allocators keep freed pages, so a closed window would stay in the
//! footprint. mimalloc hands them back on request (measured in `examples/settings_memory.rs`).

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

unsafe extern "C" {
    fn mi_collect(force: bool);
}

/// Returns freed pages to the OS. Costs a few milliseconds, so call it after a big release.
pub fn release_freed() {
    unsafe { mi_collect(true) };
}
