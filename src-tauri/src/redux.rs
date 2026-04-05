//! IOg1/RDX1/HXDR ("Redux") texture decompression.
//!
//! The CDN serves ~3.3% of resources (type 1010004 textures) as IOg1-compressed data.
//! Format: `IOz1(IOg1(RDX1(HXDR(LZMA(DDS)))))`.
//!
//! We already handle the IOz1 outer layer. This module handles the inner IOg1 layer:
//! 1. Parse IOg1/RDX1 container → extract FCTX header + mip sizes
//! 2. Skip HXDR header + 26-byte stream descriptor
//! 3. LZMA decompress → DDS file (128-byte header + mip0 DXT blocks)
//! 4. Use DDS payload as mip0
//! 5. Generate lower mip levels by box-filtering decoded DXT blocks
//! 6. Output: FCTX header + all mip levels

/// IOg1 container magic.
const IOG1_MAGIC: &[u8; 4] = b"IOg1";
/// RDX1 sub-format marker.
const RDX1_MAGIC: &[u8; 4] = b"RDX1";
/// DDS file magic.
const DDS_MAGIC: &[u8; 4] = b"DDS ";
/// DDS header size (magic + DDSURFACEDESC2).
const DDS_HEADER_SIZE: usize = 128;
/// HXDR magic.
const HXDR_MAGIC: &[u8; 4] = b"HXDR";
/// Stream descriptor size between HXDR and LZMA stream.
const STREAM_DESC_SIZE: usize = 26;

/// Texture block codec — determines block size and decode/encode path.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TextureCodec {
    /// DXT1/BC1: 8 bytes per 4×4 block, RGB color only
    Dxt1,
    /// DXT5/BC3: 16 bytes per 4×4 block, alpha + color
    Dxt5,
    /// ATI2/BC5: 16 bytes per 4×4 block, two independent BC4 channels (normal maps)
    Ati2,
}

impl TextureCodec {
    /// Block size in bytes for this codec.
    fn block_size(self) -> usize {
        match self {
            TextureCodec::Dxt1 => 8,
            TextureCodec::Dxt5 | TextureCodec::Ati2 => 16,
        }
    }

    /// Determine codec from IOg1 format tag and DDS FourCC.
    fn from_tags(iog1_fmt: &[u8], dds_fourcc: &[u8]) -> Result<Self, String> {
        match iog1_fmt {
            b"DXT1" => Ok(TextureCodec::Dxt1),
            b"DXT5" => Ok(TextureCodec::Dxt5),
            b"MIXD" => {
                // MIXD container — determine actual codec from DDS FourCC
                match dds_fourcc {
                    b"ATI2" => Ok(TextureCodec::Ati2),
                    b"DXT5" => Ok(TextureCodec::Dxt5),
                    b"DXT1" => Ok(TextureCodec::Dxt1),
                    _ => Err(format!(
                        "Unknown DDS FourCC in MIXD container: {:?}",
                        dds_fourcc
                    )),
                }
            }
            _ => Err(format!("Unknown IOg1 format tag: {:?}", iog1_fmt)),
        }
    }
}

struct ParsedStream {
    fmt_tag: [u8; 4],
    mip_sizes: Vec<usize>,
    dds_data: Vec<u8>,
    width: usize,
    height: usize,
    next_stream_offset: usize,
}

fn parse_stream(data: &[u8], pos: usize) -> Result<ParsedStream, String> {
    // Read: comp_total(u32) + fmt_tag(4) + mip_count(u32) + mip_sizes + HXDR + stream_desc + LZMA -> DDS
    if data.len() < pos + 12 {
        return Err(format!("IOg1 data truncated at stream metadata (offset {})", pos));
    }
    let comp_total = u32::from_le_bytes(data[pos..pos+4].try_into().unwrap()) as usize;
    let mut fmt_tag = [0u8; 4];
    fmt_tag.copy_from_slice(&data[pos+4..pos+8]);
    let mip_count = u32::from_le_bytes(data[pos+8..pos+12].try_into().unwrap()) as usize;

    let mip_table_start = pos + 12;
    if data.len() < mip_table_start + mip_count * 4 {
        return Err("IOg1 data truncated at mip size table".into());
    }
    let mut mip_sizes = Vec::with_capacity(mip_count);
    for i in 0..mip_count {
        let off = mip_table_start + i * 4;
        mip_sizes.push(u32::from_le_bytes(data[off..off+4].try_into().unwrap()) as usize);
    }

    let hxdr_off = mip_table_start + mip_count * 4;
    if data.len() < hxdr_off + 8 {
        return Err("IOg1 data truncated at HXDR".into());
    }
    let hxdr_size = u32::from_le_bytes(data[hxdr_off..hxdr_off+4].try_into().unwrap()) as usize;
    if &data[hxdr_off+4..hxdr_off+8] != HXDR_MAGIC {
        return Err(format!("Expected HXDR magic at offset {}", hxdr_off+4));
    }

    let sd_off = hxdr_off + hxdr_size;
    let lzma_off = sd_off + STREAM_DESC_SIZE;
    let stream_end = hxdr_off + comp_total;

    if data.len() < lzma_off + 13 {
        return Err("IOg1 data truncated before LZMA".into());
    }

    let lzma_data = &data[lzma_off..data.len().min(stream_end)];
    let mut dds_data = Vec::new();
    lzma_rs::lzma_decompress(&mut &lzma_data[..], &mut dds_data)
        .map_err(|e| format!("LZMA decompression failed: {e}"))?;

    if dds_data.len() < DDS_HEADER_SIZE + 4 || &dds_data[0..4] != DDS_MAGIC {
        return Err("Invalid DDS after LZMA decompression".into());
    }

    let height = u32::from_le_bytes(dds_data[12..16].try_into().unwrap()) as usize;
    let width = u32::from_le_bytes(dds_data[16..20].try_into().unwrap()) as usize;

    Ok(ParsedStream {
        fmt_tag, mip_sizes, dds_data, width, height,
        next_stream_offset: hxdr_off + comp_total,
    })
}

