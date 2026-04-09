//! Native DXT1 encoder — calls the original binary's range-fit function on real x87 hardware.
//!
//! Loads the encoder machine code from ClientPatcher.exe into memory at its original
//! virtual addresses, then calls it directly. This gives bit-exact results matching
//! the original binary's x87 53-bit precision rounding behavior.

#[cfg(all(target_os = "windows", target_arch = "x86"))]
mod inner {
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, Ordering};

    // PE section info
    const IMAGE_BASE: u32 = 0x00400000;

    // Encoder code range in .text
    const TEXT_ALLOC_ADDR: usize = 0x0067B000;
    const TEXT_ALLOC_SIZE: usize = 0x2400; // 9216 bytes covers 0x0067B000..0x0067D400
    const ENCODER_ENTRY: usize = 0x0067CF20;

    // .rdata constants range
    const RDATA_ALLOC_ADDR: usize = 0x0084A000;
    const RDATA_ALLOC_SIZE: usize = 0x008C0000 - 0x0084A000; // 0x76000 bytes

    // .data constant: f32 4294967296.0 at 0x008BF734
    const DATA_PAGE_ADDR: usize = 0x008BF000;
    const DATA_PAGE_SIZE: usize = 0x1000;
    const DATA_CONST_OFFSET: usize = 0x734;
    const DATA_CONST_VALUE: u32 = 0x4F800000; // f32 4294967296.0

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

    /// Parse a PE file and return (text_section_data, rdata_section_data, data_section_data)
    /// with their virtual addresses so we can compute offsets.
    struct PeSection {
        virtual_address: u32,
        virtual_size: u32,
        raw_offset: u32,
        raw_size: u32,
    }

    fn parse_pe_sections(pe_data: &[u8]) -> Option<Vec<PeSection>> {
        if pe_data.len() < 0x40 {
            return None;
        }
        // DOS header: e_lfanew at offset 0x3C
        let e_lfanew = u32::from_le_bytes(pe_data[0x3C..0x40].try_into().ok()?) as usize;
        if pe_data.len() < e_lfanew + 4 {
            return None;
        }
        // PE signature
        if &pe_data[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
            return None;
        }
        // COFF header starts at e_lfanew + 4
        let coff = e_lfanew + 4;
        let num_sections = u16::from_le_bytes(pe_data[coff + 2..coff + 4].try_into().ok()?) as usize;
        let optional_header_size = u16::from_le_bytes(pe_data[coff + 16..coff + 18].try_into().ok()?) as usize;

        // Section headers start after optional header
        let sections_start = coff + 20 + optional_header_size;
        let mut sections = Vec::with_capacity(num_sections);

        for i in 0..num_sections {
            let off = sections_start + i * 40;
            if pe_data.len() < off + 40 {
                return None;
            }
            let virtual_size = u32::from_le_bytes(pe_data[off + 8..off + 12].try_into().ok()?);
            let virtual_address = u32::from_le_bytes(pe_data[off + 12..off + 16].try_into().ok()?);
            let raw_size = u32::from_le_bytes(pe_data[off + 16..off + 20].try_into().ok()?);
            let raw_offset = u32::from_le_bytes(pe_data[off + 20..off + 24].try_into().ok()?);

            sections.push(PeSection {
                virtual_address,
                virtual_size,
                raw_offset,
                raw_size,
            });
        }

        Some(sections)
    }

    /// Find the section containing the given RVA.
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

    fn do_init() -> bool {
        eprintln!("[encoder_native] do_init starting...");
        // Locate the PE file
        let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
            Ok(d) => d,
            Err(_) => {
                // Fallback: try relative to executable
                match std::env::current_exe() {
                    Ok(p) => {
                        if let Some(parent) = p.parent().and_then(|p| p.parent()) {
                            parent.to_string_lossy().into_owned()
                        } else {
                            return false;
                        }
                    }
                    Err(_) => return false,
                }
            }
        };

        let pe_path = std::path::Path::new(&manifest_dir)
            .join("..")
            .join("game-installs")
            .join("normal-full-loggedin")
            .join("The Secret World")
            .join("ClientPatcher.exe");

        eprintln!("[encoder_native] PE path: {:?} exists={}", pe_path, pe_path.exists());
        let pe_data = match std::fs::read(&pe_path) {
            Ok(d) => { eprintln!("[encoder_native] PE loaded: {} bytes", d.len()); d },
            Err(e) => { eprintln!("[encoder_native] PE read failed: {}", e); return false; },
        };

        let sections = match parse_pe_sections(&pe_data) {
            Some(s) => s,
            None => return false,
        };

        unsafe {
            // Allocate .text range for encoder code
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

            // Copy encoder code from PE
            if !copy_from_pe(
                &pe_data,
                &sections,
                TEXT_ALLOC_ADDR as u32,
                text_ptr,
                TEXT_ALLOC_SIZE,
            ) {
                eprintln!("[encoder_native] .text copy_from_pe failed");
                return false;
            }

            // Allocate .rdata range for float constants
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

            // Copy .rdata from PE
            if !copy_from_pe(
                &pe_data,
                &sections,
                RDATA_ALLOC_ADDR as u32,
                rdata_ptr,
                RDATA_ALLOC_SIZE,
            ) {
                return false;
            }

            // The constant at 0x8BF734 is within the .rdata mapped range
            // (0x84A000 to 0x8C0000), so it's already covered by copy_from_pe.
            // Verify it was copied correctly.
            let const_ptr = RDATA_ALLOC_ADDR + (0x8BF734 - RDATA_ALLOC_ADDR);
            let const_val = *(const_ptr as *const u32);
            if const_val != DATA_CONST_VALUE {
                // Fallback: write it manually (might be outside raw PE data)
                *(const_ptr as *mut u32) = DATA_CONST_VALUE;
            }
        }

        true
    }

    /// Initialize the native encoder (one-time setup).
    pub fn ensure_init() {
        INIT.call_once(|| {
            let ok = do_init();
            READY.store(ok, Ordering::SeqCst);
            if ok {
                eprintln!("[encoder_native] Loaded original encoder at 0x{:08X}", ENCODER_ENTRY);
            } else {
                eprintln!("[encoder_native] Original encoder not available (graceful fallback)");
            }
        });
    }

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

            // Set x87 control word to 0x027F (53-bit precision, round-to-nearest)
            // then call the encoder function using cdecl convention
            core::arch::asm!(
                // Save current x87 control word and set 53-bit precision
                "sub esp, 4",
                "fnstcw [esp]",
                "push word ptr 0x027F",
                "fldcw [esp]",
                "add esp, 2",
                // Push arguments (cdecl: right to left)
                "push {out_ptr}",
                "push {pix_ptr}",
                // Call the function
                "call {func}",
                // Clean up arguments (cdecl: caller cleans)
                "add esp, 8",
                // Restore x87 control word
                "fldcw [esp]",
                "add esp, 4",
                pix_ptr = in(reg) pixel_ptr,
                out_ptr = in(reg) output_ptr,
                func = in(reg) func_addr,
                // Clobbered by cdecl
                out("eax") _,
                out("ecx") _,
                out("edx") _,
            );
        }

        Some(output)
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86"))]
pub use inner::*;

/// Stub for non-Windows or non-x86 targets.
#[cfg(not(all(target_os = "windows", target_arch = "x86")))]
pub fn native_encode_rangefit(_pixels: &[[u8; 4]; 16]) -> Option<[u8; 8]> {
    None
}
