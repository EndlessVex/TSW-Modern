//! Native pipeline — calls the original binary's mip generation functions on real x87 hardware.
//!
//! Loads machine code from ClientPatcher.exe into memory at its original
//! virtual addresses, then calls it directly. This gives bit-exact results matching
//! the original binary's x87 53-bit precision rounding behavior.
//!
//! Supports:
//! - Range-fit DXT1 block encoding (single 4x4 block)
//! - Full mip generation pipeline: u8→float, gamma, box filter, degamma, encode

#[cfg(all(target_os = "windows", target_arch = "x86"))]
mod inner {
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, Ordering};

    // PE section info
    const IMAGE_BASE: u32 = 0x00400000;

    // ── Code region: covers all pipeline + encoder + math functions ──
    // Range: 0x655000..0x685000 (pipeline handlers through pow/floor)
    const TEXT_ALLOC_ADDR: usize = 0x00655000;
    const TEXT_ALLOC_SIZE: usize = 0x030000;

    // ── Read-only data (.rdata) for float constants ──
    const RDATA_ALLOC_ADDR: usize = 0x0084A000;
    const RDATA_ALLOC_SIZE: usize = 0x008C0000 - 0x0084A000; // 0x76000 bytes

    // ── Allocator stub region ──
    // Native pipeline functions call malloc/free at these addresses.
    // We write small trampoline stubs here that redirect to our bump allocator.
    const ALLOC_STUB_ADDR: usize = 0x004F0000;
    const ALLOC_STUB_SIZE: usize = 0x2000;

    // ── Mutable globals (.data) needed by pow/floor ──
    // pow checks [0xBA0320]==0, floor checks [0xBA0440]==0
    const DATA_GLOBALS_ADDR: usize = 0x00BA0000;
    const DATA_GLOBALS_SIZE: usize = 0x1000;

    // ── Bump allocator pool ──
    // Pre-allocated scratch memory for native malloc calls.
    // The pipeline processes one texture at a time; pool resets between textures.
    const POOL_ADDR: usize = 0x10000000;
    const POOL_SIZE: usize = 128 * 1024 * 1024; // 128 MB

    // Address within the stub region where we store the pool offset variable.
    // The asm stubs reference this fixed address for the atomic bump.
    const POOL_OFFSET_STORAGE: usize = ALLOC_STUB_ADDR + 0x100;

    // ── Function entry points ──
    const ENCODER_ENTRY: usize = 0x0067CF20; // range-fit DXT1 encoder (cdecl)
    const FN_POW: usize = 0x00682EE0;        // pow: st(0)=exp, st(1)=base → st(0)=result
    const FN_FLOOR: usize = 0x00681C10;      // floor: st(0)=val → eax=int
    const FN_BOX_FILTER: usize = 0x00677FE0; // box filter: ecx=src_float_img → eax=new_float_img

    // Float image struct vtable address (from .rdata, referenced by box filter)
    const FLOAT_IMG_VTABLE: u32 = 0x00873C60;

    // VirtualAlloc constants
    const MEM_COMMIT_RESERVE: u32 = 0x3000;
    const PAGE_EXECUTE_READWRITE: u32 = 0x40;
    const PAGE_READWRITE: u32 = 0x04;

    #[link(name = "kernel32")]
    extern "system" {
        fn VirtualAlloc(
            lpAddress: usize,
            dwSize: usize,
            flAllocationType: u32,
            flProtect: u32,
        ) -> *mut u8;
    }

    static INIT: Once = Once::new();
    static READY: AtomicBool = AtomicBool::new(false);

    /// Reset the bump allocator pool. Call before each texture.
    pub fn reset_pool() {
        unsafe {
            *(POOL_OFFSET_STORAGE as *mut u32) = 0;
        }
    }

    /// Bump-allocate from the pool (Rust side). Keeps the fixed-address storage
    /// in sync so native asm stubs and Rust allocations share the same pool.
    unsafe fn pool_alloc(size: usize) -> *mut u8 {
        let aligned = (size + 15) & !15;
        let storage = POOL_OFFSET_STORAGE as *mut u32;
        let old_offset = *storage as usize;
        *storage = (old_offset + aligned) as u32;
        (POOL_ADDR + old_offset) as *mut u8
    }

    // ── PE parsing ──

    struct PeSection {
        virtual_address: u32,
        virtual_size: u32,
        raw_offset: u32,
        raw_size: u32,
    }

    fn parse_pe_sections(pe_data: &[u8]) -> Option<Vec<PeSection>> {
        if pe_data.len() < 0x40 { return None; }
        let e_lfanew = u32::from_le_bytes(pe_data[0x3C..0x40].try_into().ok()?) as usize;
        if pe_data.len() < e_lfanew + 4 { return None; }
        if &pe_data[e_lfanew..e_lfanew + 4] != b"PE\0\0" { return None; }
        let coff = e_lfanew + 4;
        let num_sections = u16::from_le_bytes(pe_data[coff + 2..coff + 4].try_into().ok()?) as usize;
        let optional_header_size = u16::from_le_bytes(pe_data[coff + 16..coff + 18].try_into().ok()?) as usize;
        let sections_start = coff + 20 + optional_header_size;
        let mut sections = Vec::with_capacity(num_sections);
        for i in 0..num_sections {
            let off = sections_start + i * 40;
            if pe_data.len() < off + 40 { return None; }
            sections.push(PeSection {
                virtual_size: u32::from_le_bytes(pe_data[off + 8..off + 12].try_into().ok()?),
                virtual_address: u32::from_le_bytes(pe_data[off + 12..off + 16].try_into().ok()?),
                raw_size: u32::from_le_bytes(pe_data[off + 16..off + 20].try_into().ok()?),
                raw_offset: u32::from_le_bytes(pe_data[off + 20..off + 24].try_into().ok()?),
            });
        }
        Some(sections)
    }

    fn find_section_for_rva(sections: &[PeSection], rva: u32) -> Option<&PeSection> {
        sections.iter().find(|s| rva >= s.virtual_address && rva < s.virtual_address + s.virtual_size)
    }

    /// Copy bytes from PE file data for a given virtual address range.
    fn copy_from_pe(
        pe_data: &[u8],
        sections: &[PeSection],
        va_start: u32,
        dest: *mut u8,
        len: usize,
    ) -> bool {
        let rva = va_start - IMAGE_BASE;
        let section = match find_section_for_rva(sections, rva) {
            Some(s) => s,
            None => return false,
        };
        let offset_in_section = rva - section.virtual_address;
        let file_offset = section.raw_offset + offset_in_section;
        let available = section.raw_size.saturating_sub(offset_in_section) as usize;
        let copy_len = len.min(available);

        let file_off = file_offset as usize;
        if file_off + copy_len > pe_data.len() {
            return false;
        }

        unsafe {
            std::ptr::copy_nonoverlapping(
                pe_data[file_off..].as_ptr(),
                dest,
                copy_len,
            );
            // Zero any remaining bytes beyond what the PE provides
            if copy_len < len {
                std::ptr::write_bytes(dest.add(copy_len), 0, len - copy_len);
            }
        }
        true
    }

    /// Set an explicit path to ClientPatcher.exe (called from the helper binary).
    static EXPLICIT_PE_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

    pub fn set_pe_path(path: &str) {
        let _ = EXPLICIT_PE_PATH.set(std::path::PathBuf::from(path));
    }

    /// Locate ClientPatcher.exe, trying explicit path first, then known paths.
    fn find_pe_path() -> Option<std::path::PathBuf> {
        // Check explicit path first (set by helper binary from install directory)
        if let Some(p) = EXPLICIT_PE_PATH.get() {
            if p.exists() {
                return Some(p.clone());
            }
        }
        let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
            Ok(d) => d,
            Err(_) => {
                match std::env::current_exe() {
                    Ok(p) => {
                        if let Some(parent) = p.parent().and_then(|p| p.parent()) {
                            parent.to_string_lossy().into_owned()
                        } else {
                            return None;
                        }
                    }
                    Err(_) => return None,
                }
            }
        };

        let base = std::path::Path::new(&manifest_dir).join("..");
        let candidates = [
            "game-installs/installer-finished/The Secret World/ClientPatcher.exe",
            "game-installs/normal-full-loggedin/The Secret World/ClientPatcher.exe",
            "game-installs/normal-full-patch/The Secret World/ClientPatcher.exe",
            "game-installs/debug-run/ClientPatcher.exe",
        ];

        for candidate in &candidates {
            let path = base.join(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    /// Write allocator stubs at the addresses the native code calls.
    ///
    /// func_0x004f02e2(size) — malloc: bump-allocate from pool
    /// func_0x004f1fad(size) — malloc variant: same implementation
    /// func_0x004f05dc()     — free: no-op (pool resets between textures)
    unsafe fn write_alloc_stubs() {
        // ── malloc stub at 0x004F02E2 ──
        // Called as: push size; call 0x4F02E2; (caller cleans stack)
        // Return pointer in EAX.
        //
        // Implementation: bump allocator from pre-allocated pool.
        //   mov eax, [esp+4]           ; requested size
        //   add eax, 15                ; align up to 16
        //   and eax, 0xFFFFFFF0
        //   lock xadd [POOL_OFFSET_STORAGE], eax  ; atomic bump, old value in eax
        //   add eax, POOL_ADDR         ; convert offset to pointer
        //   ret
        let stub_addr = 0x004F02E2usize as *mut u8;
        let pool_off_addr = POOL_OFFSET_STORAGE as u32;
        let pool_base = POOL_ADDR as u32;

        let mut code: Vec<u8> = Vec::with_capacity(32);
        // mov eax, [esp+4]
        code.extend_from_slice(&[0x8B, 0x44, 0x24, 0x04]);
        // add eax, 15
        code.extend_from_slice(&[0x83, 0xC0, 0x0F]);
        // and eax, 0xFFFFFFF0
        code.extend_from_slice(&[0x25, 0xF0, 0xFF, 0xFF, 0xFF]);
        // lock xadd [pool_off_addr], eax
        code.extend_from_slice(&[0xF0, 0x0F, 0xC1, 0x05]);
        code.extend_from_slice(&pool_off_addr.to_le_bytes());
        // add eax, pool_base
        code.extend_from_slice(&[0x05]);
        code.extend_from_slice(&pool_base.to_le_bytes());
        // ret
        code.push(0xC3);

        std::ptr::copy_nonoverlapping(code.as_ptr(), stub_addr, code.len());

        // ── malloc variant stub at 0x004F1FAD — same implementation ──
        let stub2_addr = 0x004F1FADusize as *mut u8;
        std::ptr::copy_nonoverlapping(code.as_ptr(), stub2_addr, code.len());

        // ── free stub at 0x004F05DC — no-op ret ──
        let free_addr = 0x004F05DCusize as *mut u8;
        *free_addr = 0xC3; // ret
    }

    fn do_init() -> bool {
        eprintln!("[encoder_native] do_init starting...");

        let pe_path = match find_pe_path() {
            Some(p) => p,
            None => {
                eprintln!("[encoder_native] ClientPatcher.exe not found in any known location");
                return false;
            }
        };

        eprintln!("[encoder_native] PE path: {:?} exists={}", pe_path, pe_path.exists());
        let pe_data = match std::fs::read(&pe_path) {
            Ok(d) => { eprintln!("[encoder_native] PE loaded: {} bytes", d.len()); d },
            Err(e) => { eprintln!("[encoder_native] PE read failed: {}", e); return false; },
        };

        let sections = match parse_pe_sections(&pe_data) {
            Some(s) => s,
            None => { eprintln!("[encoder_native] PE parsing failed"); return false; },
        };

        unsafe {
            // ── Allocate .text range for pipeline + encoder code ──
            let text_ptr = VirtualAlloc(
                TEXT_ALLOC_ADDR,
                TEXT_ALLOC_SIZE,
                MEM_COMMIT_RESERVE,
                PAGE_EXECUTE_READWRITE,
            );
            if text_ptr.is_null() || text_ptr as usize != TEXT_ALLOC_ADDR {
                eprintln!("[encoder_native] .text VirtualAlloc at 0x{:08X} failed (got {:?})", TEXT_ALLOC_ADDR, text_ptr);
                return false;
            }
            if !copy_from_pe(&pe_data, &sections, TEXT_ALLOC_ADDR as u32, text_ptr, TEXT_ALLOC_SIZE) {
                eprintln!("[encoder_native] .text copy_from_pe failed");
                return false;
            }
            eprintln!("[encoder_native] .text mapped at 0x{:08X} (0x{:X} bytes)", TEXT_ALLOC_ADDR, TEXT_ALLOC_SIZE);

            // ── Allocate .rdata range for float constants ──
            let rdata_ptr = VirtualAlloc(
                RDATA_ALLOC_ADDR,
                RDATA_ALLOC_SIZE,
                MEM_COMMIT_RESERVE,
                PAGE_READWRITE,
            );
            if rdata_ptr.is_null() || rdata_ptr as usize != RDATA_ALLOC_ADDR {
                eprintln!("[encoder_native] .rdata VirtualAlloc at 0x{:08X} size 0x{:X} failed (got {:?})", RDATA_ALLOC_ADDR, RDATA_ALLOC_SIZE, rdata_ptr);
                return false;
            }
            if !copy_from_pe(&pe_data, &sections, RDATA_ALLOC_ADDR as u32, rdata_ptr, RDATA_ALLOC_SIZE) {
                eprintln!("[encoder_native] .rdata copy_from_pe failed");
                return false;
            }
            eprintln!("[encoder_native] .rdata mapped at 0x{:08X} (0x{:X} bytes)", RDATA_ALLOC_ADDR, RDATA_ALLOC_SIZE);

            // Verify the f32 4294967296.0 constant at 0x8BF734 (used by encoder)
            let const_ptr = 0x8BF734usize;
            let const_val = *(const_ptr as *const u32);
            if const_val != 0x4F800000 {
                *(const_ptr as *mut u32) = 0x4F800000;
            }

            // ── Allocate stub region for allocator trampolines ──
            let stub_ptr = VirtualAlloc(
                ALLOC_STUB_ADDR,
                ALLOC_STUB_SIZE,
                MEM_COMMIT_RESERVE,
                PAGE_EXECUTE_READWRITE,
            );
            if stub_ptr.is_null() || stub_ptr as usize != ALLOC_STUB_ADDR {
                eprintln!("[encoder_native] alloc stub VirtualAlloc at 0x{:08X} failed (got {:?})", ALLOC_STUB_ADDR, stub_ptr);
                return false;
            }
            // Zero the pool offset storage
            *(POOL_OFFSET_STORAGE as *mut u32) = 0;
            write_alloc_stubs();
            eprintln!("[encoder_native] allocator stubs written at 0x004F02E2, 0x004F1FAD, 0x004F05DC");

            // ── Allocate .data globals page ──
            let data_ptr = VirtualAlloc(
                DATA_GLOBALS_ADDR,
                DATA_GLOBALS_SIZE,
                MEM_COMMIT_RESERVE,
                PAGE_READWRITE,
            );
            if data_ptr.is_null() || data_ptr as usize != DATA_GLOBALS_ADDR {
                eprintln!("[encoder_native] .data globals VirtualAlloc at 0x{:08X} failed (got {:?})", DATA_GLOBALS_ADDR, data_ptr);
                return false;
            }
            // Set globals to 0 (pow and floor check these)
            *(0x00BA0320usize as *mut u32) = 0;
            *(0x00BA0440usize as *mut u32) = 0;
            eprintln!("[encoder_native] .data globals at 0x{:08X}: [0xBA0320]=0, [0xBA0440]=0", DATA_GLOBALS_ADDR);

            // ── Allocate bump allocator pool ──
            let pool_ptr = VirtualAlloc(
                POOL_ADDR,
                POOL_SIZE,
                MEM_COMMIT_RESERVE,
                PAGE_READWRITE,
            );
            if pool_ptr.is_null() || pool_ptr as usize != POOL_ADDR {
                eprintln!("[encoder_native] pool VirtualAlloc at 0x{:08X} size {}MB failed (got {:?})", POOL_ADDR, POOL_SIZE / (1024*1024), pool_ptr);
                return false;
            }
            eprintln!("[encoder_native] pool at 0x{:08X} ({}MB)", POOL_ADDR, POOL_SIZE / (1024*1024));

            // ── Verify the float image vtable is present in .rdata ──
            let vtable_val = *(FLOAT_IMG_VTABLE as usize as *const u32);
            eprintln!("[encoder_native] float_img vtable at 0x{:08X} = 0x{:08X}", FLOAT_IMG_VTABLE, vtable_val);
        }

        true
    }

    /// Initialize the native encoder (one-time setup).
    pub fn ensure_init() {
        INIT.call_once(|| {
            let ok = do_init();
            READY.store(ok, Ordering::SeqCst);
            if ok {
                eprintln!("[encoder_native] Native pipeline ready (encoder + full mip generation)");
            } else {
                eprintln!("[encoder_native] Native pipeline not available (graceful fallback)");
            }
        });
    }

    /// Check if the native pipeline is initialized and ready.
    pub fn is_ready() -> bool {
        ensure_init();
        READY.load(Ordering::SeqCst)
    }

    // ═══════════════════════════════════════════════════════════════════
    // Low-level native function wrappers (inline asm)
    // ═══════════════════════════════════════════════════════════════════

    /// Call native pow: base^exp using original x87 code path.
    /// The native function expects st(0)=exp, st(1)=base and returns st(0)=result.
    #[inline(never)]
    unsafe fn native_pow(base: f32, exp: f32) -> f32 {
        if base <= 0.0 { return 0.0; }
        let mut data: [f32; 3] = [base, exp, 0.0];
        let ptr = data.as_mut_ptr();
        let fn_addr = FN_POW as u32;
        core::arch::asm!(
            "sub esp, 4",
            "fnstcw [esp]",
            "push word ptr 0x027F",
            "fldcw [esp]",
            "add esp, 2",
            "fld dword ptr [{p} + 4]",   // st(0) = exp
            "fld dword ptr [{p}]",        // st(0) = base, st(1) = exp
            "fxch st(1)",                  // st(0) = exp, st(1) = base
            "call {fn_ptr}",
            "fstp dword ptr [{p} + 8]",
            "fldcw [esp]",
            "add esp, 4",
            p = in(reg) ptr,
            fn_ptr = in(reg) fn_addr,
            out("eax") _,
            out("ecx") _,
            out("edx") _,
        );
        data[2]
    }

    /// Call native floor function. Takes f32 on x87 stack, returns i32 in EAX.
    #[allow(dead_code)]
    #[inline(never)]
    unsafe fn native_floor(val: f32) -> i32 {
        let result: i32;
        let fn_addr = FN_FLOOR as u32;
        core::arch::asm!(
            "sub esp, 4",
            "fnstcw [esp]",
            "push word ptr 0x027F",
            "fldcw [esp]",
            "add esp, 2",
            "fld dword ptr [{val}]",
            "call {fn_ptr}",
            "fldcw [esp]",
            "add esp, 4",
            val = in(reg) &val,
            fn_ptr = in(reg) fn_addr,
            lateout("eax") result,
            out("ecx") _,
            out("edx") _,
        );
        result
    }

    /// Gamma linearize a single f32 value: pow(val, gamma).
    /// Matches the two-step native approach: val is already stored as f32 (from
    /// the u8*recip255 conversion), then loaded back before pow.
    #[inline(never)]
    unsafe fn gamma_linearize(val: f32, gamma: f32) -> f32 {
        native_pow(val, gamma)
    }

    /// Degamma + scale + floor for one channel: pow(val, 1/gamma) * 255, floor.
    /// Uses fld1;fdiv to compute 1/gamma at x87 precision (NOT a pre-computed f32).
    #[inline(never)]
    unsafe fn degamma_scale_floor(val: f32, gamma: f32, scale: f32) -> i32 {
        let data: [f32; 3] = [val, gamma, scale];
        let ptr = data.as_ptr();
        let result: i32;
        let pow_addr = FN_POW as u32;
        let floor_addr = FN_FLOOR as u32;
        core::arch::asm!(
            "sub esp, 4",
            "fnstcw [esp]",
            "push word ptr 0x027F",
            "fldcw [esp]",
            "add esp, 2",
            // Compute 1/gamma: fld1; fdiv gamma → st(0) = 1/gamma (the exponent)
            "fld1",
            "fdiv dword ptr [{p} + 4]",
            // Load base: st(0)=base, st(1)=1/gamma
            "fld dword ptr [{p}]",
            // Swap so st(0)=exp=1/gamma, st(1)=base=val
            "fxch st(1)",
            // Call pow(val, 1/gamma)
            "call {pow}",
            // st(0) = pow result; multiply by scale (255.0)
            "fmul dword ptr [{p} + 8]",
            // Call floor
            "call {floor}",
            // Restore FPU state
            "fldcw [esp]",
            "add esp, 4",
            p = in(reg) ptr,
            pow = in(reg) pow_addr,
            floor = in(reg) floor_addr,
            lateout("eax") result,
            out("ecx") _,
            out("edx") _,
        );
        result
    }

    /// Direct scale + floor for alpha channel: floor(val * scale).
    #[inline(never)]
    unsafe fn direct_scale_floor(val: f32, scale: f32) -> i32 {
        let data: [f32; 2] = [val, scale];
        let ptr = data.as_ptr();
        let result: i32;
        let floor_addr = FN_FLOOR as u32;
        core::arch::asm!(
            "sub esp, 4",
            "fnstcw [esp]",
            "push word ptr 0x027F",
            "fldcw [esp]",
            "add esp, 2",
            "fld dword ptr [{p}]",
            "fmul dword ptr [{p} + 4]",
            "call {floor}",
            "fldcw [esp]",
            "add esp, 4",
            p = in(reg) ptr,
            floor = in(reg) floor_addr,
            lateout("eax") result,
            out("ecx") _,
            out("edx") _,
        );
        result
    }

    /// x87 box filter: sum 4 floats at 53-bit precision, multiply by 0.25, store as f32.
    /// Matches the native unrolled loop order for the given position.
    /// Currently unused (native_box_filter calls the binary directly), kept for debugging.
    #[allow(dead_code)]
    #[inline(never)]
    unsafe fn box_filter_f32(f0: f32, f1: f32, f2: f32, f3: f32) -> f32 {
        let quarter: f32 = 0.25;
        let data: [f32; 6] = [f0, f1, f2, f3, quarter, 0.0];
        let ptr = data.as_ptr();
        core::arch::asm!(
            "sub esp, 4",
            "fnstcw [esp]",
            "push word ptr 0x027F",
            "fldcw [esp]",
            "add esp, 2",
            "fld dword ptr [{p}]",
            "fadd dword ptr [{p} + 4]",
            "fadd dword ptr [{p} + 8]",
            "fadd dword ptr [{p} + 12]",
            "fmul dword ptr [{p} + 16]",
            "fstp dword ptr [{p} + 20]",
            "fldcw [esp]",
            "add esp, 4",
            p = in(reg) ptr,
        );
        *ptr.add(5)
    }

    /// Call the native box filter function (FUN_677FE0).
    /// Takes a pointer to a float image struct (thiscall: ECX = src img).
    /// Returns a pointer to a newly allocated float image (half dimensions).
    ///
    /// The native function:
    /// - Allocates a new float_img struct via func_0x004f02e2(0x14)
    /// - Allocates float data via func_0x004f1fad(size)
    /// - Computes box-filtered values from source → destination
    /// - Returns pointer to new float image in EAX
    #[inline(never)]
    unsafe fn native_box_filter(src_img: *const u8) -> *mut u8 {
        let result: *mut u8;
        let fn_addr = FN_BOX_FILTER as u32;
        core::arch::asm!(
            // Save/set FPU control word
            "sub esp, 4",
            "fnstcw [esp]",
            "push word ptr 0x027F",
            "fldcw [esp]",
            "add esp, 2",
            // thiscall: ECX = source float image pointer
            "call {fn_ptr}",
            // Restore FPU control word
            "fldcw [esp]",
            "add esp, 4",
            fn_ptr = in(reg) fn_addr,
            inlateout("ecx") src_img as u32 => _,
            lateout("eax") result,
            out("edx") _,
        );
        result
    }

    // ═══════════════════════════════════════════════════════════════════
    // Float image struct helpers
    // ═══════════════════════════════════════════════════════════════════

    /// Float image struct layout (0x14 bytes):
    ///   +0x00: vtable ptr (u32)
    ///   +0x04: width (u16) | height (u16)   [packed as u32]
    ///   +0x08: num_channels (u32)
    ///   +0x0C: total_elements (u32) = width * height * channels
    ///   +0x10: data_ptr (u32) — pointer to planar float array
    ///
    /// Data layout is planar: [R0,R1,...,Rn, G0,G1,...,Gn, B0,B1,...,Bn, A0,A1,...,An]
    /// where n = width*height.

    /// Allocate and initialize a float image struct + data buffer from the pool.
    unsafe fn alloc_float_image(width: u16, height: u16, channels: u32) -> *mut u8 {
        let total = width as u32 * height as u32 * channels;
        let data_size = total as usize * 4; // f32 per element

        // Allocate struct (0x14 bytes)
        let struct_ptr = pool_alloc(0x14);
        // Allocate data buffer
        let data_ptr = pool_alloc(data_size);

        // Initialize struct fields
        let s = struct_ptr as *mut u32;
        *s.add(0) = FLOAT_IMG_VTABLE;                // vtable
        // width (u16) at +0x04, height (u16) at +0x06 — little endian
        *(struct_ptr.add(4) as *mut u16) = width;
        *(struct_ptr.add(6) as *mut u16) = height;
        *s.add(2) = channels;                         // num_channels
        *s.add(3) = total;                            // total_elements
        *s.add(4) = data_ptr as u32;                  // data_ptr

        // Zero the data buffer
        std::ptr::write_bytes(data_ptr, 0, data_size);

        struct_ptr
    }

    /// Read width from a float image struct.
    unsafe fn float_img_width(img: *const u8) -> u16 {
        *(img.add(4) as *const u16)
    }

    /// Read height from a float image struct.
    unsafe fn float_img_height(img: *const u8) -> u16 {
        *(img.add(6) as *const u16)
    }

    /// Read data pointer from a float image struct.
    unsafe fn float_img_data(img: *const u8) -> *mut f32 {
        *(img.add(0x10) as *const u32) as *mut f32
    }

    // ═══════════════════════════════════════════════════════════════════
    // Public API: single-block encoder (existing)
    // ═══════════════════════════════════════════════════════════════════

    /// Encode a 4x4 block of RGBA pixels using the original binary's range-fit encoder.
    ///
    /// Returns `None` if the native encoder is not available.
    /// Input: 16 pixels in RGBA order (R, G, B, A).
    /// Output: 8-byte DXT1 block (c0_lo, c0_hi, c1_lo, c1_hi, idx0, idx1, idx2, idx3).
    pub fn native_encode_rangefit(pixels: &[[u8; 4]; 16]) -> Option<[u8; 8]> {
        ensure_init();
        if !READY.load(Ordering::SeqCst) {
            return None;
        }

        // Convert RGBA u8 pixels to BGRA u32 values
        let mut bgra_pixels: [u32; 16] = [0u32; 16];
        for i in 0..16 {
            let r = pixels[i][0] as u32;
            let g = pixels[i][1] as u32;
            let b = pixels[i][2] as u32;
            bgra_pixels[i] = 0xFF000000 | (r << 16) | (g << 8) | b;
        }

        let mut output = [0u8; 8];

        unsafe {
            let pixel_ptr = bgra_pixels.as_ptr();
            let output_ptr = output.as_mut_ptr();
            let func_addr = ENCODER_ENTRY as u32;

            core::arch::asm!(
                "sub esp, 4",
                "fnstcw [esp]",
                "push word ptr 0x027F",
                "fldcw [esp]",
                "add esp, 2",
                "push {out_ptr}",
                "push {pix_ptr}",
                "call {func}",
                "add esp, 8",
                "fldcw [esp]",
                "add esp, 4",
                pix_ptr = in(reg) pixel_ptr,
                out_ptr = in(reg) output_ptr,
                func = in(reg) func_addr,
                out("eax") _,
                out("ecx") _,
                out("edx") _,
            );
        }

        Some(output)
    }

    // ═══════════════════════════════════════════════════════════════════
    // Public API: full mip generation pipeline
    // ═══════════════════════════════════════════════════════════════════

    /// Generate all mip levels using the native pipeline.
    ///
    /// This runs the REAL binary's code for each pipeline stage:
    /// 1. u8 BGRA → planar float (val * 1/255)
    /// 2. Gamma linearize channels 0-2: pow(val, 2.2)
    /// 3. Box filter: 2x2 average → half dimensions (native FUN_677FE0)
    /// 4. Degamma channels 0-2: pow(val, 1/2.2) * 255, floor
    /// 5. DXT1 range-fit encode per 4x4 block
    ///
    /// Input: decoded u8 pixels (RGBA order, row-major), dimensions.
    /// Output: Vec of mip level byte arrays (mip0 through mipN), or None.
    ///
    /// `mip0_encoded` is the already-encoded mip0 data (just passed through).
    /// The generated mips start from mip1.
    pub fn native_generate_mips(
        decoded_rgba: &[[u8; 4]],
        width: usize,
        height: usize,
        block_size: usize,
        mip_count: usize,
        mip_sizes: &[usize],
        mip0_encoded: &[u8],
    ) -> Option<Vec<Vec<u8>>> {
        ensure_init();
        if !READY.load(Ordering::SeqCst) { return None; }
        if width < 2 || height < 2 || mip_count < 2 { return None; }
        if decoded_rgba.len() != width * height { return None; }

        // Reset bump allocator for this texture
        reset_pool();

        let recip255: f32 = 1.0 / 255.0;
        let gamma: f32 = 2.2;
        let scale: f32 = 255.0;
        let pixel_count = width * height;

        unsafe {
            // ── Step 1: Build planar float image from u8 RGBA pixels ──
            // Layout matches native FUN_678F40: planar [R...][G...][B...][A...]
            // where each channel plane = width*height f32 values.
            // Channel mapping: RGBA input → planar R,G,B,A
            // (The native stores as: ch0=R(from bits 16-23 of BGRA), ch1=G(bits 8-15),
            //  ch2=B(bits 0-7), ch3=A(bits 24-31). Since our input is RGBA, we map:
            //  ch0=R=input[0], ch1=G=input[1], ch2=B=input[2], ch3=A=input[3])
            let float_img = alloc_float_image(width as u16, height as u16, 4);
            let data = float_img_data(float_img);

            for i in 0..pixel_count {
                let p = &decoded_rgba[i];
                // ch0 (R) at offset 0
                *data.add(i) = p[0] as f32 * recip255;
                // ch1 (G) at offset pixel_count
                *data.add(pixel_count + i) = p[1] as f32 * recip255;
                // ch2 (B) at offset pixel_count*2
                *data.add(pixel_count * 2 + i) = p[2] as f32 * recip255;
                // ch3 (A) at offset pixel_count*3
                *data.add(pixel_count * 3 + i) = p[3] as f32 * recip255;
            }

            // ── Step 2: Gamma linearize channels 0-2 ──
            // Apply pow(val, 2.2) to each pixel in channels 0, 1, 2 (R, G, B).
            // Channel 3 (A) is left as-is (linear).
            for ch in 0..3u32 {
                let ch_offset = ch as usize * pixel_count;
                for i in 0..pixel_count {
                    let ptr = data.add(ch_offset + i);
                    let val = *ptr;
                    *ptr = gamma_linearize(val, gamma);
                }
            }

            // ── Generate mip levels ──
            let mut all_mips: Vec<Vec<u8>> = Vec::with_capacity(mip_count);
            all_mips.push(mip0_encoded.to_vec());

            let mut current_img = float_img;
            let mut _cur_w = width;
            let mut _cur_h = height;

            for mip_idx in 1..mip_count {
                // ── Step 3: Box filter → half dimensions ──
                // Call the native box filter function directly.
                // It allocates a new float image from the pool and returns it.
                let filtered_img = native_box_filter(current_img);
                if filtered_img.is_null() {
                    eprintln!("[encoder_native] box filter returned null at mip {}", mip_idx);
                    return None;
                }

                let new_w = float_img_width(filtered_img) as usize;
                let new_h = float_img_height(filtered_img) as usize;
                let new_pixel_count = new_w * new_h;
                let filtered_data = float_img_data(filtered_img);

                // ── Step 4: Degamma → u8 pixels for encoding ──
                // Channels 0-2: pow(val, 1/2.2) * 255, floor
                // Channel 3 (A): floor(val * 255)
                let new_bx = (new_w / 4).max(1);
                let new_by = (new_h / 4).max(1);
                let mut mip_data = Vec::with_capacity(new_bx * new_by * block_size);

                for by in 0..new_by {
                    for bx in 0..new_bx {
                        let mut block_bgra: [u32; 16] = [0u32; 16];
                        for py in 0..4 {
                            for px in 0..4 {
                                let fx = (bx * 4 + px).min(new_w - 1);
                                let fy = (by * 4 + py).min(new_h - 1);
                                let idx = fy * new_w + fx;

                                // Read planar float values
                                let r_f = *filtered_data.add(idx);
                                let g_f = *filtered_data.add(new_pixel_count + idx);
                                let b_f = *filtered_data.add(new_pixel_count * 2 + idx);
                                let a_f = *filtered_data.add(new_pixel_count * 3 + idx);

                                // Degamma R, G, B channels
                                let r = degamma_scale_floor(r_f, gamma, scale).clamp(0, 255) as u32;
                                let g = degamma_scale_floor(g_f, gamma, scale).clamp(0, 255) as u32;
                                let b = degamma_scale_floor(b_f, gamma, scale).clamp(0, 255) as u32;
                                // Alpha: direct scale
                                let a = direct_scale_floor(a_f, scale).clamp(0, 255) as u32;

                                // Pack as BGRA u32 for the encoder
                                // Encoder expects: 0xAARRGGBB (alpha in high byte)
                                block_bgra[py * 4 + px] = (a << 24) | (r << 16) | (g << 8) | b;
                            }
                        }

                        // ── Step 5: Encode the 4x4 block ──
                        if block_size == 8 {
                            // DXT1: call the native range-fit encoder
                            let mut output = [0u8; 8];
                            let pixel_ptr = block_bgra.as_ptr();
                            let output_ptr = output.as_mut_ptr();
                            let func_addr = ENCODER_ENTRY as u32;
                            core::arch::asm!(
                                "sub esp, 4",
                                "fnstcw [esp]",
                                "push word ptr 0x027F",
                                "fldcw [esp]",
                                "add esp, 2",
                                "push {out_ptr}",
                                "push {pix_ptr}",
                                "call {func}",
                                "add esp, 8",
                                "fldcw [esp]",
                                "add esp, 4",
                                pix_ptr = in(reg) pixel_ptr,
                                out_ptr = in(reg) output_ptr,
                                func = in(reg) func_addr,
                                out("eax") _,
                                out("ecx") _,
                                out("edx") _,
                            );
                            mip_data.extend_from_slice(&output);
                        } else {
                            // DXT5/ATI2: not yet supported via native encoder.
                            // Return None to fall back to Rust pipeline.
                            return None;
                        }
                    }
                }

                if mip_idx < mip_sizes.len() && mip_data.len() != mip_sizes[mip_idx] {
                    eprintln!(
                        "[encoder_native] mip {} size mismatch: generated {} bytes, expected {}",
                        mip_idx, mip_data.len(), mip_sizes[mip_idx]
                    );
                    return None;
                }

                all_mips.push(mip_data);
                current_img = filtered_img;
                _cur_w = new_w;
                _cur_h = new_h;

                // Re-apply gamma linearization to the box-filtered image for the next
                // mip level's box filter. The native pipeline keeps the float image in
                // LINEAR space across mip levels — the gamma was applied once at the top,
                // so the box filter operates on already-linearized data. No re-gamma needed.
            }

            Some(all_mips)
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Tests
    // ═══════════════════════════════════════════════════════════════════

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        #[ignore]
        fn test_native_pow_basic() {
            ensure_init();
            if !READY.load(Ordering::SeqCst) {
                eprintln!("Native pipeline not available, skipping");
                return;
            }
            unsafe {
                let result = native_pow(2.0, 2.0);
                assert!((result - 4.0).abs() < 0.001, "pow(2,2)={}", result);

                let result2 = native_pow(0.5, 2.2);
                assert!(result2 > 0.0 && result2 < 0.5, "pow(0.5, 2.2)={}", result2);
                eprintln!("pow tests passed: pow(2,2)={}, pow(0.5,2.2)={}", result, result2);
            }
        }

        #[test]
        #[ignore]
        fn test_native_floor_basic() {
            ensure_init();
            if !READY.load(Ordering::SeqCst) {
                eprintln!("Native pipeline not available, skipping");
                return;
            }
            unsafe {
                assert_eq!(native_floor(3.7), 3);
                assert_eq!(native_floor(3.0), 3);
                assert_eq!(native_floor(-1.5), -2);
                eprintln!("floor tests passed");
            }
        }

        #[test]
        #[ignore]
        fn test_native_box_filter_basic() {
            ensure_init();
            if !READY.load(Ordering::SeqCst) {
                eprintln!("Native pipeline not available, skipping");
                return;
            }
            unsafe {
                // Create a 4x4 float image with 4 channels, uniform value 0.5
                let img = alloc_float_image(4, 4, 4);
                let data = float_img_data(img);
                let pixel_count = 16usize; // 4*4
                for ch in 0..4 {
                    for i in 0..pixel_count {
                        *data.add(ch * pixel_count + i) = 0.5;
                    }
                }

                let result = native_box_filter(img);
                assert!(!result.is_null(), "box filter returned null");

                let rw = float_img_width(result);
                let rh = float_img_height(result);
                assert_eq!(rw, 2, "expected width 2, got {}", rw);
                assert_eq!(rh, 2, "expected height 2, got {}", rh);

                let rdata = float_img_data(result);
                let rpixels = 4usize; // 2*2
                for ch in 0..4 {
                    for i in 0..rpixels {
                        let val = *rdata.add(ch * rpixels + i);
                        assert!(
                            (val - 0.5).abs() < 0.001,
                            "ch{} pixel{}: expected ~0.5, got {}",
                            ch, i, val
                        );
                    }
                }
                eprintln!("box filter test passed: 4x4 → 2x2, values preserved");
            }
        }

        #[test]
        #[ignore]
        fn test_native_full_pipeline() {
            ensure_init();
            if !READY.load(Ordering::SeqCst) {
                eprintln!("Native pipeline not available, skipping");
                return;
            }

            // Create a simple 8x8 test image with known pixel values
            let w = 8usize;
            let h = 8usize;
            let mut pixels = vec![[128u8, 128, 128, 255]; w * h];
            // Add some variation
            for y in 0..h {
                for x in 0..w {
                    pixels[y * w + x] = [
                        (100 + x * 15) as u8,
                        (80 + y * 15) as u8,
                        (120 + (x + y) * 5) as u8,
                        255u8,
                    ];
                }
            }

            // DXT1: block_size=8, 2 mip levels (mip0=8x8, mip1=4x4)
            let block_size = 8;
            let mip0_bx = w / 4;
            let mip0_by = h / 4;
            let mip0_size = mip0_bx * mip0_by * block_size;
            let mip1_size = 1 * 1 * block_size; // 4x4 = one block

            let mip_sizes = vec![mip0_size, mip1_size];
            let mip0_encoded = vec![0u8; mip0_size]; // dummy mip0

            let result = native_generate_mips(
                &pixels, w, h, block_size, 2, &mip_sizes, &mip0_encoded,
            );

            match result {
                Some(mips) => {
                    assert_eq!(mips.len(), 2, "expected 2 mip levels");
                    assert_eq!(mips[0].len(), mip0_size, "mip0 size mismatch");
                    assert_eq!(mips[1].len(), mip1_size, "mip1 size mismatch");
                    eprintln!("Full pipeline test passed!");
                    eprintln!("  mip0: {} bytes", mips[0].len());
                    eprintln!("  mip1: {} bytes = {:02X?}", mips[1].len(), &mips[1]);
                }
                None => {
                    panic!("native_generate_mips returned None");
                }
            }
        }
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86"))]
pub use inner::*;

// ═══════════════════════════════════════════════════════════════════
// Stubs for non-Windows or non-x86 targets
// ═══════════════════════════════════════════════════════════════════

#[cfg(not(all(target_os = "windows", target_arch = "x86")))]
pub fn native_encode_rangefit(_pixels: &[[u8; 4]; 16]) -> Option<[u8; 8]> {
    None
}

#[cfg(not(all(target_os = "windows", target_arch = "x86")))]
pub fn native_generate_mips(
    _decoded_rgba: &[[u8; 4]],
    _width: usize,
    _height: usize,
    _block_size: usize,
    _mip_count: usize,
    _mip_sizes: &[usize],
    _mip0_encoded: &[u8],
) -> Option<Vec<Vec<u8>>> {
    None
}

#[cfg(not(all(target_os = "windows", target_arch = "x86")))]
pub fn is_ready() -> bool {
    false
}

#[cfg(not(all(target_os = "windows", target_arch = "x86")))]
pub fn reset_pool() {}

#[cfg(not(all(target_os = "windows", target_arch = "x86")))]
pub fn set_pe_path(_path: &str) {}