/// Returns `true` if `data` starts with the IOg1 magic.
pub fn is_iog1(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == IOG1_MAGIC
}

/// Decompress IOg1/RDX1 texture data → FCTX output.
///
/// Input: raw IOg1 data (after IOz1 decompression).
/// Output: FCTX header + mipmap chain (ready to write to rdbdata).
pub fn decompress_iog1(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 20 {
        return Err("IOg1 data too short for header".into());
    }

    // ── 1. Parse IOg1 header ────────────────────────────────────────
    if &data[0..4] != IOG1_MAGIC {
        return Err(format!("Expected IOg1 magic, got {:?}", &data[0..4]));
    }
    if &data[4..8] != RDX1_MAGIC {
        return Err(format!("Expected RDX1 sub-format, got {:?}", &data[4..8]));
    }

    let decomp_size = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    let fctx_hdr_size = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    // data[16..20] = flags (not needed for decompression)

    if data.len() < 20 + fctx_hdr_size {
        return Err("IOg1 data truncated at FCTX header".into());
    }

    // ── 2. Extract FCTX header (copied verbatim to output) ─────────
    let fctx_hdr = &data[20..20 + fctx_hdr_size];
    let pos = 20 + fctx_hdr_size;

    // ── 3. Parse first stream ───────────────────────────────────────
    let stream1 = parse_stream(data, pos)?;
    let fmt_tag = stream1.fmt_tag;
    let mip_count = stream1.mip_sizes.len();

    // ── 4. Branch on MIXD vs single-stream ──────────────────────────
    if &fmt_tag == b"MIXD" {
        return decompress_mixd(data, fctx_hdr, decomp_size, stream1);
    }

    // ── Single-stream path (DXT1/DXT5) ──────────────────────────────
    let dds_fourcc = &stream1.dds_data[84..88];
    let codec = TextureCodec::from_tags(&fmt_tag, dds_fourcc)?;

    let mip0_data = &stream1.dds_data[DDS_HEADER_SIZE..];
    let mip_sizes = &stream1.mip_sizes;

    if mip_sizes.is_empty() {
        return Err("No mip levels defined".into());
    }
    if mip0_data.len() < mip_sizes[0] {
        return Err(format!(
            "DDS payload too small: {} < mip0 size {}",
            mip0_data.len(),
            mip_sizes[0]
        ));
    }

    // ── 5. Build output: FCTX header + all mips ─────────────────────
    let mut output = Vec::with_capacity(decomp_size);
    output.extend_from_slice(fctx_hdr);

    // Mip 0: copy directly from DDS
    output.extend_from_slice(&mip0_data[..mip_sizes[0]]);

    // Mips 1..N: generate by box-filtering
    let mut current_data = mip0_data[..mip_sizes[0]].to_vec();
    let mut current_w = stream1.width;
    let mut current_h = stream1.height;

    for mip_idx in 1..mip_count {
        let new_w = (current_w / 2).max(4);
        let new_h = (current_h / 2).max(4);

        let mip_data = generate_mip(
            &current_data, current_w, current_h, new_w, new_h, codec,
        );

        if mip_data.len() != mip_sizes[mip_idx] {
            return Err(format!(
                "Mip {} size mismatch: generated {} bytes, expected {}",
                mip_idx,
                mip_data.len(),
                mip_sizes[mip_idx]
            ));
        }

        output.extend_from_slice(&mip_data);
        current_data = mip_data;
        current_w = new_w;
        current_h = new_h;
    }

    if output.len() < decomp_size {
        // Rare: DXT1/DXT5 resource with empty extra planes beyond the mip chain
        // (e.g., resource 205281: DXT1 diffuse bundled with zeroed normal/gloss).
        // The IOg1 only contains the color stream; zero-fill extra planes.
        output.resize(decomp_size, 0);
    } else if output.len() != decomp_size {
        return Err(format!(
            "Output size mismatch: {} bytes, expected {}",
            output.len(),
            decomp_size
        ));
    }

    Ok(output)
}

fn decompress_mixd(
    data: &[u8],
    fctx_hdr: &[u8],
    decomp_size: usize,
    stream1: ParsedStream,
) -> Result<Vec<u8>, String> {
    let width = stream1.width;
    let height = stream1.height;
    let mip_count = stream1.mip_sizes.len();

    if mip_count == 0 {
        return Err("MIXD stream has zero mip levels".into());
    }

    // Check if stream 2 exists. MIXD format_enum=6 has two streams (ATI2+ATI1),
    // but format_enum=2 has only one stream (ATI2 only, simpler normal map).
    let has_stream2 = stream1.next_stream_offset + 12 <= data.len()
        && {
            let s2_comp = u32::from_le_bytes(
                data[stream1.next_stream_offset..stream1.next_stream_offset + 4]
                    .try_into()
                    .unwrap(),
            ) as usize;
            s2_comp > 0 && s2_comp < data.len()
        };

    // Single-stream MIXD (format_enum=2): interleaved 22 bytes/block.
    // Layout per block: 6 bytes (BC4 indices for 3rd channel) + 16 bytes (ATI2).
    // The 3rd channel has implicit endpoints; indices zero-filled (not in IOg1).
    if !has_stream2 {
        let ati2_mip0 = &stream1.dds_data[DDS_HEADER_SIZE..];
        if ati2_mip0.len() < stream1.mip_sizes[0] {
            return Err("ATI2 DDS payload too small".into());
        }

        // Generate full ATI2 mip chain
        let mut ati2_mips: Vec<Vec<u8>> = vec![ati2_mip0[..stream1.mip_sizes[0]].to_vec()];
        let mut cw = width;
        let mut ch = height;
        for mip_idx in 1..mip_count {
            let nw = (cw / 2).max(4);
            let nh = (ch / 2).max(4);
            let mip = generate_mip(&ati2_mips[mip_idx - 1], cw, ch, nw, nh, TextureCodec::Ati2);
            ati2_mips.push(mip);
            cw = nw;
            ch = nh;
        }

        // Assemble interleaved output: for each block, 6 zero bytes + 16 ATI2 bytes
        let mut output = Vec::with_capacity(decomp_size);
        output.extend_from_slice(fctx_hdr);

        let zero_indices = [0u8; 6];
        for mip in &ati2_mips {
            let num_blocks = mip.len() / 16;
            for block_idx in 0..num_blocks {
                let off = block_idx * 16;
                output.extend_from_slice(&zero_indices);
                output.extend_from_slice(&mip[off..off + 16]);
            }
        }

        if output.len() != decomp_size {
            return Err(format!(
                "MIXD single-stream output size mismatch: {} vs expected {}",
                output.len(), decomp_size
            ));
        }
        return Ok(output);
    }

    // Two-stream MIXD: parse stream 2 (ATI1 gloss)
    let stream2 = parse_stream(data, stream1.next_stream_offset)?;
    if &stream2.fmt_tag != b"ATI1" {
        return Err(format!("Expected ATI1 for MIXD stream 2, got {:?}", stream2.fmt_tag));
    }

    // -- Generate ATI2 mip chain (for extracting BC4-red lower mips) --
    let ati2_mip0 = &stream1.dds_data[DDS_HEADER_SIZE..];
    if ati2_mip0.len() < stream1.mip_sizes[0] {
        return Err("ATI2 DDS payload too small".into());
    }
    let ati2_mip0_data = &ati2_mip0[..stream1.mip_sizes[0]];

    let mut ati2_mips: Vec<Vec<u8>> = vec![ati2_mip0_data.to_vec()];
    let mut cw = width;
    let mut ch = height;
    for mip_idx in 1..mip_count {
        let nw = (cw / 2).max(4);
        let nh = (ch / 2).max(4);
        let mip = generate_mip(&ati2_mips[mip_idx - 1], cw, ch, nw, nh, TextureCodec::Ati2);
        ati2_mips.push(mip);
        cw = nw;
        ch = nh;
    }

    // -- Generate BC4 gloss mip chain --
    let bc4_mip0 = &stream2.dds_data[DDS_HEADER_SIZE..];
    if stream2.mip_sizes.is_empty() || bc4_mip0.len() < stream2.mip_sizes[0] {
        return Err("ATI1 DDS payload too small".into());
    }
    let bc4_mip0_data = &bc4_mip0[..stream2.mip_sizes[0]];

    let mut bc4_mips: Vec<Vec<u8>> = vec![bc4_mip0_data.to_vec()];
    let mut cw = width;
    let mut ch = height;
    for mip_idx in 1..mip_count {
        let nw = (cw / 2).max(4);
        let nh = (ch / 2).max(4);
        let mip = generate_mip_bc4(&bc4_mips[mip_idx - 1], cw, ch, nw, nh);
        bc4_mips.push(mip);
        cw = nw;
        ch = nh;
    }

    // -- Assemble output --
    // Correct format_enum=6 FCTX layout (verified against ClientPatcher output):
    //   FCTX header (24 bytes)
    //   Interleaved section: for each mip, for each block: 6 zero bytes + 16 ATI2 bytes
    //   "ATI1" tag (4 bytes)
    //   BC4 gloss: for each mip, all BC4 blocks (8 bytes each)
    let mut output = Vec::with_capacity(decomp_size);
    output.extend_from_slice(fctx_hdr);

    // Interleaved section: 6 zero prefix + 16 ATI2 per block, all mips
    let zero_prefix = [0u8; 6];
    for mip in &ati2_mips {
        let num_blocks = mip.len() / 16;
        for block_idx in 0..num_blocks {
            let off = block_idx * 16;
            output.extend_from_slice(&zero_prefix);
            output.extend_from_slice(&mip[off..off + 16]);
        }
    }

    // "ATI1" tag
    output.extend_from_slice(b"ATI1");

    // BC4 gloss all mips (full 8-byte blocks, SMALLEST mip first)
    for mip in bc4_mips.iter().rev() {
        output.extend_from_slice(mip);
    }

    // Verify output size
    if output.len() != decomp_size {
        return Err(format!(
            "MIXD output size mismatch: {} bytes, expected {} (diff={})",
            output.len(), decomp_size, output.len() as i64 - decomp_size as i64
        ));
    }

    Ok(output)
}

// ─── Mipmap generation ───────────────────────────────────────────────────────

/// Generate a downsampled mip level from DXT block data.
///
/// Decodes 2×2 groups of source blocks to RGBA, box-filters to 4×4,
/// and re-encodes to DXT blocks.
fn generate_mip(
    prev: &[u8],
    prev_w: usize,
    prev_h: usize,
    new_w: usize,
    new_h: usize,
    codec: TextureCodec,
) -> Vec<u8> {
    let block_size = codec.block_size();
    let prev_bx = (prev_w / 4).max(1);
    let prev_by = (prev_h / 4).max(1);
    let new_bx = (new_w / 4).max(1);
    let new_by = (new_h / 4).max(1);

    let mut result = Vec::with_capacity(new_bx * new_by * block_size);

    for by in 0..new_by {
        for bx in 0..new_bx {
            let src_bx = bx * 2;
            let src_by = by * 2;

            let mut pixels = [[0u8; 4]; 64];

            for dy in 0..2 {
                for dx in 0..2 {
                    let sbx = (src_bx + dx).min(prev_bx - 1);
                    let sby = (src_by + dy).min(prev_by - 1);
                    let src_idx = sby * prev_bx + sbx;
                    let block_off = src_idx * block_size;

                    if block_off + block_size > prev.len() {
                        continue;
                    }

                    let block = &prev[block_off..block_off + block_size];
                    let block_pixels = match codec {
                        TextureCodec::Dxt1 => decode_dxt1_block(block),
                        TextureCodec::Dxt5 => decode_dxt5_block(block),
                        TextureCodec::Ati2 => decode_ati2_block(block),
                    };

                    for py in 0..4 {
                        for px in 0..4 {
                            let gy = dy * 4 + py;
                            let gx = dx * 4 + px;
                            pixels[gy * 8 + gx] = block_pixels[py * 4 + px];
                        }
                    }
                }
            }

            let mut filtered = [[0u8; 4]; 16];
            for py in 0..4 {
                for px in 0..4 {
                    let mut r = 0u32;
                    let mut g = 0u32;
                    let mut b = 0u32;
                    let mut a = 0u32;
                    for fy in 0..2 {
                        for fx in 0..2 {
                            let p = &pixels[(py * 2 + fy) * 8 + (px * 2 + fx)];
                            r += p[0] as u32;
                            g += p[1] as u32;
                            b += p[2] as u32;
                            a += p[3] as u32;
                        }
                    }
                    filtered[py * 4 + px] = [
                        (r / 4) as u8,
                        (g / 4) as u8,
                        (b / 4) as u8,
                        (a / 4) as u8,
                    ];
                }
            }

            match codec {
                TextureCodec::Dxt1 => result.extend_from_slice(&encode_dxt1_block(&filtered)),
                TextureCodec::Dxt5 => result.extend_from_slice(&encode_dxt5_block(&filtered)),
                TextureCodec::Ati2 => result.extend_from_slice(&encode_ati2_block(&filtered)),
            }
        }
    }

    result
}

/// Generate a downsampled BC4 mip level from BC4 block data.
fn generate_mip_bc4(prev: &[u8], prev_w: usize, prev_h: usize, new_w: usize, new_h: usize) -> Vec<u8> {
    let prev_bx = (prev_w / 4).max(1);
    let prev_by = (prev_h / 4).max(1);
    let new_bx = (new_w / 4).max(1);
    let new_by = (new_h / 4).max(1);
    let mut result = Vec::with_capacity(new_bx * new_by * 8);

    for by in 0..new_by {
        for bx in 0..new_bx {
            let src_bx = bx * 2;
            let src_by = by * 2;
            let mut values = [0u8; 64];
            for dy in 0..2 {
                for dx in 0..2 {
                    let sbx = (src_bx + dx).min(prev_bx - 1);
                    let sby = (src_by + dy).min(prev_by - 1);
                    let block_off = (sby * prev_bx + sbx) * 8;
                    if block_off + 8 > prev.len() { continue; }
                    let block_vals = decode_bc4_block(&prev[block_off..block_off + 8]);
                    for py in 0..4 {
                        for px in 0..4 {
                            values[(dy * 4 + py) * 8 + (dx * 4 + px)] = block_vals[py * 4 + px];
                        }
                    }
                }
            }
            let mut filtered = [0u8; 16];
            for py in 0..4 {
                for px in 0..4 {
                    let mut sum = 0u32;
                    for fy in 0..2 {
                        for fx in 0..2 {
                            sum += values[(py * 2 + fy) * 8 + (px * 2 + fx)] as u32;
                        }
                    }
                    filtered[py * 4 + px] = (sum / 4) as u8;
                }
            }
            result.extend_from_slice(&encode_bc4_block(&filtered));
        }
    }
    result
}

// ─── DXT1 decode/encode ──────────────────────────────────────────────────────

/// Decode RGB565 → (R, G, B) as u8.
#[inline]
fn decode_rgb565(c: u16) -> (u8, u8, u8) {
    let r = ((c >> 11) & 0x1F) as u32;
    let g = ((c >> 5) & 0x3F) as u32;
    let b = (c & 0x1F) as u32;
    (
        (r * 255 / 31) as u8,
        (g * 255 / 63) as u8,
        (b * 255 / 31) as u8,
    )
}

/// Encode (R, G, B) → RGB565.
#[inline]
fn encode_rgb565(r: u8, g: u8, b: u8) -> u16 {
    let r5 = (r as u16 * 31 / 255) & 0x1F;
    let g6 = (g as u16 * 63 / 255) & 0x3F;
    let b5 = (b as u16 * 31 / 255) & 0x1F;
    (r5 << 11) | (g6 << 5) | b5
}

/// Decode a DXT1 block → 16 RGBA pixels (row-major order, alpha=255).
fn decode_dxt1_block(data: &[u8]) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([data[0], data[1]]);
    let c1 = u16::from_le_bytes([data[2], data[3]]);
    let indices = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);

    let (r0, g0, b0) = decode_rgb565(c0);
    let (r1, g1, b1) = decode_rgb565(c1);

    let palette: [[u8; 4]; 4] = if c0 > c1 {
        [
            [r0, g0, b0, 255],
            [r1, g1, b1, 255],
            [
                ((2 * r0 as u16 + r1 as u16 + 1) / 3) as u8,
                ((2 * g0 as u16 + g1 as u16 + 1) / 3) as u8,
                ((2 * b0 as u16 + b1 as u16 + 1) / 3) as u8,
                255,
            ],
            [
                ((r0 as u16 + 2 * r1 as u16 + 1) / 3) as u8,
                ((g0 as u16 + 2 * g1 as u16 + 1) / 3) as u8,
                ((b0 as u16 + 2 * b1 as u16 + 1) / 3) as u8,
                255,
            ],
        ]
    } else {
        [
            [r0, g0, b0, 255],
            [r1, g1, b1, 255],
            [
                ((r0 as u16 + r1 as u16) / 2) as u8,
                ((g0 as u16 + g1 as u16) / 2) as u8,
                ((b0 as u16 + b1 as u16) / 2) as u8,
                255,
            ],
            [0, 0, 0, 0],
        ]
    };

    let mut pixels = [[0u8; 4]; 16];
    for i in 0..16 {
        let sel = ((indices >> (i * 2)) & 3) as usize;
        pixels[i] = palette[sel];
    }
    pixels
}

/// Encode 16 RGBA pixels → DXT1 block (8 bytes).
///
/// Uses bounding-box endpoint selection for speed. Quality is sufficient for
/// mipmap generation where the source is already DXT-compressed.
fn encode_dxt1_block(pixels: &[[u8; 4]; 16]) -> [u8; 8] {
    let (mut min_r, mut min_g, mut min_b) = (255u8, 255u8, 255u8);
    let (mut max_r, mut max_g, mut max_b) = (0u8, 0u8, 0u8);

    for p in pixels {
        min_r = min_r.min(p[0]);
        min_g = min_g.min(p[1]);
        min_b = min_b.min(p[2]);
        max_r = max_r.max(p[0]);
        max_g = max_g.max(p[1]);
        max_b = max_b.max(p[2]);
    }

    let mut c0 = encode_rgb565(max_r, max_g, max_b);
    let mut c1 = encode_rgb565(min_r, min_g, min_b);

    if c0 == c1 {
        // All pixels same color → indices all 0
        let mut out = [0u8; 8];
        out[0..2].copy_from_slice(&c0.to_le_bytes());
        out[2..4].copy_from_slice(&c1.to_le_bytes());
        return out;
    }

    // Ensure c0 > c1 for 4-color mode
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }

    // Build palette
    let (r0, g0, b0) = decode_rgb565(c0);
    let (r1, g1, b1) = decode_rgb565(c1);
    let palette: [(i16, i16, i16); 4] = [
        (r0 as i16, g0 as i16, b0 as i16),
        (r1 as i16, g1 as i16, b1 as i16),
        (
            (2 * r0 as i16 + r1 as i16 + 1) / 3,
            (2 * g0 as i16 + g1 as i16 + 1) / 3,
            (2 * b0 as i16 + b1 as i16 + 1) / 3,
        ),
        (
            (r0 as i16 + 2 * r1 as i16 + 1) / 3,
            (g0 as i16 + 2 * g1 as i16 + 1) / 3,
            (b0 as i16 + 2 * b1 as i16 + 1) / 3,
        ),
    ];

    // Find closest palette entry for each pixel
    let mut indices = 0u32;
    for (i, p) in pixels.iter().enumerate() {
        let pr = p[0] as i16;
        let pg = p[1] as i16;
        let pb = p[2] as i16;

        let mut best_dist = i32::MAX;
        let mut best_sel = 0u32;
        for (sel, &(cr, cg, cb)) in palette.iter().enumerate() {
            let dr = (pr - cr) as i32;
            let dg = (pg - cg) as i32;
            let db = (pb - cb) as i32;
            let dist = dr * dr + dg * dg + db * db;
            if dist < best_dist {
                best_dist = dist;
                best_sel = sel as u32;
            }
        }
        indices |= best_sel << (i * 2);
    }

    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    out
}

// ─── DXT5 decode/encode ──────────────────────────────────────────────────────

/// Decode a DXT5 block → 16 RGBA pixels.
fn decode_dxt5_block(data: &[u8]) -> [[u8; 4]; 16] {
    let a0 = data[0] as u16;
    let a1 = data[1] as u16;

    // Build alpha lookup table
    let alpha_table: [u8; 8] = if a0 > a1 {
        [
            a0 as u8,
            a1 as u8,
            ((6 * a0 + 1 * a1 + 3) / 7) as u8,
            ((5 * a0 + 2 * a1 + 3) / 7) as u8,
            ((4 * a0 + 3 * a1 + 3) / 7) as u8,
            ((3 * a0 + 4 * a1 + 3) / 7) as u8,
            ((2 * a0 + 5 * a1 + 3) / 7) as u8,
            ((1 * a0 + 6 * a1 + 3) / 7) as u8,
        ]
    } else {
        [
            a0 as u8,
            a1 as u8,
            ((4 * a0 + 1 * a1 + 2) / 5) as u8,
            ((3 * a0 + 2 * a1 + 2) / 5) as u8,
            ((2 * a0 + 3 * a1 + 2) / 5) as u8,
            ((1 * a0 + 4 * a1 + 2) / 5) as u8,
            0,
            255,
        ]
    };

    // Extract 3-bit alpha indices (48 bits = 6 bytes, little-endian)
    let alpha_bits = u64::from_le_bytes([
        data[2], data[3], data[4], data[5], data[6], data[7], 0, 0,
    ]);

    // Color portion (offset 8, same as DXT1)
    let color_pixels = decode_dxt1_block(&data[8..16]);

    let mut pixels = [[0u8; 4]; 16];
    for i in 0..16 {
        let alpha_idx = ((alpha_bits >> (i * 3)) & 7) as usize;
        pixels[i] = [
            color_pixels[i][0],
            color_pixels[i][1],
            color_pixels[i][2],
            alpha_table[alpha_idx],
        ];
    }
    pixels
}

/// Encode 16 RGBA pixels → DXT5 block (16 bytes).
fn encode_dxt5_block(pixels: &[[u8; 4]; 16]) -> [u8; 16] {
    // ── Alpha ──
    let mut min_a = 255u8;
    let mut max_a = 0u8;
    for p in pixels {
        min_a = min_a.min(p[3]);
        max_a = max_a.max(p[3]);
    }

    let (a0, a1) = (max_a, min_a);

    let alpha_table: [u8; 8] = if a0 > a1 {
        let a0w = a0 as u16;
        let a1w = a1 as u16;
        [
            a0,
            a1,
            ((6 * a0w + 1 * a1w + 3) / 7) as u8,
            ((5 * a0w + 2 * a1w + 3) / 7) as u8,
            ((4 * a0w + 3 * a1w + 3) / 7) as u8,
            ((3 * a0w + 4 * a1w + 3) / 7) as u8,
            ((2 * a0w + 5 * a1w + 3) / 7) as u8,
            ((1 * a0w + 6 * a1w + 3) / 7) as u8,
        ]
    } else if a0 == a1 {
        [a0; 8]
    } else {
        let a0w = a0 as u16;
        let a1w = a1 as u16;
        [
            a0,
            a1,
            ((4 * a0w + 1 * a1w + 2) / 5) as u8,
            ((3 * a0w + 2 * a1w + 2) / 5) as u8,
            ((2 * a0w + 3 * a1w + 2) / 5) as u8,
            ((1 * a0w + 4 * a1w + 2) / 5) as u8,
            0,
            255,
        ]
    };

    // Find closest alpha index for each pixel
    let mut alpha_bits: u64 = 0;
    for (i, p) in pixels.iter().enumerate() {
        let a = p[3];
        let mut best_dist = u16::MAX;
        let mut best_idx = 0u64;
        for (j, &ta) in alpha_table.iter().enumerate() {
            let d = (a as i16 - ta as i16).unsigned_abs();
            if d < best_dist {
                best_dist = d;
                best_idx = j as u64;
            }
        }
        alpha_bits |= best_idx << (i * 3);
    }

    let alpha_bytes = alpha_bits.to_le_bytes();

    // ── Color (DXT1) ──
    let color_block = encode_dxt1_block(pixels);

    let mut out = [0u8; 16];
    out[0] = a0;
    out[1] = a1;
    out[2..8].copy_from_slice(&alpha_bytes[0..6]);
    out[8..16].copy_from_slice(&color_block);
    out
}

// ─── BC4/ATI2 decode/encode ─────────────────────────────────────────────────

/// Decode a BC4 block (8 bytes) → 16 channel values.
///
/// BC4 is the alpha portion of DXT5: two endpoint bytes + 6 bytes of 3-bit
/// interpolation indices for 16 pixels.
fn decode_bc4_block(data: &[u8]) -> [u8; 16] {
    let a0 = data[0] as u16;
    let a1 = data[1] as u16;

    let table: [u8; 8] = if a0 > a1 {
        [
            a0 as u8,
            a1 as u8,
            ((6 * a0 + 1 * a1 + 3) / 7) as u8,
            ((5 * a0 + 2 * a1 + 3) / 7) as u8,
            ((4 * a0 + 3 * a1 + 3) / 7) as u8,
            ((3 * a0 + 4 * a1 + 3) / 7) as u8,
            ((2 * a0 + 5 * a1 + 3) / 7) as u8,
            ((1 * a0 + 6 * a1 + 3) / 7) as u8,
        ]
    } else {
        [
            a0 as u8,
            a1 as u8,
            ((4 * a0 + 1 * a1 + 2) / 5) as u8,
            ((3 * a0 + 2 * a1 + 2) / 5) as u8,
            ((2 * a0 + 3 * a1 + 2) / 5) as u8,
            ((1 * a0 + 4 * a1 + 2) / 5) as u8,
            0,
            255,
        ]
    };

    let bits = u64::from_le_bytes([
        data[2], data[3], data[4], data[5], data[6], data[7], 0, 0,
    ]);

    let mut values = [0u8; 16];
    for i in 0..16 {
        let idx = ((bits >> (i * 3)) & 7) as usize;
        values[i] = table[idx];
    }
    values
}

/// Encode 16 channel values → BC4 block (8 bytes).
fn encode_bc4_block(values: &[u8; 16]) -> [u8; 8] {
    let mut min_v = 255u8;
    let mut max_v = 0u8;
    for &v in values {
        min_v = min_v.min(v);
        max_v = max_v.max(v);
    }

    let (a0, a1) = (max_v, min_v);

    let table: [u8; 8] = if a0 > a1 {
        let a0w = a0 as u16;
        let a1w = a1 as u16;
        [
            a0,
            a1,
            ((6 * a0w + 1 * a1w + 3) / 7) as u8,
            ((5 * a0w + 2 * a1w + 3) / 7) as u8,
            ((4 * a0w + 3 * a1w + 3) / 7) as u8,
            ((3 * a0w + 4 * a1w + 3) / 7) as u8,
            ((2 * a0w + 5 * a1w + 3) / 7) as u8,
            ((1 * a0w + 6 * a1w + 3) / 7) as u8,
        ]
    } else if a0 == a1 {
        [a0; 8]
    } else {
        let a0w = a0 as u16;
        let a1w = a1 as u16;
        [
            a0,
            a1,
            ((4 * a0w + 1 * a1w + 2) / 5) as u8,
            ((3 * a0w + 2 * a1w + 2) / 5) as u8,
            ((2 * a0w + 3 * a1w + 2) / 5) as u8,
            ((1 * a0w + 4 * a1w + 2) / 5) as u8,
            0,
            255,
        ]
    };

    let mut bits: u64 = 0;
    for (i, &v) in values.iter().enumerate() {
        let mut best_dist = u16::MAX;
        let mut best_idx = 0u64;
        for (j, &tv) in table.iter().enumerate() {
            let d = (v as i16 - tv as i16).unsigned_abs();
            if d < best_dist {
                best_dist = d;
                best_idx = j as u64;
            }
        }
        bits |= best_idx << (i * 3);
    }

    let bit_bytes = bits.to_le_bytes();
    let mut out = [0u8; 8];
    out[0] = a0;
    out[1] = a1;
    out[2..8].copy_from_slice(&bit_bytes[0..6]);
    out
}

/// Decode an ATI2/BC5 block (16 bytes) → 16 RGBA pixels.
///
/// ATI2 = two independent BC4 blocks: first → R channel, second → G channel.
/// B is set to 0, A to 255.
fn decode_ati2_block(data: &[u8]) -> [[u8; 4]; 16] {
    let red = decode_bc4_block(&data[0..8]);
    let green = decode_bc4_block(&data[8..16]);

    let mut pixels = [[0u8; 4]; 16];
    for i in 0..16 {
        pixels[i] = [red[i], green[i], 0, 255];
    }
    pixels
}

/// Encode 16 RGBA pixels → ATI2/BC5 block (16 bytes).
///
/// Takes R channel → first BC4 block, G channel → second BC4 block.
fn encode_ati2_block(pixels: &[[u8; 4]; 16]) -> [u8; 16] {
    let mut red = [0u8; 16];
    let mut green = [0u8; 16];
    for i in 0..16 {
        red[i] = pixels[i][0];
        green[i] = pixels[i][1];
    }

    let r_block = encode_bc4_block(&red);
    let g_block = encode_bc4_block(&green);

    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&r_block);
    out[8..16].copy_from_slice(&g_block);
    out
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_iog1() {
        assert!(is_iog1(b"IOg1RDX1\x00\x00\x00\x00"));
        assert!(!is_iog1(b"IOz1\x00\x00\x00\x00"));
        assert!(!is_iog1(b"FCT"));
        assert!(!is_iog1(b""));
    }

    #[test]
    fn test_rgb565_roundtrip() {
        for &(r, g, b) in &[(0, 0, 0), (255, 255, 255), (128, 64, 32), (255, 0, 0)] {
            let encoded = encode_rgb565(r, g, b);
            let (dr, dg, db) = decode_rgb565(encoded);
            // RGB565 has limited precision: 5-bit R/B (±8), 6-bit G (±4)
            assert!((dr as i16 - r as i16).abs() <= 8, "R: {} vs {}", dr, r);
            assert!((dg as i16 - g as i16).abs() <= 4, "G: {} vs {}", dg, g);
            assert!((db as i16 - b as i16).abs() <= 8, "B: {} vs {}", db, b);
        }
    }

    #[test]
    fn test_dxt1_decode_encode_roundtrip() {
        // A simple DXT1 block: solid red
        let c0 = encode_rgb565(255, 0, 0);
        let c1 = encode_rgb565(0, 0, 0);
        let (c0, c1) = if c0 > c1 { (c0, c1) } else { (c1, c0) };
        let mut block = [0u8; 8];
        block[0..2].copy_from_slice(&c0.to_le_bytes());
        block[2..4].copy_from_slice(&c1.to_le_bytes());
        // All indices = 0 → all pixels use c0

        let pixels = decode_dxt1_block(&block);
        // All pixels should be red-ish
        for p in &pixels {
            assert!(p[0] > 200, "Expected red, got R={}", p[0]);
            assert!(p[1] < 20, "Expected low green, got G={}", p[1]);
            assert!(p[2] < 20, "Expected low blue, got B={}", p[2]);
        }

        // Re-encode should produce a valid block
        let re_encoded = encode_dxt1_block(&pixels);
        assert_eq!(re_encoded.len(), 8);
    }

    #[test]
    fn test_decompress_iog1_bad_magic() {
        // Data long enough for header but wrong magic
        let mut bad = vec![0u8; 100];
        bad[0..13].copy_from_slice(b"NOT_IOG1_DATA");
        let err = decompress_iog1(&bad).unwrap_err();
        assert!(err.contains("IOg1 magic"), "Unexpected error: {}", err);
    }

    #[test]
    fn test_decompress_iog1_too_short() {
        let err = decompress_iog1(b"IOg1").unwrap_err();
        assert!(
            err.contains("too short"),
            "Unexpected error: {}",
            err
        );
    }

    #[test]
    fn test_bc4_roundtrip() {
        // Build a BC4 block with endpoints 200 and 50
        let mut block = [0u8; 8];
        block[0] = 200; // a0
        block[1] = 50;  // a1
        // All indices = 0 → all pixels get value a0 (200)

        let values = decode_bc4_block(&block);
        assert_eq!(values[0], 200);
        assert_eq!(values[15], 200);

        // Re-encode and verify
        let re_encoded = encode_bc4_block(&values);
        let re_decoded = decode_bc4_block(&re_encoded);
        for i in 0..16 {
            assert!(
                (re_decoded[i] as i16 - values[i] as i16).abs() <= 1,
                "BC4 roundtrip pixel {}: {} vs {}",
                i, re_decoded[i], values[i]
            );
        }
    }

    #[test]
    fn test_bc4_gradient() {
        // Create a gradient of values and verify encode/decode preserves them reasonably
        let values: [u8; 16] = [0, 17, 34, 51, 68, 85, 102, 119, 136, 153, 170, 187, 204, 221, 238, 255];
        let encoded = encode_bc4_block(&values);
        let decoded = decode_bc4_block(&encoded);
        // BC4 has limited precision (8 interpolated values between endpoints)
        // but should preserve the general gradient
        assert!(decoded[0] < 30, "Low end should be low: {}", decoded[0]);
        assert!(decoded[15] > 225, "High end should be high: {}", decoded[15]);
    }

    #[test]
    fn test_ati2_roundtrip() {
        // Create ATI2 block with known R and G channels
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [200, 100, 0, 255]; // R=200, G=100
        }

        let encoded = encode_ati2_block(&pixels);
        assert_eq!(encoded.len(), 16);

        let decoded = decode_ati2_block(&encoded);
        for i in 0..16 {
            assert!(
                (decoded[i][0] as i16 - 200).abs() <= 1,
                "ATI2 R channel pixel {}: {}",
                i, decoded[i][0]
            );
            assert!(
                (decoded[i][1] as i16 - 100).abs() <= 1,
                "ATI2 G channel pixel {}: {}",
                i, decoded[i][1]
            );
            assert_eq!(decoded[i][2], 0, "ATI2 B should be 0");
            assert_eq!(decoded[i][3], 255, "ATI2 A should be 255");
        }
    }

    #[test]
    fn test_generate_mip_bc4() {
        let mut data = Vec::new();
        for _ in 0..4 {
            data.extend_from_slice(&encode_bc4_block(&[200u8; 16]));
        }
        let mip = generate_mip_bc4(&data, 8, 8, 4, 4);
        assert_eq!(mip.len(), 8);
        let decoded = decode_bc4_block(&mip);
        for &v in &decoded {
            assert!((v as i16 - 200).abs() <= 2, "BC4 mip value: {}", v);
        }
    }

    #[test]
    fn test_texture_codec_from_tags() {
        assert_eq!(
            TextureCodec::from_tags(b"DXT1", b"DXT1").unwrap(),
            TextureCodec::Dxt1
        );
        assert_eq!(
            TextureCodec::from_tags(b"DXT5", b"DXT5").unwrap(),
            TextureCodec::Dxt5
        );
        assert_eq!(
            TextureCodec::from_tags(b"MIXD", b"ATI2").unwrap(),
            TextureCodec::Ati2
        );
        assert_eq!(
            TextureCodec::from_tags(b"MIXD", b"DXT5").unwrap(),
            TextureCodec::Dxt5
        );
        assert_eq!(
            TextureCodec::from_tags(b"MIXD", b"DXT1").unwrap(),
            TextureCodec::Dxt1
        );
        assert!(TextureCodec::from_tags(b"XXXX", b"DXT1").is_err());
    }

    // ── Integration tests using real IOg1 data from rdbdata files ────

    /// Read IOg1 data from an rdbdata file at a known offset.
    fn read_iog1_sample(rdb_dir: &str, file_num: u8, offset: u64, size: usize) -> Option<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(rdb_dir)
            .join("RDB")
            .join(format!("{:02}.rdbdata", file_num));

        let mut file = std::fs::File::open(&path).ok()?;
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut buf = vec![0u8; size];
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    #[test]
    #[ignore] // Run with: cargo test -- --ignored iog1_dxt1
    fn iog1_dxt1_128x128_decompresses() {
        let data = read_iog1_sample(
            "failed-attempt/The Secret World",
            4,
            601903056,
            5928,
        )
        .expect("Need failed-attempt rdbdata files for this test");

        assert!(is_iog1(&data), "Sample should be IOg1");

        let result = decompress_iog1(&data).expect("Decompression should succeed");

        // FCTX header should be preserved
        assert_eq!(&result[0..4], b"FCTX", "Output should start with FCTX");

        // Expected output: FCTX header (24) + mips totaling 10936 = 10960
        assert_eq!(result.len(), 10960, "Output size should match decomp_size");

        // Verify DXT1 block structure: 128x128 = 32×32 blocks, 8 bytes each
        // First mip (8192 bytes) should have valid DXT1 blocks
        let mip0 = &result[24..24 + 8192];
        for block_idx in 0..1024 {
            let block = &mip0[block_idx * 8..(block_idx + 1) * 8];
            let c0 = u16::from_le_bytes([block[0], block[1]]);
            let c1 = u16::from_le_bytes([block[2], block[3]]);
            // At least some blocks should have non-zero endpoints
            if block_idx < 10 {
                assert!(
                    c0 != 0 || c1 != 0 || block_idx > 0,
                    "Block {} has zero endpoints",
                    block_idx
                );
            }
        }
    }

    #[test]
    #[ignore] // Run with: cargo test -- --ignored iog1_dxt5
    fn iog1_dxt5_512x256_decompresses() {
        let data = read_iog1_sample(
            "failed-attempt/The Secret World",
            25,
            1022597456,
            10844,
        )
        .expect("Need failed-attempt rdbdata files for this test");

        assert!(is_iog1(&data), "Sample should be IOg1");

        let result = decompress_iog1(&data).expect("Decompression should succeed");

        assert_eq!(&result[0..4], b"FCTX", "Output should start with FCTX");
        assert_eq!(result.len(), 174808, "Output size should match decomp_size");

        // DXT5 blocks are 16 bytes. 512×256 = 128×64 blocks = 8192 blocks
        let mip0 = &result[24..24 + 131072];
        assert_eq!(mip0.len(), 131072);
        assert_eq!(mip0.len() / 16, 8192, "Should have 8192 DXT5 blocks");
    }

    #[test]
    #[ignore] // Run with: cargo test -- --ignored iog1_large_dxt1
    fn iog1_large_dxt1_1024x1024_decompresses() {
        let data = read_iog1_sample(
            "failed-attempt/The Secret World",
            0,
            5252608,
            200_000, // Read more than enough
        )
        .expect("Need failed-attempt rdbdata files for this test");

        assert!(is_iog1(&data), "Sample should be IOg1");

        let result = decompress_iog1(&data).expect("Decompression should succeed");

        assert_eq!(&result[0..4], b"FCTX", "Output should start with FCTX");
        assert_eq!(result.len(), 699088, "Output size should match decomp_size");
    }

}
