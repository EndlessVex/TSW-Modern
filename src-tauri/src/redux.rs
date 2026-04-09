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

/// DXT1 solid-block lookup tables matching the game's output.
/// 5-bit (R/B channels) and 6-bit (G channel) optimal endpoint pairs.
/// Each entry: [c0_quantized, c1_quantized] for input value 0-255.
static DXT1_SOLID_5BIT: [[u8; 2]; 256] = [
    [ 0, 0], [ 0, 0], [ 0, 1], [ 0, 1], [ 1, 0], [ 1, 0], [ 1, 0], [ 1, 1],
    [ 1, 1], [ 1, 1], [ 1, 2], [ 0, 4], [ 2, 1], [ 2, 1], [ 2, 1], [ 2, 2],
    [ 2, 2], [ 2, 2], [ 2, 3], [ 1, 5], [ 3, 2], [ 3, 2], [ 4, 0], [ 3, 3],
    [ 3, 3], [ 3, 3], [ 3, 4], [ 3, 4], [ 3, 4], [ 3, 5], [ 4, 3], [ 4, 3],
    [ 5, 2], [ 4, 4], [ 4, 4], [ 4, 5], [ 4, 5], [ 5, 4], [ 5, 4], [ 5, 4],
    [ 6, 3], [ 5, 5], [ 5, 5], [ 5, 6], [ 4, 8], [ 6, 5], [ 6, 5], [ 6, 5],
    [ 6, 6], [ 6, 6], [ 6, 6], [ 6, 7], [ 5, 9], [ 7, 6], [ 7, 6], [ 8, 4],
    [ 7, 7], [ 7, 7], [ 7, 7], [ 7, 8], [ 7, 8], [ 7, 8], [ 7, 9], [ 8, 7],
    [ 8, 7], [ 9, 6], [ 8, 8], [ 8, 8], [ 8, 9], [ 8, 9], [ 9, 8], [ 9, 8],
    [ 9, 8], [10, 7], [ 9, 9], [ 9, 9], [ 9,10], [ 8,12], [10, 9], [10, 9],
    [10, 9], [10,10], [10,10], [10,10], [10,11], [ 9,13], [11,10], [11,10],
    [12, 8], [11,11], [11,11], [11,11], [11,12], [11,12], [11,12], [11,13],
    [12,11], [12,11], [13,10], [12,12], [12,12], [12,13], [12,13], [13,12],
    [13,12], [13,12], [14,11], [13,13], [13,13], [13,14], [12,16], [14,13],
    [14,13], [14,13], [14,14], [14,14], [14,14], [14,15], [13,17], [15,14],
    [15,14], [16,12], [15,15], [15,15], [15,15], [15,16], [15,16], [15,16],
    [15,17], [16,15], [16,15], [17,14], [16,16], [16,16], [16,17], [16,17],
    [17,16], [17,16], [17,16], [18,15], [17,17], [17,17], [17,18], [16,20],
    [18,17], [18,17], [18,17], [18,18], [18,18], [18,18], [18,19], [17,21],
    [19,18], [19,18], [20,16], [19,19], [19,19], [19,19], [19,20], [19,20],
    [19,20], [19,21], [20,19], [20,19], [21,18], [20,20], [20,20], [20,21],
    [20,21], [21,20], [21,20], [21,20], [22,19], [21,21], [21,21], [21,22],
    [20,24], [22,21], [22,21], [22,21], [22,22], [22,22], [22,22], [22,23],
    [21,25], [23,22], [23,22], [24,20], [23,23], [23,23], [23,23], [23,24],
    [23,24], [23,24], [23,25], [24,23], [24,23], [25,22], [24,24], [24,24],
    [24,25], [24,25], [25,24], [25,24], [25,24], [26,23], [25,25], [25,25],
    [25,26], [24,28], [26,25], [26,25], [26,25], [26,26], [26,26], [26,26],
    [26,27], [25,29], [27,26], [27,26], [28,24], [27,27], [27,27], [27,27],
    [27,28], [27,28], [27,28], [27,29], [28,27], [28,27], [29,26], [28,28],
    [28,28], [28,29], [28,29], [29,28], [29,28], [29,28], [30,27], [29,29],
    [29,29], [29,30], [29,30], [30,29], [30,29], [30,29], [30,30], [30,30],
    [30,30], [30,31], [30,31], [31,30], [31,30], [31,30], [31,31], [31,31],
];

static DXT1_SOLID_6BIT: [[u8; 2]; 256] = [
    [ 0, 0], [ 0, 1], [ 1, 0], [ 1, 1], [ 1, 1], [ 1, 2], [ 2, 1], [ 2, 2],
    [ 2, 2], [ 2, 3], [ 3, 2], [ 3, 3], [ 3, 3], [ 3, 4], [ 4, 3], [ 4, 4],
    [ 4, 4], [ 4, 5], [ 5, 4], [ 5, 5], [ 5, 5], [ 5, 6], [ 6, 5], [ 0,17],
    [ 6, 6], [ 6, 7], [ 7, 6], [ 2,16], [ 7, 7], [ 7, 8], [ 8, 7], [ 3,17],
    [ 8, 8], [ 8, 9], [ 9, 8], [ 5,16], [ 9, 9], [ 9,10], [10, 9], [ 6,17],
    [10,10], [10,11], [11,10], [ 8,16], [11,11], [11,12], [12,11], [ 9,17],
    [12,12], [12,13], [13,12], [11,16], [13,13], [13,14], [14,13], [12,17],
    [14,14], [14,15], [15,14], [14,16], [15,15], [15,16], [16,14], [16,15],
    [17,14], [16,16], [16,17], [17,16], [18,15], [17,17], [17,18], [18,17],
    [20,14], [18,18], [18,19], [19,18], [21,15], [19,19], [19,20], [20,19],
    [23,14], [20,20], [20,21], [21,20], [24,15], [21,21], [21,22], [22,21],
    [26,14], [22,22], [22,23], [23,22], [27,15], [23,23], [23,24], [24,23],
    [19,33], [24,24], [24,25], [25,24], [21,32], [25,25], [25,26], [26,25],
    [22,33], [26,26], [26,27], [27,26], [24,32], [27,27], [27,28], [28,27],
    [25,33], [28,28], [28,29], [29,28], [27,32], [29,29], [29,30], [30,29],
    [28,33], [30,30], [30,31], [31,30], [30,32], [31,31], [31,32], [32,30],
    [32,31], [33,30], [32,32], [32,33], [33,32], [34,31], [33,33], [33,34],
    [34,33], [36,30], [34,34], [34,35], [35,34], [37,31], [35,35], [35,36],
    [36,35], [39,30], [36,36], [36,37], [37,36], [40,31], [37,37], [37,38],
    [38,37], [42,30], [38,38], [38,39], [39,38], [43,31], [39,39], [39,40],
    [40,39], [35,49], [40,40], [40,41], [41,40], [37,48], [41,41], [41,42],
    [42,41], [38,49], [42,42], [42,43], [43,42], [40,48], [43,43], [43,44],
    [44,43], [41,49], [44,44], [44,45], [45,44], [43,48], [45,45], [45,46],
    [46,45], [44,49], [46,46], [46,47], [47,46], [46,48], [47,47], [47,48],
    [48,46], [48,47], [49,46], [48,48], [48,49], [49,48], [50,47], [49,49],
    [49,50], [50,49], [52,46], [50,50], [50,51], [51,50], [53,47], [51,51],
    [51,52], [52,51], [55,46], [52,52], [52,53], [53,52], [56,47], [53,53],
    [53,54], [54,53], [58,46], [54,54], [54,55], [55,54], [59,47], [55,55],
    [55,56], [56,55], [61,46], [56,56], [56,57], [57,56], [62,47], [57,57],
    [57,58], [58,57], [58,58], [58,58], [58,59], [59,58], [59,59], [59,59],
    [59,60], [60,59], [60,60], [60,60], [60,61], [61,60], [61,61], [61,61],
    [61,62], [62,61], [62,62], [62,62], [62,63], [63,62], [63,63], [63,63],
];


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

/// DDS header information extracted from IOg1 data.
#[derive(Debug)]
pub struct DdsInfo {
    pub mip_map_count: u32,
    pub width: u32,
    pub height: u32,
    pub fourcc: [u8; 4],
    pub payload_size: usize,
    pub mip0_size: usize,
    pub iog1_fmt_tag: [u8; 4],
    pub iog1_mip_count: usize,
    pub iog1_mip_sizes: Vec<usize>,
}

/// Inspect the DDS header inside IOg1 data without generating mips.
pub fn inspect_iog1_dds(data: &[u8]) -> Result<DdsInfo, String> {
    if data.len() < 20 {
        return Err("IOg1 data too short".into());
    }
    if &data[0..4] != IOG1_MAGIC || &data[4..8] != RDX1_MAGIC {
        return Err("Not IOg1/RDX1 data".into());
    }
    let fctx_hdr_size = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let pos = 20 + fctx_hdr_size;

    let stream = parse_stream(data, pos)?;
    let dds = &stream.dds_data;

    if dds.len() < DDS_HEADER_SIZE {
        return Err("DDS too short for header".into());
    }

    let mip_map_count = u32::from_le_bytes(dds[28..32].try_into().unwrap());
    let height = u32::from_le_bytes(dds[12..16].try_into().unwrap());
    let width = u32::from_le_bytes(dds[16..20].try_into().unwrap());
    let mut fourcc = [0u8; 4];
    fourcc.copy_from_slice(&dds[84..88]);

    let payload_size = dds.len() - DDS_HEADER_SIZE;

    let block_size = match &fourcc {
        b"DXT1" => 8,
        b"DXT5" | b"ATI2" => 16,
        _ => 0,
    };
    let mip0_size = if block_size > 0 {
        ((width as usize + 3) / 4) * ((height as usize + 3) / 4) * block_size
    } else {
        0
    };

    Ok(DdsInfo {
        mip_map_count, width, height, fourcc, payload_size, mip0_size,
        iog1_fmt_tag: stream.fmt_tag,
        iog1_mip_count: stream.mip_sizes.len(),
        iog1_mip_sizes: stream.mip_sizes,
    })
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

    // Check DDS mip count: if dwMipMapCount > 1, the DDS contains pre-generated
    // mips that might be used verbatim instead of regenerating.
    let dds_mip_count = if stream1.dds_data.len() >= 32 {
        u32::from_le_bytes(stream1.dds_data[28..32].try_into().unwrap())
    } else { 0 };
    let dds_payload_size = mip0_data.len();
    let _dds_has_extra_mips = dds_payload_size > mip_sizes[0];

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

    // If the DDS contains multiple mip levels (dwMipMapCount > 1 and payload > mip0),
    // use the pre-generated mips from the DDS instead of regenerating.
    if dds_mip_count as usize > 1 && dds_payload_size > mip_sizes[0] {
        // DDS has pre-generated mips — use them directly
        let mut all_mips: Vec<Vec<u8>> = Vec::with_capacity(mip_count);
        let mut dds_off = 0;
        for idx in 0..mip_count {
            let sz = mip_sizes[idx];
            if dds_off + sz <= dds_payload_size {
                all_mips.push(mip0_data[dds_off..dds_off + sz].to_vec());
                dds_off += sz;
            } else {
                // DDS ran out of mip data — generate remaining from last available
                break;
            }
        }
        // If we got all mips from the DDS, write them out
        if all_mips.len() == mip_count {
            let mut output = Vec::with_capacity(decomp_size);
            output.extend_from_slice(fctx_hdr);
            for mip in all_mips.iter().rev() {
                output.extend_from_slice(mip);
            }
            if output.len() < decomp_size {
                output.resize(decomp_size, 0);
            }
            return Ok(output);
        }
        // Otherwise fall through to mip generation
    }

    // ── 5. binaryAlpha flag ──────────────────────────────────────────
    // The original encoder has a `binaryAlpha` flag but it's NOT based on
    // scanning DXT1 blocks for 3-color mode — 99.8% of DXT1 textures have at
    // least one c0<=c1 block (degenerate solid-color blocks), so block scanning
    // triggers on virtually everything. The flag is likely a content-pipeline
    // metadata flag we don't have access to. Disabled until we can determine
    // the correct detection method.
    let has_binary_alpha = false;

    // ── 6. Generate all mips, then write SMALLEST-FIRST ────────────
    // The FCTX format stores mips smallest-first (1x1 at start, full mip0 at end).
    //
    // CRITICAL: The original cascades resizes in uncompressed pixel space.
    // It decodes mip0 ONCE, then repeatedly box-filters the pixel buffer
    // for each mip level, encoding only at output time. It does NOT
    // re-decode the DXT1 blocks between mip levels.
    let mut all_mips: Vec<Vec<u8>> = vec![mip0_data[..mip_sizes[0]].to_vec()];
    let mut current_w = stream1.width;
    let mut current_h = stream1.height;

    // Decode mip0 to pixel buffer, convert to f32 for x87 float cascading.
    // The original keeps an f32 planar buffer across mip levels, using x87
    // 80-bit precision for the box filter (fadd chain), and only converts
    // to uint8 (via fistp round-to-nearest-even) at the encoding step.
    let block_size = codec.block_size();
    let prev_bx = (current_w / 4).max(1);
    let prev_by = (current_h / 4).max(1);
    let mut decoded_u8: Vec<[u8; 4]> = vec![[0u8; 4]; current_w * current_h];
    for by in 0..prev_by {
        for bx in 0..prev_bx {
            let block_off = (by * prev_bx + bx) * block_size;
            if block_off + block_size > mip_sizes[0] { continue; }
            let block = &mip0_data[block_off..block_off + block_size];
            let block_pixels = match codec {
                TextureCodec::Dxt1 => decode_dxt1_block(block),
                TextureCodec::Dxt5 => decode_dxt5_block(block),
                TextureCodec::Ati2 => decode_ati2_block(block),
            };
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x < current_w && y < current_h {
                        decoded_u8[y * current_w + x] = block_pixels[py * 4 + px];
                    }
                }
            }
        }
    }

    // Convert to f32 [0,1] and gamma-linearize channels 0-2.
    // The original always applies gamma=2.2 linearization (FUN_677FC0) before
    // box filtering, confirmed via Frida trace: FUN_6554E0 always sets gamma=2.2
    // and FUN_677FC0 is called once per unique texture (511/511 = 100%).
    let recip255: f32 = 1.0 / 255.0;
    let gamma: f32 = 2.2;
    let mut current_float: Vec<[f32; 4]> = decoded_u8.iter().map(|p| {
        [x87_powf(p[0] as f32 * recip255, gamma),
         x87_powf(p[1] as f32 * recip255, gamma),
         x87_powf(p[2] as f32 * recip255, gamma),
         p[3] as f32 * recip255]
    }).collect();

    for mip_idx in 1..mip_count {
        let new_w = (current_w / 2).max(1);
        let new_h = (current_h / 2).max(1);

        // x87 box filter in float space (keeps f32 for cascading)
        let mut new_float = vec![[0.0f32; 4]; new_w * new_h];
        for y in 0..new_h {
            for x in 0..new_w {
                let x0 = (x * 2).min(current_w - 1);
                let y0 = (y * 2).min(current_h - 1);
                let x1 = (x * 2 + 1).min(current_w - 1);
                let y1 = (y * 2 + 1).min(current_h - 1);
                for c in 0..4 {
                    new_float[y * new_w + x][c] = x87_box_filter_f32(
                        current_float[y0 * current_w + x0][c],
                        current_float[y0 * current_w + x1][c],
                        current_float[y1 * current_w + x0][c],
                        current_float[y1 * current_w + x1][c],
                    );
                }
            }
        }

        // Convert to uint8 for encoding: degamma channels 0-2 via
        // pow(val, 1/2.2) * 255, floor (matching FUN_679220 + FUN_681C10).
        // Channel 3 (alpha) uses direct floor(val * 255) (matching FUN_679070).
        let inv_gamma: f32 = 1.0f32 / 2.2f32;
        let scale: f32 = 255.0;
        let new_bx = (new_w / 4).max(1);
        let new_by = (new_h / 4).max(1);
        let mut mip_data = Vec::with_capacity(new_bx * new_by * block_size);
        for by in 0..new_by {
            for bx in 0..new_bx {
                let mut block_pixels = [[0u8; 4]; 16];
                for py in 0..4 {
                    for px in 0..4 {
                        let fx = (bx * 4 + px).min(new_w - 1);
                        let fy = (by * 4 + py).min(new_h - 1);
                        let fv = &new_float[fy * new_w + fx];
                        block_pixels[py * 4 + px] = [
                            x87_pow_scale_floor(fv[0], inv_gamma, scale).clamp(0, 255) as u8,
                            x87_pow_scale_floor(fv[1], inv_gamma, scale).clamp(0, 255) as u8,
                            x87_pow_scale_floor(fv[2], inv_gamma, scale).clamp(0, 255) as u8,
                            x87_float_to_u8(fv[3]),
                        ];
                    }
                }
                match codec {
                    TextureCodec::Dxt1 => mip_data.extend_from_slice(&encode_dxt1_block(&block_pixels, has_binary_alpha, true)),
                    TextureCodec::Dxt5 => mip_data.extend_from_slice(&encode_dxt5_block(&block_pixels)),
                    TextureCodec::Ati2 => mip_data.extend_from_slice(&encode_ati2_block(&block_pixels)),
                }
            }
        }

        if mip_data.len() != mip_sizes[mip_idx] {
            return Err(format!(
                "Mip {} size mismatch: generated {} bytes, expected {}",
                mip_idx,
                mip_data.len(),
                mip_sizes[mip_idx]
            ));
        }

        all_mips.push(mip_data);
        current_float = new_float;
        current_w = new_w;
        current_h = new_h;
    }

    // Write mips smallest-first (reverse order: mip[N-1] first, mip0 last)
    let mut output = Vec::with_capacity(decomp_size);
    output.extend_from_slice(fctx_hdr);
    for mip in all_mips.iter().rev() {
        output.extend_from_slice(mip);
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
            let nw = (cw / 2).max(1);
            let nh = (ch / 2).max(1);
            let mip = generate_mip(&ati2_mips[mip_idx - 1], cw, ch, nw, nh, TextureCodec::Ati2, false);
            ati2_mips.push(mip);
            cw = nw;
            ch = nh;
        }

        // Assemble per-mip grouped output (verified layout, same as format_enum=6):
        // Per mip (smallest first): [6-byte prefix × blocks] then [16-byte ATI2 × blocks]
        let mut output = Vec::with_capacity(decomp_size);
        output.extend_from_slice(fctx_hdr);

        for mip in ati2_mips.iter().rev() {
            let num_blocks = mip.len() / 16;
            // 6-byte prefix section (zeros) for this mip
            output.resize(output.len() + num_blocks * 6, 0);
            // 16-byte ATI2 section for this mip
            output.extend_from_slice(mip);
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
        let nw = (cw / 2).max(1);
        let nh = (ch / 2).max(1);
        let mip = generate_mip(&ati2_mips[mip_idx - 1], cw, ch, nw, nh, TextureCodec::Ati2, false);
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
        let nw = (cw / 2).max(1);
        let nh = (ch / 2).max(1);
        let mip = generate_mip_bc4(&bc4_mips[mip_idx - 1], cw, ch, nw, nh);
        bc4_mips.push(mip);
        cw = nw;
        ch = nh;
    }

    // -- Assemble output in per-mip grouped layout --
    // The game reads per-mip, NOT as global planes. Each mip has its prefix section
    // (6 bytes/block) grouped together, then its ATI2 section (16 bytes/block).
    //
    // Layout:
    //   FCTX header (24 bytes)
    //   Per mip (smallest first):
    //     [6-byte prefix × blocks_in_mip]  (zeros — BC4 index placeholder)
    //     [16-byte ATI2 × blocks_in_mip]   (normal map data for ALL mips)
    //   "ATI1" tag (4 bytes)
    //   BC4 gloss (8 bytes/block) for ALL mips, smallest-first
    let mut output = Vec::with_capacity(decomp_size);
    output.extend_from_slice(fctx_hdr);

    // Per-mip interleaved sections (smallest mip first = ati2_mips reversed)
    for mip in ati2_mips.iter().rev() {
        let num_blocks = mip.len() / 16;
        // 6-byte prefix section (zeros) for this mip
        output.resize(output.len() + num_blocks * 6, 0);
        // 16-byte ATI2 section for this mip
        output.extend_from_slice(mip);
    }

    // "ATI1" tag
    output.extend_from_slice(b"ATI1");

    // BC4 gloss full blocks (8B each) for all mips, smallest-first
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

/// Convert sRGB byte value [0,255] to linear float [0,1].
/// Currently unused — gamma correction did not improve matching
/// (the original may not gamma-correct during mip filtering).
#[allow(dead_code)]
#[inline]
fn srgb_to_linear(s: u8) -> f32 {
    (s as f32 / 255.0).powf(2.2)
}

/// Convert linear float [0,1] back to sRGB byte [0,255].
#[allow(dead_code)]
#[inline]
fn linear_to_srgb(l: f32) -> u8 {
    (l.powf(1.0 / 2.2) * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// Generate a downsampled mip level from DXT block data.
///
/// Matches the game's mip generation pipeline:
/// x87-precision pow(base, exp) matching the original's FUN_682EE0.
///
/// Uses the standard x87 sequence: pow(x, y) = 2^(y * log2(x))
/// via FYL2X + FRNDINT + F2XM1 + FSCALE, all at 80-bit precision.
/// The result is stored as f32 to match the original's fstp dword.
fn x87_powf(base: f32, exp: f32) -> f32 {
    // For base <= 0, return 0 (gamma only applies to positive pixel values)
    if base <= 0.0 {
        return 0.0;
    }
    // x87 pow using FYL2X + F2XM1 + FSCALE, matching FUN_682EE0.
    // Uses CW=0x027F (53-bit, MSVC default) to match the original's precision.
    // FUN_682EE0 does NOT set the CW — it inherits the MSVC default.
    let mut result: f32 = 0.0;
    let cw: u16 = 0x027F; // 53-bit precision (MSVC default), NOT 0x037F
    unsafe {
        std::arch::asm!(
            "fldcw word ptr [{cw}]",
            "fld dword ptr [{exp}]",    // st(0) = exp
            "fld dword ptr [{base}]",   // st(0) = base, st(1) = exp
            "fyl2x",                     // st(0) = exp * log2(base)
            "fld st(0)",                 // st(0) = st(1) = exp*log2(base)
            "frndint",                   // st(0) = int part
            "fsub st(1), st(0)",         // st(1) = frac part
            "fxch st(1)",               // st(0) = frac, st(1) = int
            "f2xm1",                     // st(0) = 2^frac - 1
            "fld1",                      // st(0) = 1.0
            "faddp st(1), st(0)",       // st(0) = 2^frac
            "fscale",                    // st(0) = 2^frac * 2^int = base^exp
            "fstp st(1)",               // pop int part
            "fstp dword ptr [{out}]",   // store result as f32
            cw = in(reg) &cw,
            exp = in(reg) &exp,
            base = in(reg) &base,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// x87 pow(base, exp) * scale → floor → i32, all at 80-bit precision.
/// Matches the original's FUN_682EE0 → fmul 255 → FUN_681c10 chain.
#[inline(never)]
fn x87_pow_scale_floor(base: f32, exp: f32, scale: f32) -> i32 {
    if base <= 0.0 {
        return 0;
    }
    let mut result: i32 = 0;
    let cw_ext: u16 = 0x027F;  // 53-bit precision (MSVC default)
    let cw_trunc: u16 = 0x0E7F; // 53-bit precision, truncation (round toward zero)
    unsafe {
        std::arch::asm!(
            "fldcw word ptr [{cw_ext}]",
            "fld dword ptr [{exp}]",
            "fld dword ptr [{base}]",
            "fyl2x",
            "fld st(0)",
            "frndint",
            "fsub st(1), st(0)",
            "fxch st(1)",
            "f2xm1",
            "fld1",
            "faddp st(1), st(0)",
            "fscale",
            "fstp st(1)",
            // Now st(0) = base^exp at 80-bit precision
            "fmul dword ptr [{scale}]",  // st(0) = base^exp * scale
            // Floor via truncation mode
            "fldcw word ptr [{cw_trunc}]",
            "fistp dword ptr [{out}]",
            "fldcw word ptr [{cw_ext}]",  // restore
            cw_ext = in(reg) &cw_ext,
            cw_trunc = in(reg) &cw_trunc,
            exp = in(reg) &exp,
            base = in(reg) &base,
            scale = in(reg) &scale,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// NVTT's fast pow(x, 11/5) approximation for gamma 2.2 linearization.
/// Splits float into exponent + mantissa, uses a lookup table for pow(2^e, 2.2)
/// and a degree-4 minimax polynomial for pow(mantissa, 2.2).
/// Relative error < 2.9e-6.
fn nvtt_powf_11_5(x: f32) -> f32 {
    // pow(2.0, e * 11/5.0) over e=[-127,128], indexed by [sign|exponent] bits
    static TABLE: [f32; 512] = [
        // sign bit = 0
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 0.00000000e+00,
        0.00000000e+00, 0.00000000e+00, 0.00000000e+00, 1.40129846e-45,
        4.20389539e-45, 1.96181785e-44, 8.96831017e-44, 4.11981749e-43,
        1.89315423e-42, 8.69926087e-42, 3.99734400e-41, 1.83670992e-40,
        8.43930599e-40, 3.87768572e-39, 1.78171625e-38, 8.18661824e-38,
        3.76158192e-37, 1.72836915e-36, 7.94149964e-36, 3.64895487e-35,
        1.67661942e-34, 7.70371978e-34, 3.53970002e-33, 1.62641913e-32,
        7.47305957e-32, 3.43371656e-31, 1.57772181e-30, 7.24930563e-30,
        3.33090637e-29, 1.53048260e-28, 7.03225152e-28, 3.23117427e-27,
        1.48465779e-26, 6.82169625e-26, 3.13442837e-25, 1.44020511e-24,
        6.61744490e-24, 3.04057916e-23, 1.39708339e-22, 6.41930929e-22,
        2.94954007e-21, 1.35525272e-20, 6.22710612e-20, 2.86122679e-19,
        1.31467454e-18, 6.04065806e-18, 2.77555756e-17, 1.27531133e-16,
        5.85979246e-16, 2.69245347e-15, 1.23712677e-14, 5.68434189e-14,
        2.61183761e-13, 1.20008550e-12, 5.51414470e-12, 2.53363563e-11,
        1.16415322e-10, 5.34904343e-10, 2.45777509e-09, 1.12929683e-08,
        5.18888577e-08, 2.38418579e-07, 1.09548409e-06, 5.03352339e-06,
        2.31279992e-05, 1.06268380e-04, 4.88281250e-04, 2.24355143e-03,
        1.03086559e-02, 4.73661423e-02, 2.17637643e-01, 1.00000000e+00,
        4.59479332e+00, 2.11121273e+01, 9.70058594e+01, 4.45721893e+02,
        2.04800000e+03, 9.41013672e+03, 4.32376367e+04, 1.98668000e+05,
        9.12838438e+05, 4.19430400e+06, 1.92719600e+07, 8.85506800e+07,
        4.06872064e+08, 1.86949312e+09, 8.58993459e+09, 3.94689741e+10,
        1.81351793e+11, 8.33273987e+11, 3.82872191e+12, 1.75921860e+13,
        8.08324589e+13, 3.71408471e+14, 1.70654513e+15, 7.84122247e+15,
        3.60287970e+16, 1.65544876e+17, 7.60644549e+17, 3.49500442e+18,
        1.60588236e+19, 7.37869763e+19, 3.39035906e+20, 1.55780004e+21,
        7.15776905e+21, 3.28884708e+22, 1.51115727e+23, 6.94345535e+23,
        3.19037448e+24, 1.46591110e+25, 6.73555881e+25, 3.09485010e+26,
        1.42201966e+27, 6.53388693e+27, 3.00218593e+28, 1.37944245e+29,
        6.33825300e+29, 2.91229625e+30, 1.33814004e+31, 6.14847679e+31,
        2.82509813e+32, 1.29807421e+33, 5.96438273e+33, 2.74051081e+34,
        1.25920805e+35, 5.78580097e+35, 2.65845599e+36, 1.22150558e+37,
        5.61256613e+37, 2.57885808e+38, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        f32::INFINITY, f32::INFINITY, f32::INFINITY, f32::INFINITY,
        // sign bit = 1 (all zeros)
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ];

    let bits = x.to_bits();
    let k = (bits >> 23) as usize; // [sign|exponent] bits = 0..511
    let m_bits = (bits & 0x007F_FFFF) | (127 << 23);
    let m = f32::from_bits(m_bits);

    let pow_e = TABLE[k];
    // Minimax polynomial for pow(m, 11/5) over m=[1,2)
    let pow_m = (((-0.00916587552f32 * m + 0.119315466) * m + 1.01847068) * m - 0.158338739) * m + 0.0297184721;
    pow_e * pow_m
}

/// NVTT's fast pow(x, 5/11) approximation for gamma 1/2.2 de-linearization.
/// Splits float into exponent + mantissa, uses a lookup table for pow(2^e, 1/2.2)
/// and a degree-4 minimax polynomial for pow(mantissa, 1/2.2).
/// Relative error < 1.2e-5.
fn nvtt_powf_5_11(x: f32) -> f32 {
    // pow(2.0, e * 5/11.0) over e=[-127,128], indexed by [sign|exponent] bits
    static TABLE: [f32; 512] = [
        // sign bit = 0
        0.00000000e+00, 5.74369237e-18, 7.87087416e-18, 1.07858603e-17,
        1.47804139e-17, 2.02543544e-17, 2.77555756e-17, 3.80348796e-17,
        5.21211368e-17, 7.14242467e-17, 9.78762916e-17, 1.34124875e-16,
        1.83798156e-16, 2.51867973e-16, 3.45147530e-16, 4.72973245e-16,
        6.48139341e-16, 8.88178420e-16, 1.21711615e-15, 1.66787638e-15,
        2.28557589e-15, 3.13204133e-15, 4.29199599e-15, 5.88154098e-15,
        8.05977514e-15, 1.10447209e-14, 1.51351438e-14, 2.07404589e-14,
        2.84217094e-14, 3.89477167e-14, 5.33720441e-14, 7.31384286e-14,
        1.00225323e-13, 1.37343872e-13, 1.88209311e-13, 2.57912805e-13,
        3.53431070e-13, 4.84324603e-13, 6.63694685e-13, 9.09494702e-13,
        1.24632693e-12, 1.70790541e-12, 2.34042972e-12, 3.20721032e-12,
        4.39500389e-12, 6.02269797e-12, 8.25320975e-12, 1.13097942e-11,
        1.54983873e-11, 2.12382299e-11, 2.91038305e-11, 3.98824619e-11,
        5.46529731e-11, 7.48937509e-11, 1.02630730e-10, 1.40640125e-10,
        1.92726335e-10, 2.64102712e-10, 3.61913416e-10, 4.95948393e-10,
        6.79623358e-10, 9.31322575e-10, 1.27623878e-09, 1.74889514e-09,
        2.39660003e-09, 3.28418337e-09, 4.50048399e-09, 6.16724272e-09,
        8.45128678e-09, 1.15812293e-08, 1.58703486e-08, 2.17479474e-08,
        2.98023224e-08, 4.08396410e-08, 5.59646445e-08, 7.66912009e-08,
        1.05093868e-07, 1.44015488e-07, 1.97351767e-07, 2.70441177e-07,
        3.70599338e-07, 5.07851155e-07, 6.95934318e-07, 9.53674316e-07,
        1.30686851e-06, 1.79086862e-06, 2.45411843e-06, 3.36300377e-06,
        4.60849560e-06, 6.31525654e-06, 8.65411766e-06, 1.18591788e-05,
        1.62512370e-05, 2.22698982e-05, 3.05175781e-05, 4.18197924e-05,
        5.73077959e-05, 7.85317898e-05, 1.07616121e-04, 1.47471859e-04,
        2.02088209e-04, 2.76931765e-04, 3.79493722e-04, 5.20039583e-04,
        7.12636742e-04, 9.76562500e-04, 1.33823336e-03, 1.83384947e-03,
        2.51301727e-03, 3.44371586e-03, 4.71909950e-03, 6.46682270e-03,
        8.86181649e-03, 1.21437991e-02, 1.66412666e-02, 2.28043757e-02,
        3.12500000e-02, 4.28234674e-02, 5.86831830e-02, 8.04165527e-02,
        1.10198908e-01, 1.51011184e-01, 2.06938326e-01, 2.83578128e-01,
        3.88601571e-01, 5.32520533e-01, 7.29740024e-01, 1.00000000e+00,
        1.37035096e+00, 1.87786186e+00, 2.57332969e+00, 3.52636504e+00,
        4.83235788e+00, 6.62202644e+00, 9.07450008e+00, 1.24352503e+01,
        1.70406570e+01, 2.33516808e+01, 3.20000000e+01, 4.38512306e+01,
        6.00915794e+01, 8.23465500e+01, 1.12843681e+02, 1.54635452e+02,
        2.11904846e+02, 2.90384003e+02, 3.97928009e+02, 5.45301025e+02,
        7.47253784e+02, 1.02400000e+03, 1.40323938e+03, 1.92293054e+03,
        2.63508960e+03, 3.61099780e+03, 4.94833447e+03, 6.78095508e+03,
        9.29228809e+03, 1.27336963e+04, 1.74496328e+04, 2.39121211e+04,
        3.27680000e+04, 4.49036602e+04, 6.15337773e+04, 8.43228672e+04,
        1.15551930e+05, 1.58346703e+05, 2.16990563e+05, 2.97353219e+05,
        4.07478281e+05, 5.58388250e+05, 7.65187875e+05, 1.04857600e+06,
        1.43691713e+06, 1.96908088e+06, 2.69833175e+06, 3.69766175e+06,
        5.06709450e+06, 6.94369800e+06, 9.51530300e+06, 1.30393050e+07,
        1.78684240e+07, 2.44860120e+07, 3.35544320e+07, 4.59813480e+07,
        6.30105880e+07, 8.63466160e+07, 1.18325176e+08, 1.62147024e+08,
        2.22198336e+08, 3.04489696e+08, 4.17257760e+08, 5.71789568e+08,
        7.83552384e+08, 1.07374182e+09, 1.47140314e+09, 2.01633882e+09,
        2.76309171e+09, 3.78640563e+09, 5.18870477e+09, 7.11034675e+09,
        9.74367027e+09, 1.33522483e+10, 1.82972662e+10, 2.50736763e+10,
        3.43597384e+10, 4.70849004e+10, 6.45228421e+10, 8.84189348e+10,
        1.21164980e+11, 1.66038553e+11, 2.27531096e+11, 3.11797449e+11,
        4.27271946e+11, 5.85512518e+11, 8.02357641e+11, 1.09951163e+12,
        1.50671681e+12, 2.06473095e+12, 2.82940591e+12, 3.87727937e+12,
        5.31323368e+12, 7.28099507e+12, 9.97751836e+12, 1.36727023e+13,
        1.87364006e+13, 2.56754445e+13, 3.51843721e+13, 4.82149380e+13,
        6.60713903e+13, 9.05409892e+13, 1.24072940e+14, 1.70023478e+14,
        2.32991842e+14, 3.19280587e+14, 4.37526473e+14, 5.99564818e+14,
        8.21614225e+14, 1.12589991e+15, 1.54287801e+15, 2.11428449e+15,
        2.89731166e+15, 3.97033407e+15, 5.44075129e+15, 7.45573896e+15,
        1.02169788e+16, 1.40008471e+16, 1.91860742e+16, 2.62916552e+16,
        3.60287970e+16, 4.93720965e+16, 6.76571037e+16, 9.27139730e+16,
        1.27050690e+17, 1.74104041e+17, 2.38583647e+17, f32::INFINITY,
        // sign bit = 1 (all zeros)
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ];

    let bits = x.to_bits();
    let k = (bits >> 23) as usize;
    let m_bits = (bits & 0x007F_FFFF) | (127 << 23);
    let m = f32::from_bits(m_bits);

    let pow_e = TABLE[k];
    // Minimax polynomial for pow(m, 5/11) over m=[1,2)
    let pow_m = (((-0.0110083047f32 * m + 0.0905038750) * m - 0.324697506) * m + 0.876040946) * m + 0.369160989;
    pow_e * pow_m
}

/// 1. Decode entire source mip to full pixel buffer
/// 2. Box-filter in float space (2x2 average with rounding)
/// 3. Re-encode block by block from filtered image
fn generate_mip(
    prev: &[u8],
    prev_w: usize,
    prev_h: usize,
    new_w: usize,
    new_h: usize,
    codec: TextureCodec,
    force_3color: bool,
) -> Vec<u8> {
    let block_size = codec.block_size();
    let prev_bx = (prev_w / 4).max(1);
    let prev_by = (prev_h / 4).max(1);
    let new_bx = (new_w / 4).max(1);
    let new_by = (new_h / 4).max(1);

    // Step 1: Decode entire source mip to pixel buffer
    let mut src_pixels = vec![[0u8; 4]; prev_w * prev_h];
    for by in 0..prev_by {
        for bx in 0..prev_bx {
            let block_off = (by * prev_bx + bx) * block_size;
            if block_off + block_size > prev.len() { continue; }
            let block = &prev[block_off..block_off + block_size];
            let block_pixels = match codec {
                TextureCodec::Dxt1 => decode_dxt1_block(block),
                TextureCodec::Dxt5 => decode_dxt5_block(block),
                TextureCodec::Ati2 => decode_ati2_block(block),
            };
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x < prev_w && y < prev_h {
                        src_pixels[y * prev_w + x] = block_pixels[py * 4 + px];
                    }
                }
            }
        }
    }

    // Step 2: Box-filter with truncating integer division.
    let mut dst_pixels = vec![[0u8; 4]; new_w * new_h];
    for y in 0..new_h {
        for x in 0..new_w {
            let sx = x * 2;
            let sy = y * 2;
            let x0 = sx.min(prev_w - 1);
            let y0 = sy.min(prev_h - 1);
            let x1 = (sx + 1).min(prev_w - 1);
            let y1 = (sy + 1).min(prev_h - 1);

            let p00 = src_pixels[y0 * prev_w + x0];
            let p10 = src_pixels[y0 * prev_w + x1];
            let p01 = src_pixels[y1 * prev_w + x0];
            let p11 = src_pixels[y1 * prev_w + x1];

            for c in 0..4 {
                let sum = p00[c] as u32 + p10[c] as u32 + p01[c] as u32 + p11[c] as u32;
                dst_pixels[y * new_w + x][c] = (sum / 4) as u8;
            }
        }
    }

    // Step 3: Re-encode block by block from filtered image
    let mut result = Vec::with_capacity(new_bx * new_by * block_size);
    for by in 0..new_by {
        for bx in 0..new_bx {
            let mut block_pixels = [[0u8; 4]; 16];
            for py in 0..4 {
                for px in 0..4 {
                    let x = (bx * 4 + px).min(new_w - 1);
                    let y = (by * 4 + py).min(new_h - 1);
                    block_pixels[py * 4 + px] = dst_pixels[y * new_w + x];
                }
            }
            match codec {
                TextureCodec::Dxt1 => result.extend_from_slice(&encode_dxt1_block(&block_pixels, force_3color, true)),
                TextureCodec::Dxt5 => result.extend_from_slice(&encode_dxt5_block(&block_pixels)),
                TextureCodec::Ati2 => result.extend_from_slice(&encode_ati2_block(&block_pixels)),
            }
        }
    }

    result
}

/// Generate a downsampled BC4 mip level from BC4 block data.
/// Generate a mip level from an UNCOMPRESSED pixel buffer.
///
/// Matches the original's pipeline: the resize operates on uncompressed pixels
/// (not re-decoded DXT1). Returns (encoded_blocks, filtered_pixels) so the
/// filtered pixels can be passed to the next mip level without re-encoding.
/// x87 box filter for float cascading: sum 4 f32 values at 53-bit precision
/// (MSVC default CW=0x027F), multiply by 0.25, store result as f32.
/// Matches the original's FUN_677FE0 which is MSVC-compiled code running
/// at the default control word, NOT 80-bit extended precision.
#[inline(never)]
fn x87_box_filter_f32(f0: f32, f1: f32, f2: f32, f3: f32) -> f32 {
    let quarter: f32 = 0.25;
    let mut result: f32 = 0.0;
    let cw: u16 = 0x027F; // 53-bit precision (MSVC default), NOT 0x037F
    unsafe {
        std::arch::asm!(
            "fldcw word ptr [{cw}]",
            "fld dword ptr [{f0}]",
            "fadd dword ptr [{f1}]",
            "fadd dword ptr [{f2}]",
            "fadd dword ptr [{f3}]",
            "fmul dword ptr [{quarter}]",
            "fstp dword ptr [{out}]",
            cw = in(reg) &cw,
            f0 = in(reg) &f0,
            f1 = in(reg) &f1,
            f2 = in(reg) &f2,
            f3 = in(reg) &f3,
            quarter = in(reg) &quarter,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result
}

/// x87 float→uint8: multiply by 255, floor (truncation).
/// Matches the original's FUN_679070 + FUN_681c10 which uses floor, NOT round-to-nearest.
#[inline(never)]
fn x87_float_to_u8(val: f32) -> u8 {
    let scale: f32 = 255.0;
    let mut result: i32 = 0;
    let cw_ext: u16 = 0x037F;   // 80-bit precision, round-to-nearest
    let cw_trunc: u16 = 0x0F7F; // 80-bit precision, truncation
    unsafe {
        std::arch::asm!(
            "fldcw word ptr [{cw_ext}]",
            "fld dword ptr [{val}]",
            "fmul dword ptr [{scale}]",
            "fldcw word ptr [{cw_trunc}]",
            "fistp dword ptr [{out}]",
            "fldcw word ptr [{cw_ext}]",
            cw_ext = in(reg) &cw_ext,
            cw_trunc = in(reg) &cw_trunc,
            val = in(reg) &val,
            scale = in(reg) &scale,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result.clamp(0, 255) as u8
}

/// Generate a mip from LINEAR float pixel data (gamma-correct pipeline).
/// Box-filters in linear space, de-linearizes to uint8 for encoding,
/// returns linear float data for cascading to next mip level.
fn generate_mip_from_linear(
    src: &[[f32; 4]],
    prev_w: usize, prev_h: usize,
    new_w: usize, new_h: usize,
    codec: TextureCodec, force_3color: bool,
) -> (Vec<u8>, Vec<[f32; 4]>) {
    let block_size = codec.block_size();
    let new_bx = (new_w / 4).max(1);
    let new_by = (new_h / 4).max(1);

    // Box filter in linear float space — x87 80-bit precision matching FUN_677FE0
    let mut dst_lin = vec![[0.0f32; 4]; new_w * new_h];
    for y in 0..new_h {
        for x in 0..new_w {
            let x0 = (x * 2).min(prev_w - 1);
            let y0 = (y * 2).min(prev_h - 1);
            let x1 = (x * 2 + 1).min(prev_w - 1);
            let y1 = (y * 2 + 1).min(prev_h - 1);
            for c in 0..4 {
                dst_lin[y * new_w + x][c] = x87_box_filter_f32(
                    src[y0*prev_w+x0][c], src[y0*prev_w+x1][c],
                    src[y1*prev_w+x0][c], src[y1*prev_w+x1][c],
                );
            }
        }
    }

    // De-linearize to uint8: pow(val, 1/2.2) * 255.0, floor.
    // Uses x87 for the full chain matching FUN_679220 → FUN_681C10 (floor).
    // Tested rounding (10.60%) — floor (10.90%) is better.
    let inv_gamma: f32 = 1.0f32 / 2.2f32;
    let scale: f32 = 255.0;
    let mut dst_u8 = vec![[0u8; 4]; new_w * new_h];
    for i in 0..new_w * new_h {
        for c in 0..3 {
            let v = x87_pow_scale_floor(dst_lin[i][c], inv_gamma, scale);
            dst_u8[i][c] = v.clamp(0, 255) as u8;
        }
        dst_u8[i][3] = x87_float_to_u8(dst_lin[i][3]);
    }

    // Encode
    let mut result = Vec::with_capacity(new_bx * new_by * block_size);
    for by in 0..new_by {
        for bx in 0..new_bx {
            let mut bp = [[0u8; 4]; 16];
            for py in 0..4 { for px in 0..4 {
                bp[py*4+px] = dst_u8[((by*4+py).min(new_h-1))*new_w + (bx*4+px).min(new_w-1)];
            }}
            match codec {
                TextureCodec::Dxt1 => result.extend_from_slice(&encode_dxt1_block(&bp, force_3color, true)),
                TextureCodec::Dxt5 => result.extend_from_slice(&encode_dxt5_block(&bp)),
                TextureCodec::Ati2 => result.extend_from_slice(&encode_ati2_block(&bp)),
            }
        }
    }
    (result, dst_lin)
}

/// x87 box filter: sum 4 f32 values at 80-bit precision, multiply by 0.25 * 255,
/// and convert to u8 via floor (truncation).
/// Matches the original's FUN_677FE0 + FUN_679070 (floor via FUN_681c10) pipeline.
/// The original uses floor (truncation), NOT fistp round-to-nearest.
#[inline(never)]
fn x87_box_filter_u8(f0: f32, f1: f32, f2: f32, f3: f32) -> u8 {
    let quarter: f32 = 0.25;
    let scale: f32 = 255.0;
    let mut result: i32 = 0;
    let cw_ext: u16 = 0x037F;   // 80-bit precision, round-to-nearest
    let cw_trunc: u16 = 0x0F7F; // 80-bit precision, truncation (round toward zero)
    unsafe {
        std::arch::asm!(
            "fldcw word ptr [{cw_ext}]",
            "fld dword ptr [{f0}]",
            "fadd dword ptr [{f1}]",
            "fadd dword ptr [{f2}]",
            "fadd dword ptr [{f3}]",
            "fmul dword ptr [{quarter}]",
            "fmul dword ptr [{scale}]",
            // Switch to truncation mode for the floor conversion
            "fldcw word ptr [{cw_trunc}]",
            "fistp dword ptr [{out}]",
            "fldcw word ptr [{cw_ext}]",  // restore
            cw_ext = in(reg) &cw_ext,
            cw_trunc = in(reg) &cw_trunc,
            f0 = in(reg) &f0,
            f1 = in(reg) &f1,
            f2 = in(reg) &f2,
            f3 = in(reg) &f3,
            quarter = in(reg) &quarter,
            scale = in(reg) &scale,
            out = in(reg) &mut result,
            options(nostack),
        );
    }
    result.clamp(0, 255) as u8
}

fn generate_mip_from_pixels(
    src_pixels: &[[u8; 4]],
    prev_w: usize,
    prev_h: usize,
    new_w: usize,
    new_h: usize,
    codec: TextureCodec,
    force_3color: bool,
) -> (Vec<u8>, Vec<[u8; 4]>) {
    let block_size = codec.block_size();
    let new_bx = (new_w / 4).max(1);
    let new_by = (new_h / 4).max(1);

    // Box-filter matching the original's x87 FPU pipeline:
    // The original uses x87 80-bit extended precision for intermediate sums
    // and fistp (round-to-nearest-even) for float→uint8 conversion.
    // This produces slightly different results from f32/integer arithmetic
    // for ~25% of pixel values near the 0.5 boundary.
    let recip255: f32 = 1.0 / 255.0;
    let quarter: f32 = 0.25;
    let scale255: f32 = 255.0;
    let mut dst_pixels = vec![[0u8; 4]; new_w * new_h];
    for y in 0..new_h {
        for x in 0..new_w {
            let sx = x * 2;
            let sy = y * 2;
            let x0 = sx.min(prev_w - 1);
            let y0 = sy.min(prev_h - 1);
            let x1 = (sx + 1).min(prev_w - 1);
            let y1 = (sy + 1).min(prev_h - 1);

            let p00 = src_pixels[y0 * prev_w + x0];
            let p10 = src_pixels[y0 * prev_w + x1];
            let p01 = src_pixels[y1 * prev_w + x0];
            let p11 = src_pixels[y1 * prev_w + x1];

            for c in 0..4 {
                // Match x87 pipeline exactly: fld f32 values, fadd at 80-bit,
                // fmul 0.25, fmul 255.0, fistp (round to nearest even)
                let f0 = p00[c] as f32 * recip255;
                let f1 = p10[c] as f32 * recip255;
                let f2 = p01[c] as f32 * recip255;
                let f3 = p11[c] as f32 * recip255;
                dst_pixels[y * new_w + x][c] = x87_box_filter_u8(f0, f1, f2, f3);
            }
        }
    }

    // Encode block by block from filtered image
    let mut result = Vec::with_capacity(new_bx * new_by * block_size);
    for by in 0..new_by {
        for bx in 0..new_bx {
            let mut block_pixels = [[0u8; 4]; 16];
            for py in 0..4 {
                for px in 0..4 {
                    let x = (bx * 4 + px).min(new_w - 1);
                    let y = (by * 4 + py).min(new_h - 1);
                    block_pixels[py * 4 + px] = dst_pixels[y * new_w + x];
                }
            }
            match codec {
                TextureCodec::Dxt1 => result.extend_from_slice(&encode_dxt1_block(&block_pixels, force_3color, true)),
                TextureCodec::Dxt5 => result.extend_from_slice(&encode_dxt5_block(&block_pixels)),
                TextureCodec::Ati2 => result.extend_from_slice(&encode_ati2_block(&block_pixels)),
            }
        }
    }

    (result, dst_pixels)
}

/// Same float pipeline as generate_mip but for single-channel BC4.
fn generate_mip_bc4(prev: &[u8], prev_w: usize, prev_h: usize, new_w: usize, new_h: usize) -> Vec<u8> {
    let prev_bx = (prev_w / 4).max(1);
    let prev_by = (prev_h / 4).max(1);
    let new_bx = (new_w / 4).max(1);
    let new_by = (new_h / 4).max(1);

    // Step 1: Decode entire source mip to single-channel pixel buffer
    let mut src_pixels = vec![0u8; prev_w * prev_h];
    for by in 0..prev_by {
        for bx in 0..prev_bx {
            let block_off = (by * prev_bx + bx) * 8;
            if block_off + 8 > prev.len() { continue; }
            let block_vals = decode_bc4_block(&prev[block_off..block_off + 8]);
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x < prev_w && y < prev_h {
                        src_pixels[y * prev_w + x] = block_vals[py * 4 + px];
                    }
                }
            }
        }
    }

    // Step 2: Box-filter in float space with rounding
    let mut dst_pixels = vec![0u8; new_w * new_h];
    for y in 0..new_h {
        for x in 0..new_w {
            let sx = x * 2;
            let sy = y * 2;
            let x0 = sx.min(prev_w - 1);
            let y0 = sy.min(prev_h - 1);
            let x1 = (sx + 1).min(prev_w - 1);
            let y1 = (sy + 1).min(prev_h - 1);

            let sum = src_pixels[y0 * prev_w + x0] as f32
                + src_pixels[y0 * prev_w + x1] as f32
                + src_pixels[y1 * prev_w + x0] as f32
                + src_pixels[y1 * prev_w + x1] as f32;
            dst_pixels[y * new_w + x] = (sum / 4.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    // Step 3: Re-encode block by block
    let mut result = Vec::with_capacity(new_bx * new_by * 8);
    for by in 0..new_by {
        for bx in 0..new_bx {
            let mut block_vals = [0u8; 16];
            for py in 0..4 {
                for px in 0..4 {
                    let x = (bx * 4 + px).min(new_w - 1);
                    let y = (by * 4 + py).min(new_h - 1);
                    block_vals[py * 4 + px] = dst_pixels[y * new_w + x];
                }
            }
            result.extend_from_slice(&encode_bc4_block(&block_vals));
        }
    }

    result
}

// ─── DXT1 decode/encode ──────────────────────────────────────────────────────

/// Decode RGB565 → (R, G, B) as u8.
#[inline]
fn decode_rgb565(c: u16) -> (u8, u8, u8) {
    // Bit-replication expansion confirmed in binary at 0x0067C1C3-0x0067C22D.
    let r = ((c >> 11) & 0x1F) as u8;
    let g = ((c >> 5) & 0x3F) as u8;
    let b = (c & 0x1F) as u8;
    (
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
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

    // NVIDIA-style palette interpolation (no +1 rounding).
    // Verified 100% match against reference data for 14,281 samples.
    let palette: [[u8; 4]; 4] = if c0 > c1 {
        [
            [r0, g0, b0, 255],
            [r1, g1, b1, 255],
            [
                ((2 * r0 as u16 + r1 as u16) / 3) as u8,
                ((2 * g0 as u16 + g1 as u16) / 3) as u8,
                ((2 * b0 as u16 + b1 as u16) / 3) as u8,
                255,
            ],
            [
                ((r0 as u16 + 2 * r1 as u16) / 3) as u8,
                ((g0 as u16 + 2 * g1 as u16) / 3) as u8,
                ((b0 as u16 + 2 * b1 as u16) / 3) as u8,
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

/// Encode a solid-color DXT1 block matching the original game's approach:
/// both endpoints set to the nearest RGB565 value, all indices = 0.
fn encode_dxt1_solid(r: u8, g: u8, b: u8) -> [u8; 8] {
    // Quantize to nearest RGB565 using round-to-nearest (matching the original's
    // floor(normalized * scale + 0.5) formula from the Compress4 decompilation)
    let r5 = ((r as u16 * 31 + 127) / 255) as u16;
    let g6 = ((g as u16 * 63 + 127) / 255) as u16;
    let b5 = ((b as u16 * 31 + 127) / 255) as u16;
    let c = (r5 << 11) | (g6 << 5) | b5;

    let mut block = [0u8; 8];
    block[0..2].copy_from_slice(&c.to_le_bytes());
    block[2..4].copy_from_slice(&c.to_le_bytes());
    // indices = 0: all pixels select palette entry 0 (= c0 = c1)
    block
}

/// Build the shared color set for cluster fit: float RGB colors, sqrt-weights,
/// deduplicated by raw byte equality (matching the game's color set construction).
/// Returns (colors, weights, order, n) where colors/weights are sorted along
/// the principal axis and n is the number of unique colors.
fn build_color_set(pixels: &[[u8; 4]; 16], dedup: bool) -> (Vec<[f32; 3]>, Vec<f32>, Vec<usize>, usize) {
    let mut colors: Vec<[f32; 3]> = Vec::with_capacity(16);
    let mut weights: Vec<f32> = Vec::with_capacity(16);
    // Color normalization: multiply by reciprocal constant (matching original's
    // fmul [0.003921569] instruction). IEEE division `x / 255.0` gives a different
    // f32 result than `x * (1/255)` for some pixel values because divss uses the
    // exact divisor while fmul uses the rounded reciprocal.
    const C_1_255: f32 = 1.0 / 255.0; // = 0x3B808081, same constant as original
    const C_1_256: f32 = 1.0 / 256.0; // = 0.00390625, same as original's 0.00390625

    if dedup {
        let mut color_bytes: Vec<[u8; 3]> = Vec::with_capacity(16);
        for (_i, p) in pixels.iter().enumerate() {
            let rgb_bytes = [p[0], p[1], p[2]];
            let rgb = [p[0] as f32 * C_1_255, p[1] as f32 * C_1_255, p[2] as f32 * C_1_255];
            let w = (p[3] as f32 + 1.0) * C_1_256;
            let mut found = false;
            for (j, existing) in color_bytes.iter().enumerate() {
                if *existing == rgb_bytes {
                    weights[j] += w;
                    found = true;
                    break;
                }
            }
            if !found {
                colors.push(rgb);
                color_bytes.push(rgb_bytes);
                weights.push(w);
            }
        }
    } else {
        // No dedup: all 16 pixels as separate entries (matching original's param_4=0)
        for p in pixels.iter() {
            colors.push([p[0] as f32 * C_1_255, p[1] as f32 * C_1_255, p[2] as f32 * C_1_255]);
            weights.push((p[3] as f32 + 1.0) * C_1_256);
        }
    }

    let n = colors.len();

    // Weighted centroid
    let total_weight: f32 = weights.iter().sum();
    let mut centroid = [0.0f32; 3];
    for (c, &w) in colors.iter().zip(weights.iter()) {
        for k in 0..3 { centroid[k] += c[k] * w; }
    }
    for k in 0..3 { centroid[k] /= total_weight; }

    // Set x87 to extended precision for PCA (matching original 32-bit x87 behavior)
    #[cfg(target_arch = "x86_64")]
    let saved_cw = unsafe { x87_set_extended_precision() };

    // Weighted covariance matrix with METRIC WEIGHTS applied to deviations.
    // The original's FUN_00680fe0 applies metric_r/g/b roots to the
    // deviations BEFORE computing the covariance:
    //   dr_w = (color_r - mean_r) * metric_r
    //   cov[rr] += w * dr_w * dr_w
    // With uniform metric (1,1,1) this is the same as without.
    // The metric weights come from FUN_0067d200 param_2/3/4 stored at
    // offsets 0x118/0x11c/0x120 of the encoder object.
    // TODO: determine actual metric values from the caller
    let metric = [1.0f32, 1.0, 1.0]; // metric weight roots
    let mut cov = [0.0f32; 6];
    for (c, &w) in colors.iter().zip(weights.iter()) {
        let dr = (c[0] - centroid[0]) * metric[0];
        let dg = (c[1] - centroid[1]) * metric[1];
        let db = (c[2] - centroid[2]) * metric[2];
        cov[0] += w * dr * dr;
        cov[1] += w * dr * dg;
        cov[2] += w * dr * db;
        cov[3] += w * dg * dg;
        cov[4] += w * dg * db;
        cov[5] += w * db * db;
    }

    // Power iteration — 8 iterations, seed = max-magnitude row
    let rows: [[f32; 3]; 3] = [
        [cov[0], cov[1], cov[2]],
        [cov[1], cov[3], cov[4]],
        [cov[2], cov[4], cov[5]],
    ];
    let mut axis = {
        let mut best_row = 0;
        let mut best_mag = 0.0f32;
        for (i, row) in rows.iter().enumerate() {
            // x87 dot product for magnitude
            let t1 = x87_fma_f32(row[0], row[0], 0.0);
            let t2 = x87_fma_f32(row[1], row[1], 0.0);
            let t3 = x87_fma_f32(row[2], row[2], 0.0);
            let mag = t1 + t2 + t3;
            if mag > best_mag { best_mag = mag; best_row = i; }
        }
        rows[best_row]
    };
    for _ in 0..8 {
        // Matrix-vector product at x87 precision
        let next = [
            x87_fma_f32(cov[0], axis[0], x87_fma_f32(cov[1], axis[1], 0.0)) + cov[2] * axis[2],
            x87_fma_f32(cov[1], axis[0], x87_fma_f32(cov[3], axis[1], 0.0)) + cov[4] * axis[2],
            x87_fma_f32(cov[2], axis[0], x87_fma_f32(cov[4], axis[1], 0.0)) + cov[5] * axis[2],
        ];
        // Normalize by max RAW value (not absolute!) — matching original's FUN_00681700.
        // The original finds the largest component value and divides by it.
        // This preserves direction differently than dividing by max absolute value.
        let mut max_val = next[0];
        if next[0] <= next[1] { max_val = next[1]; }
        if max_val < next[2] { max_val = next[2]; }
        if max_val == 0.0 {
            axis = [0.0; 3];
            break;
        }
        let inv = 1.0f32 / max_val;
        axis = [next[0] * inv, next[1] * inv, next[2] * inv];
    }

    // Project colors onto axis and insertion-sort ascending
    let mut order: Vec<usize> = (0..n).collect();
    let mut dots: Vec<f32> = colors.iter().map(|c| {
        // Dot product at x87 precision for sort-critical projection
        x87_fma_f32(c[0], axis[0], x87_fma_f32(c[1], axis[1], 0.0)) + c[2] * axis[2]
    }).collect();

    #[cfg(target_arch = "x86_64")]
    unsafe { x87_restore(saved_cw); }
    for i in 1..n {
        let key_dot = dots[i];
        let key_idx = order[i];
        let mut j = i;
        while j > 0 && dots[j - 1] > key_dot {
            dots[j] = dots[j - 1];
            order[j] = order[j - 1];
            j -= 1;
        }
        dots[j] = key_dot;
        order[j] = key_idx;
    }
    let sorted_colors: Vec<[f32; 3]> = order.iter().map(|&i| colors[i]).collect();
    let sorted_weights: Vec<f32> = order.iter().map(|&i| weights[i]).collect();

    (sorted_colors, sorted_weights, order, n)
}

/// Compute the actual weighted error of an encoded DXT1 4-color block against
/// the original pixels, using the quantized RGB565 palette.
fn compute_4color_error(pixels: &[[u8; 4]; 16], c0: u16, c1: u16) -> f32 {
    let (r0, g0, b0) = decode_rgb565(c0);
    let (r1, g1, b1) = decode_rgb565(c1);
    let palette: [[f64; 3]; 4] = [
        [r0 as f64 / 255.0, g0 as f64 / 255.0, b0 as f64 / 255.0],
        [r1 as f64 / 255.0, g1 as f64 / 255.0, b1 as f64 / 255.0],
        [
            (2.0 * r0 as f64 + r1 as f64 + 1.5) / (3.0 * 255.0),
            (2.0 * g0 as f64 + g1 as f64 + 1.5) / (3.0 * 255.0),
            (2.0 * b0 as f64 + b1 as f64 + 1.5) / (3.0 * 255.0),
        ],
        [
            (r0 as f64 + 2.0 * r1 as f64 + 1.5) / (3.0 * 255.0),
            (g0 as f64 + 2.0 * g1 as f64 + 1.5) / (3.0 * 255.0),
            (b0 as f64 + 2.0 * b1 as f64 + 1.5) / (3.0 * 255.0),
        ],
    ];
    let mut total_err = 0.0f64;
    for p in pixels.iter() {
        let w = (p[3] as f64 + 1.0) / 256.0;
        let pr = p[0] as f64 / 255.0;
        let pg = p[1] as f64 / 255.0;
        let pb = p[2] as f64 / 255.0;
        let mut best_dist = f64::MAX;
        for entry in &palette {
            let dr = pr - entry[0];
            let dg = pg - entry[1];
            let db = pb - entry[2];
            let dist = dr * dr + dg * dg + db * db;
            if dist < best_dist { best_dist = dist; }
        }
        total_err += w * w * best_dist;
    }
    total_err as f32
}

/// Compute the actual weighted error of an encoded DXT1 3-color block against
/// the original pixels, using the quantized RGB565 palette.
#[allow(dead_code)]
fn compute_3color_error(pixels: &[[u8; 4]; 16], c0: u16, c1: u16) -> f32 {
    let (r0, g0, b0) = decode_rgb565(c0);
    let (r1, g1, b1) = decode_rgb565(c1);
    let palette: [[f64; 3]; 3] = [
        [r0 as f64 / 255.0, g0 as f64 / 255.0, b0 as f64 / 255.0],
        [r1 as f64 / 255.0, g1 as f64 / 255.0, b1 as f64 / 255.0],
        [
            (r0 as f64 + r1 as f64 + 1.0) / (2.0 * 255.0),
            (g0 as f64 + g1 as f64 + 1.0) / (2.0 * 255.0),
            (b0 as f64 + b1 as f64 + 1.0) / (2.0 * 255.0),
        ],
    ];
    let mut total_err = 0.0f64;
    for p in pixels.iter() {
        let w = (p[3] as f64 + 1.0) / 256.0;
        let pr = p[0] as f64 / 255.0;
        let pg = p[1] as f64 / 255.0;
        let pb = p[2] as f64 / 255.0;
        let mut best_dist = f64::MAX;
        for entry in &palette {
            let dr = pr - entry[0];
            let dg = pg - entry[1];
            let db = pb - entry[2];
            let dist = dr * dr + dg * dg + db * db;
            if dist < best_dist { best_dist = dist; }
        }
        total_err += w * w * best_dist;
    }
    total_err as f32
}

/// 4-color ClusterFit encoder.
/// Returns (encoded_block, weighted_error).
// ─── x87 FPU helpers for byte-exact replication ─────────────────────────────
// The original 32-bit encoder uses x87 with 64-bit extended precision (80-bit).
// On x86-64 Windows, the x87 FPU defaults to 53-bit (double) precision.
// These helpers set the FPU to extended precision, matching the original.

/// Set x87 FPU to 64-bit extended precision. Returns the saved control word.
#[cfg(target_arch = "x86_64")]
unsafe fn x87_set_extended_precision() -> u16 {
    let mut save_cw: u16 = 0;
    let mut new_cw: u16 = 0;
    unsafe {
        std::arch::asm!(
            "fnstcw [{save}]",
            "mov ax, [{save}]",
            "or ax, 0x0300",       // set PC bits to 11 (extended precision)
            "mov [{new}], ax",
            "fldcw [{new}]",
            save = in(reg) &mut save_cw,
            new = in(reg) &mut new_cw,
            out("ax") _,
        );
    }
    save_cw
}

/// Restore x87 FPU control word.
#[cfg(target_arch = "x86_64")]
unsafe fn x87_restore(saved_cw: u16) {
    let cw = saved_cw;
    unsafe {
        std::arch::asm!(
            "fldcw [{cw}]",
            cw = in(reg) &cw,
        );
    }
}

/// Compute (a * b - c * d) at x87 80-bit precision, return as f32.
/// Matches: fld a; fmul b; fld c; fmul d; fsubp; fstp result
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn x87_cross_f32(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{a}]",
            "fmul dword ptr [{b}]",
            "fld dword ptr [{c}]",
            "fmul dword ptr [{d}]",
            "fsubp",
            "fstp dword ptr [{out}]",
            a = in(reg) &a, b = in(reg) &b,
            c = in(reg) &c, d = in(reg) &d,
            out = in(reg) &mut result,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    result
}

/// Compute a * b + c at x87 80-bit precision, return as f32.
/// Matches: fld a; fmul b; fadd c; fstp result
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn x87_fma_f32(a: f32, b: f32, c: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{a}]",
            "fmul dword ptr [{b}]",
            "fadd dword ptr [{c}]",
            "fstp dword ptr [{out}]",
            a = in(reg) &a, b = in(reg) &b,
            c = in(reg) &c,
            out = in(reg) &mut result,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    result
}

/// Compute 1.0 / a at x87 80-bit precision, return as f32.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn x87_rcp_f32(a: f32) -> f32 {
    let mut result: f32 = 0.0;
    let one: f32 = 1.0;
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{one}]",
            "fdiv dword ptr [{a}]",
            "fstp dword ptr [{out}]",
            one = in(reg) &one, a = in(reg) &a,
            out = in(reg) &mut result,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    result
}

/// Compute a + b + c * d at x87 80-bit precision, return as f32.
/// Matches: fld a; fadd b; fld c; fmul d; faddp; fstp result
/// Critical: a+b stays at 80-bit without f32 truncation before adding c*d.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn x87_add_fma_f32(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{a}]",
            "fadd dword ptr [{b}]",
            "fld dword ptr [{c}]",
            "fmul dword ptr [{d}]",
            "faddp",
            "fstp dword ptr [{out}]",
            a = in(reg) &a, b = in(reg) &b,
            c = in(reg) &c, d = in(reg) &d,
            out = in(reg) &mut result,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    result
}

/// Compute a - b at x87 80-bit precision, return as f32.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn x87_sub_f32(a: f32, b: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{a}]",
            "fsub dword ptr [{b}]",
            "fstp dword ptr [{out}]",
            a = in(reg) &a, b = in(reg) &b,
            out = in(reg) &mut result,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    result
}

/// Compute (a * b - c * d) * e at x87 80-bit precision, return as f32.
/// Matches: fld a; fmul b; fld c; fmul d; fsubp; fmul e; fstp result
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn x87_cross_mul_f32(a: f32, b: f32, c: f32, d: f32, e: f32) -> f32 {
    let mut result: f32 = 0.0;
    unsafe {
        std::arch::asm!(
            "fld dword ptr [{a}]",
            "fmul dword ptr [{b}]",
            "fld dword ptr [{c}]",
            "fmul dword ptr [{d}]",
            "fsubp",
            "fmul dword ptr [{e}]",
            "fstp dword ptr [{out}]",
            a = in(reg) &a, b = in(reg) &b,
            c = in(reg) &c, d = in(reg) &d,
            e = in(reg) &e,
            out = in(reg) &mut result,
            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
        );
    }
    result
}

fn cluster_fit_4color(pixels: &[[u8; 4]; 16], dedup: bool) -> ([u8; 8], f32) {
    // Set x87 FPU to 64-bit extended precision (matching original 32-bit code)
    #[cfg(target_arch = "x86_64")]
    let saved_cw = unsafe { x87_set_extended_precision() };

    let (colors, weights, order, n) = build_color_set(pixels, dedup);

    // Pre-multiply colors by weights (matching the original which stores
    // color*weight at object offset 0x20 in FUN_0067f490)
    let mut wt_colors = vec![[0.0f32; 3]; n];
    for i in 0..n {
        for k in 0..3 {
            wt_colors[i][k] = colors[i][k] * weights[i];
        }
    }

    // Precompute totals (matching offsets 0x13c-0x148 in the original)
    let mut total_rgb = [0.0f32; 3];
    let mut total_w: f32 = 0.0;
    for i in 0..n {
        for k in 0..3 { total_rgb[k] += wt_colors[i][k]; }
        total_w += weights[i];
    }

    let mut best_err = f32::MAX;
    let mut best_ep0 = [0.0f32; 3];
    let mut best_ep1 = [0.0f32; 3];
    let mut best_s = 0usize;
    let mut best_t = 0usize;
    let mut best_u = 0usize;

    // Incremental accumulation matching the original's triple-nested loop
    // structure (FUN_0067e280). Each loop accumulates partial sums one color
    // at a time, avoiding cumulative-sum-with-difference which rounds differently.

    // f64 intermediates emulate x87: compute at extended precision, store to f32.
    macro_rules! f32via64 {
        ($expr:expr) => { ($expr) as f32 }
    }

    // Outer loop: boundary s, group A = sorted[0..s]
    let mut outer_rgb = [0.0f32; 3];
    let mut outer_w: f32 = 0.0;

    for s in 0..=n {
        // Middle loop: boundary t, group B = sorted[s..t]
        let mut mid_rgb = [0.0f32; 3];
        let mut mid_w: f32 = 0.0;

        for t in s..=n {
            // Precompute values constant across inner loop
            // (matching fStack_6c, fStack_68, fStack_7c, fStack_64 in the original)
            let c_4_9: f32 = 4.0 / 9.0;
            let c_1_9: f32 = 1.0 / 9.0;
            let c_2_9: f32 = 2.0 / 9.0;
            let c_2_3: f32 = 2.0 / 3.0;
            let c_1_3: f32 = 1.0 / 3.0;
            let c_1_31: f32 = 1.0 / 31.0;
            let c_1_63: f32 = 1.0 / 63.0;
            let c_31: f32 = 31.0;
            let c_63: f32 = 63.0;
            let c_half: f32 = 0.5;
            let c_2: f32 = 2.0;

            let aa_partial = x87_fma_f32(mid_w, c_4_9, outer_w);
            let remaining_w: f32 = {
                // (total_w - outer_w) - mid_w — two subtractions
                let tmp = total_w - outer_w; // f32 sub (stored to stack in original)
                tmp - mid_w
            };
            let bb_mid_term = x87_fma_f32(mid_w, c_1_9, 0.0); // mid_w * 1/9 (no add, but fma with 0)
            let mut beta_a_mid = [0.0f32; 3];
            for k in 0..3 {
                beta_a_mid[k] = mid_rgb[k] * c_2_3; // simple f32 multiply
            }

            // Inner loop: boundary u, group C = sorted[t..u]
            let mut inner_rgb = [0.0f32; 3];
            let mut inner_w: f32 = 0.0;

            for u in t..=n {
                // Partition metrics using x87 for critical operations
                let alpha_aa = x87_fma_f32(inner_w, c_1_9, aa_partial);
                // alpha_bb = inner_w*4/9 + (remaining_w - inner_w) + bb_mid_term
                // Must be ONE x87 sequence — no intermediate f32 stores.
                let alpha_bb: f32 = {
                    let mut result: f32 = 0.0;
                    unsafe {
                        std::arch::asm!(
                            "fld dword ptr [{iw}]",      // st0 = inner_w
                            "fmul dword ptr [{c49}]",    // st0 = inner_w * 4/9
                            "fld dword ptr [{rw}]",      // st0 = remaining_w, st1 = inner_w*4/9
                            "fsub dword ptr [{iw}]",     // st0 = remaining_w - inner_w
                            "faddp",                      // st0 = inner_w*4/9 + (remaining_w - inner_w)
                            "fadd dword ptr [{bbm}]",    // st0 = + bb_mid_term
                            "fstp dword ptr [{out}]",    // store to f32
                            iw = in(reg) &inner_w,
                            c49 = in(reg) &c_4_9,
                            rw = in(reg) &remaining_w,
                            bbm = in(reg) &bb_mid_term,
                            out = in(reg) &mut result,
                            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                        );
                    }
                    result
                };
                let alpha_ab = x87_fma_f32(inner_w + mid_w, c_2_9, 0.0);

                // Determinant and endpoint solve — single x87 block matching the original.
                // FPU stack trace shows: inv_det is computed ONCE at 80-bit and stays
                // on the FPU stack across ALL 6 endpoint multiplications (3 channels × 2).
                // beta_a values computed with beta_a_mid + outer at 80-bit (via FST keep).
                let det = x87_cross_f32(alpha_aa, alpha_bb, alpha_ab, alpha_ab);

                // Original has NO det check — computes 1/det unconditionally.
                // Infinity from zero det gets clamped to [0,1], producing high
                // error that loses the partition comparison naturally.
                if true {
                    // Compute all beta values first (matching original's sequence)
                    let beta_a = [
                        x87_add_fma_f32(beta_a_mid[0], outer_rgb[0], inner_rgb[0], c_1_3),
                        x87_add_fma_f32(beta_a_mid[1], outer_rgb[1], inner_rgb[1], c_1_3),
                        x87_add_fma_f32(beta_a_mid[2], outer_rgb[2], inner_rgb[2], c_1_3),
                    ];
                    let beta_b = [
                        x87_sub_f32(total_rgb[0], beta_a[0]),
                        x87_sub_f32(total_rgb[1], beta_a[1]),
                        x87_sub_f32(total_rgb[2], beta_a[2]),
                    ];

                    // Single asm block: compute inv_det ONCE at 80-bit, then all 6 endpoints.
                    // Uses array pointers to stay within register limits.
                    let mut ep_a = [0.0f32; 3];
                    let mut ep_b = [0.0f32; 3];
                    let mut _dummy: f32 = 0.0;
                    let alphas = [alpha_aa, alpha_bb, alpha_ab];
                    unsafe {
                        std::arch::asm!(
                            // --- Compute inv_det at 80-bit (stays on stack) ---
                            "fld dword ptr [{al}]",        // aa
                            "fmul dword ptr [{al} + 4]",   // * bb
                            "fld dword ptr [{al} + 8]",    // ab
                            "fmul dword ptr [{al} + 8]",   // * ab
                            "fsubp",                        // det = aa*bb - ab*ab (80-bit)
                            "fld1",
                            "fdivrp",                       // st0 = inv_det (80-bit)

                            // --- Channel R ---
                            "fld dword ptr [{ba}]",        // beta_a[0]
                            "fmul dword ptr [{al} + 4]",   // * bb
                            "fld dword ptr [{bv}]",        // beta_b[0]
                            "fmul dword ptr [{al} + 8]",   // * ab
                            "fsubp",
                            "fmul st, st(1)",               // * inv_det (80-bit!)
                            "fstp dword ptr [{ea}]",        // ep_a[0]

                            "fld dword ptr [{bv}]",        // beta_b[0]
                            "fmul dword ptr [{al}]",       // * aa
                            "fld dword ptr [{ba}]",        // beta_a[0]
                            "fmul dword ptr [{al} + 8]",   // * ab
                            "fsubp",
                            "fmul st, st(1)",
                            "fstp dword ptr [{eb}]",        // ep_b[0]

                            // --- Channel G ---
                            "fld dword ptr [{ba} + 4]",
                            "fmul dword ptr [{al} + 4]",
                            "fld dword ptr [{bv} + 4]",
                            "fmul dword ptr [{al} + 8]",
                            "fsubp",
                            "fmul st, st(1)",
                            "fstp dword ptr [{ea} + 4]",

                            "fld dword ptr [{bv} + 4]",
                            "fmul dword ptr [{al}]",
                            "fld dword ptr [{ba} + 4]",
                            "fmul dword ptr [{al} + 8]",
                            "fsubp",
                            "fmul st, st(1)",
                            "fstp dword ptr [{eb} + 4]",

                            // --- Channel B ---
                            "fld dword ptr [{ba} + 8]",
                            "fmul dword ptr [{al} + 4]",
                            "fld dword ptr [{bv} + 8]",
                            "fmul dword ptr [{al} + 8]",
                            "fsubp",
                            "fmul st, st(1)",
                            "fstp dword ptr [{ea} + 8]",

                            "fld dword ptr [{bv} + 8]",
                            "fmul dword ptr [{al}]",
                            "fld dword ptr [{ba} + 8]",
                            "fmul dword ptr [{al} + 8]",
                            "fsubp",
                            "fmul st, st(1)",
                            "fstp dword ptr [{eb} + 8]",

                            // Pop inv_det
                            "fstp dword ptr [{dm}]",

                            al = in(reg) alphas.as_ptr(),
                            ba = in(reg) beta_a.as_ptr(),
                            bv = in(reg) beta_b.as_ptr(),
                            ea = in(reg) ep_a.as_mut_ptr(),
                            eb = in(reg) ep_b.as_mut_ptr(),
                            dm = in(reg) &mut _dummy,
                            out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                            out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                        );
                    }
                    for k in 0..3 {
                        ep_a[k] = ep_a[k].clamp(0.0, 1.0);
                        ep_b[k] = ep_b[k].clamp(0.0, 1.0);
                    }

                    // Grid-snap: quantize to 5/6/5 then dequantize
                    let grid_vals = [c_31, c_63, c_31];
                    let inv_vals = [c_1_31, c_1_63, c_1_31];
                    for k in 0..3 {
                        // Grid-snap matching original's FUN_006818c0 path:
                        // floor at f64 precision (not f32) to avoid rounding past
                        // integer boundaries.
                        let qa = (ep_a[k] as f64 * grid_vals[k] as f64 + 0.5f64).floor();
                        ep_a[k] = (qa * inv_vals[k] as f64) as f32;
                        let qb = (ep_b[k] as f64 * grid_vals[k] as f64 + 0.5f64).floor();
                        ep_b[k] = (qb * inv_vals[k] as f64) as f32;
                    }

                    // Error computation — ENTIRE error at x87 80-bit precision.
                    // Hardware test confirmed: x87 gives different f32 results than SSE
                    // (1 ULP difference). The original uses x87 for the full error
                    // accumulation across all 3 channels. Competing partitions differ
                    // by as little as 0.006, so even 1 ULP in the error sum can flip
                    // the partition comparison.
                    //
                    // Structure (from decompilation):
                    //   For each channel k:
                    //     diag_k = ea_k² * aa + eb_k² * bb (80-bit, stored to f32)
                    //     cross_k = eb_k*ea_k*ab - beta_a_k*ea_k - eb_k*beta_b_k (80-bit, stored to f32)
                    //   total_err = (cross_R*2 + diag_R)*wR² + (cross_G*2 + diag_G)*wG² + (cross_B*2 + diag_B)*wB²
                    //   With uniform weights (1.0), this simplifies to sum of (cross*2 + diag)
                    let mut err: f32 = 0.0;
                    {
                        // Recompute beta for error (matching original which reuses the same values)
                        let ba = [
                            x87_add_fma_f32(beta_a_mid[0], outer_rgb[0], inner_rgb[0], c_1_3),
                            x87_add_fma_f32(beta_a_mid[1], outer_rgb[1], inner_rgb[1], c_1_3),
                            x87_add_fma_f32(beta_a_mid[2], outer_rgb[2], inner_rgb[2], c_1_3),
                        ];
                        let bv = [
                            x87_sub_f32(total_rgb[0], ba[0]),
                            x87_sub_f32(total_rgb[1], ba[1]),
                            x87_sub_f32(total_rgb[2], ba[2]),
                        ];

                        // Pack inputs for asm block
                        let err_in = [
                            alpha_aa, alpha_bb, alpha_ab,   // [0..3] alphas
                            ep_a[0], ep_a[1], ep_a[2],      // [3..6] ep_a RGB
                            ep_b[0], ep_b[1], ep_b[2],      // [6..9] ep_b RGB
                            ba[0], ba[1], ba[2],            // [9..12] beta_a RGB
                            bv[0], bv[1], bv[2],            // [12..15] beta_b RGB
                            c_2,                             // [15] constant 2.0
                        ];
                        // Compute error at x87 80-bit, accumulate across channels at 80-bit
                        unsafe {
                            std::arch::asm!(
                                // Initialize error sum to 0 on FPU stack
                                "fldz",                             // st0 = 0 (running sum)

                                // === Channel R (offsets: aa=0, bb=4, ab=8, ea=12, eb=24, ba=36, bv=48, c2=60) ===
                                // diag_R = ea_R² * aa + eb_R² * bb
                                "fld dword ptr [{p} + 12]",         // ea_R
                                "fmul dword ptr [{p} + 12]",        // ea_R²
                                "fmul dword ptr [{p}]",             // ea_R² * aa
                                "fld dword ptr [{p} + 24]",         // eb_R
                                "fmul dword ptr [{p} + 24]",        // eb_R²
                                "fmul dword ptr [{p} + 4]",         // eb_R² * bb
                                "faddp",                             // diag_R (80-bit)
                                // cross_R = eb_R*ea_R*ab - ba_R*ea_R - eb_R*bv_R
                                "fld dword ptr [{p} + 24]",         // eb_R
                                "fmul dword ptr [{p} + 12]",        // eb_R * ea_R
                                "fmul dword ptr [{p} + 8]",         // * ab
                                "fld dword ptr [{p} + 36]",         // ba_R
                                "fmul dword ptr [{p} + 12]",        // ba_R * ea_R
                                "fsubp",                             // eb*ea*ab - ba*ea
                                "fld dword ptr [{p} + 24]",         // eb_R
                                "fmul dword ptr [{p} + 48]",        // eb_R * bv_R
                                "fsubp",                             // cross_R (80-bit)
                                // ch_err = cross*2 + diag, add to running sum
                                "fmul dword ptr [{p} + 60]",        // cross * 2.0
                                "faddp",                             // cross*2 + diag (80-bit)
                                "faddp",                             // running_sum += ch_err

                                // === Channel G (ea=16, eb=28, ba=40, bv=52) ===
                                "fld dword ptr [{p} + 16]",
                                "fmul dword ptr [{p} + 16]",
                                "fmul dword ptr [{p}]",
                                "fld dword ptr [{p} + 28]",
                                "fmul dword ptr [{p} + 28]",
                                "fmul dword ptr [{p} + 4]",
                                "faddp",
                                "fld dword ptr [{p} + 28]",
                                "fmul dword ptr [{p} + 16]",
                                "fmul dword ptr [{p} + 8]",
                                "fld dword ptr [{p} + 40]",
                                "fmul dword ptr [{p} + 16]",
                                "fsubp",
                                "fld dword ptr [{p} + 28]",
                                "fmul dword ptr [{p} + 52]",
                                "fsubp",
                                "fmul dword ptr [{p} + 60]",
                                "faddp",
                                "faddp",

                                // === Channel B (ea=20, eb=32, ba=44, bv=56) ===
                                "fld dword ptr [{p} + 20]",
                                "fmul dword ptr [{p} + 20]",
                                "fmul dword ptr [{p}]",
                                "fld dword ptr [{p} + 32]",
                                "fmul dword ptr [{p} + 32]",
                                "fmul dword ptr [{p} + 4]",
                                "faddp",
                                "fld dword ptr [{p} + 32]",
                                "fmul dword ptr [{p} + 20]",
                                "fmul dword ptr [{p} + 8]",
                                "fld dword ptr [{p} + 44]",
                                "fmul dword ptr [{p} + 20]",
                                "fsubp",
                                "fld dword ptr [{p} + 32]",
                                "fmul dword ptr [{p} + 56]",
                                "fsubp",
                                "fmul dword ptr [{p} + 60]",
                                "faddp",
                                "faddp",

                                // Store final error sum to f32
                                "fstp dword ptr [{out}]",

                                p = in(reg) err_in.as_ptr(),
                                out = in(reg) &mut err,
                                out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                                out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                            );
                        }
                    }

                    if err < best_err {
                        best_err = err;
                        best_ep0 = ep_a;
                        best_ep1 = ep_b;
                        best_s = s;
                        best_t = t;
                        best_u = u;
                    }
                }

                // Advance inner accumulation (add color[u] to inner sums)
                if u < n {
                    for k in 0..3 { inner_rgb[k] += wt_colors[u][k]; }
                    inner_w += weights[u];
                }
            }

            // Advance middle accumulation
            if t < n {
                for k in 0..3 { mid_rgb[k] += wt_colors[t][k]; }
                mid_w += weights[t];
            }
        }

        // Advance outer accumulation
        if s < n {
            for k in 0..3 { outer_rgb[k] += wt_colors[s][k]; }
            outer_w += weights[s];
        }
    }

    // Quantize to RGB565 (endpoints are already grid-snapped,
    // but we re-quantize here for the integer values)
    let r0 = (best_ep0[0] * 31.0 + 0.5).floor().clamp(0.0, 31.0) as u8;
    let g0 = (best_ep0[1] * 63.0 + 0.5).floor().clamp(0.0, 63.0) as u8;
    let b0 = (best_ep0[2] * 31.0 + 0.5).floor().clamp(0.0, 31.0) as u8;
    let r1 = (best_ep1[0] * 31.0 + 0.5).floor().clamp(0.0, 31.0) as u8;
    let g1 = (best_ep1[1] * 63.0 + 0.5).floor().clamp(0.0, 63.0) as u8;
    let b1 = (best_ep1[2] * 31.0 + 0.5).floor().clamp(0.0, 31.0) as u8;

    let mut c0 = ((r0 as u16) << 11) | ((g0 as u16) << 5) | (b0 as u16);
    let mut c1 = ((r1 as u16) << 11) | ((g1 as u16) << 5) | (b1 as u16);

    // Assign indices from partition boundaries (matching the original encoder).
    // The partition search found (best_s, best_t, best_u) which divide the n
    // sorted unique colors into 4 groups:
    //   sorted[0..s]   → index 0 (endpoint c0)
    //   sorted[s..t]   → index 2 (2/3 c0 + 1/3 c1)
    //   sorted[t..u]   → index 3 (1/3 c0 + 2/3 c1)
    //   sorted[u..n]   → index 1 (endpoint c1)
    //
    // build_color_set returns `order`: the mapping from unique color index to
    // sorted position. Each pixel was assigned a unique color index during dedup
    // (stored implicitly by position in the colors array). We need to map each
    // pixel back through the dedup + sort to find its partition group.

    // Step 1: Build sorted_position → DXT1 index mapping for unique colors
    let mut unique_to_index = vec![0u32; n];
    for i in 0..n {
        let idx = if i < best_s { 0u32 }
            else if i < best_t { 2 }
            else if i < best_u { 3 }
            else { 1 };
        unique_to_index[i] = idx;
    }

    // Map each pixel through sort to find its partition group index.
    let mut inv_order = vec![0usize; n];
    for (sorted_pos, &orig_idx) in order.iter().enumerate() {
        inv_order[orig_idx] = sorted_pos;
    }

    let mut indices = 0u32;
    if dedup {
        // With dedup: need pixel → unique color → sorted position mapping
        let mut color_bytes_list: Vec<[u8; 3]> = Vec::with_capacity(16);
        let mut pixel_to_unique = [0usize; 16];
        for (i, p) in pixels.iter().enumerate() {
            let rgb = [p[0], p[1], p[2]];
            let mut found = None;
            for (j, existing) in color_bytes_list.iter().enumerate() {
                if *existing == rgb { found = Some(j); break; }
            }
            let unique_idx = match found {
                Some(j) => j,
                None => { color_bytes_list.push(rgb); color_bytes_list.len() - 1 }
            };
            pixel_to_unique[i] = unique_idx;
        }
        for i in 0..16 {
            let sorted_pos = inv_order[pixel_to_unique[i]];
            indices |= unique_to_index[sorted_pos] << (i * 2);
        }
    } else {
        // Without dedup: pixel i = entry i directly
        for i in 0..16 {
            let sorted_pos = inv_order[i];
            indices |= unique_to_index[sorted_pos] << (i * 2);
        }
    }

    // Now enforce 4-color mode: c0 > c1. If we need to swap, XOR bit 0 of
    // each 2-bit index (the game's remapping: 0<->1, 2<->3).
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
        let mut remapped = 0u32;
        for i in 0..16 {
            let idx = (indices >> (i * 2)) & 3;
            remapped |= (idx ^ 1) << (i * 2);
        }
        indices = remapped;
    }

    if c0 == c1 {
        let mut out = [0u8; 8];
        out[0..2].copy_from_slice(&c0.to_le_bytes());
        out[2..4].copy_from_slice(&c1.to_le_bytes());
        let err = compute_4color_error(pixels, c0, c1);
        #[cfg(target_arch = "x86_64")]
        unsafe { x87_restore(saved_cw); }
        return (out, err);
    }

    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    let err = compute_4color_error(pixels, c0, c1);
    #[cfg(target_arch = "x86_64")]
    unsafe { x87_restore(saved_cw); }
    (out, err)
}

/// 3-color ClusterFit encoder.
/// Returns (encoded_block, weighted_error).
#[allow(dead_code)]
fn cluster_fit_3color(pixels: &[[u8; 4]; 16]) -> ([u8; 8], f32) {
    let (sorted_colors, sorted_weights, _order, n) = build_color_set(pixels, true);

    // Precompute cumulative sums for partition search
    let mut cum_w = vec![0.0f32; n + 1];
    let mut cum_rgb = vec![[0.0f32; 3]; n + 1];
    for i in 0..n {
        cum_w[i + 1] = cum_w[i] + sorted_weights[i];
        for k in 0..3 {
            cum_rgb[i + 1][k] = cum_rgb[i][k] + sorted_colors[i][k] * sorted_weights[i];
        }
    }
    let total_rgb = cum_rgb[n];

    // Exhaustive 3-partition search (2 boundaries)
    let mut best_err = f32::MAX;
    let mut best_ep0 = [0.0f32; 3];
    let mut best_ep1 = [0.0f32; 3];

    for s in 0..=n {
        let w_a = cum_w[s];
        for t in s..=n {
            let w_b = cum_w[t] - cum_w[s];
            let w_c = cum_w[n] - cum_w[t];

            let alpha_aa = w_a + w_b * 0.25f32;
            let alpha_bb = w_c + w_b * 0.25f32;
            let alpha_ab = w_b * 0.25f32;
            let det = alpha_aa * alpha_bb - alpha_ab * alpha_ab;
            if det.abs() < 1e-6 { continue; }
            let inv_det: f32 = 1.0 / det;

            let sum_a = cum_rgb[s];
            let sum_b = [cum_rgb[t][0] - cum_rgb[s][0], cum_rgb[t][1] - cum_rgb[s][1], cum_rgb[t][2] - cum_rgb[s][2]];
            let sum_c = [total_rgb[0] - cum_rgb[t][0], total_rgb[1] - cum_rgb[t][1], total_rgb[2] - cum_rgb[t][2]];

            let mut err = 0.0f32;
            let mut ep_a = [0.0f32; 3];
            let mut ep_b = [0.0f32; 3];
            for k in 0..3 {
                let beta_a = sum_a[k] + sum_b[k] * 0.5;
                let beta_b = sum_c[k] + sum_b[k] * 0.5;
                let a_k = (beta_a * alpha_bb - beta_b * alpha_ab) * inv_det;
                let b_k = (beta_b * alpha_aa - beta_a * alpha_ab) * inv_det;
                ep_a[k] = a_k.clamp(0.0, 1.0);
                ep_b[k] = b_k.clamp(0.0, 1.0);
                err += alpha_aa * ep_a[k] * ep_a[k]
                    + alpha_bb * ep_b[k] * ep_b[k]
                    + 2.0 * alpha_ab * ep_a[k] * ep_b[k]
                    - 2.0 * (beta_a * ep_a[k] + beta_b * ep_b[k]);
            }
            if err < best_err {
                best_err = err;
                best_ep0 = ep_a;
                best_ep1 = ep_b;
            }
        }
    }

    // Quantize to RGB565
    let r0 = (best_ep0[0] * 31.0 + 0.5).floor().clamp(0.0, 31.0) as u8;
    let g0 = (best_ep0[1] * 63.0 + 0.5).floor().clamp(0.0, 63.0) as u8;
    let b0 = (best_ep0[2] * 31.0 + 0.5).floor().clamp(0.0, 31.0) as u8;
    let r1 = (best_ep1[0] * 31.0 + 0.5).floor().clamp(0.0, 31.0) as u8;
    let g1 = (best_ep1[1] * 63.0 + 0.5).floor().clamp(0.0, 63.0) as u8;
    let b1 = (best_ep1[2] * 31.0 + 0.5).floor().clamp(0.0, 31.0) as u8;

    let mut c0 = ((r0 as u16) << 11) | ((g0 as u16) << 5) | (b0 as u16);
    let mut c1 = ((r1 as u16) << 11) | ((g1 as u16) << 5) | (b1 as u16);

    // 3-color mode requires c0 <= c1; swap endpoints if needed
    if c0 > c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    if c0 == c1 {
        if c1 < 0xFFFF { c1 += 1; }
        else { c0 -= 1; }
    }

    // Build 3-color palette from quantized endpoints
    let (dr0, dg0, db0) = decode_rgb565(c0);
    let (dr1, dg1, db1) = decode_rgb565(c1);
    let palette: [(i16, i16, i16); 3] = [
        (dr0 as i16, dg0 as i16, db0 as i16),
        (dr1 as i16, dg1 as i16, db1 as i16),
        (
            (dr0 as i16 + dr1 as i16) / 2,
            (dg0 as i16 + dg1 as i16) / 2,
            (db0 as i16 + db1 as i16) / 2,
        ),
    ];

    // Assign indices: 0=c0, 1=c1, 2=midpoint
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
            if dist < best_dist { best_dist = dist; best_sel = sel as u32; }
        }
        indices |= best_sel << (i * 2);
    }

    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    let err = compute_3color_error(pixels, c0, c1);
    (out, err)
}

/// Quantize a float RGB channel value [0,255] to 5-bit or 6-bit.
/// Matches FUN_0067c0b0: val * scale, clamp to [0, max], then trunc(clamped + 0.5).
/// The original sets x87 to truncation mode (OR 0x0C00) before fistp.
/// trunc(x + 0.5) for positive x = floor(x + 0.5) = standard round-half-up.
fn quantize_channel_rf(val: f32, scale: f32, max_val: f32) -> u32 {
    let scaled = val * scale;
    let clamped = if scaled < 0.0 { 0.0 } else if scaled > max_val { max_val } else { scaled };
    (clamped + 0.5) as u32 // trunc for positive = floor = round-half-up
}

/// Assign DXT1 4-color indices by nearest distance (FUN_0067c260).
/// Endpoints are dequantized float RGB [0,255].
fn assign_indices_distance(fpixels: &[[f32; 3]; 16], ep0: &[f32; 3], ep1: &[f32; 3]) -> u32 {
    // Interpolated palette colors in float space
    let color2 = [
        ep0[0] * 0.6666666 + ep1[0] * 0.33333334,
        ep0[1] * 0.6666666 + ep1[1] * 0.33333334,
        ep0[2] * 0.6666666 + ep1[2] * 0.33333334,
    ];
    let color3 = [
        ep0[0] * 0.3333333 + ep1[0] * 0.6666667,
        ep0[1] * 0.3333333 + ep1[1] * 0.6666667,
        ep0[2] * 0.3333333 + ep1[2] * 0.6666667,
    ];

    let mut indices = 0u32;
    for i in 0..16 {
        let p = &fpixels[i];
        let d0 = (ep0[0]-p[0])*(ep0[0]-p[0]) + (ep0[1]-p[1])*(ep0[1]-p[1]) + (ep0[2]-p[2])*(ep0[2]-p[2]);
        let d1 = (ep1[0]-p[0])*(ep1[0]-p[0]) + (ep1[1]-p[1])*(ep1[1]-p[1]) + (ep1[2]-p[2])*(ep1[2]-p[2]);
        let d2 = (color2[0]-p[0])*(color2[0]-p[0]) + (color2[1]-p[1])*(color2[1]-p[1]) + (color2[2]-p[2])*(color2[2]-p[2]);
        let d3 = (color3[0]-p[0])*(color3[0]-p[0]) + (color3[1]-p[1])*(color3[1]-p[1]) + (color3[2]-p[2])*(color3[2]-p[2]);

        // Bit-logic matching FUN_0067c260
        let bit1 = (d2 < d0 && d2 < d1) || (d3 < d1 && d3 < d0);
        let bit0 = d3 < d2 && d3 < d0;
        let idx = (bit1 as u32) * 2 | (bit0 as u32);
        indices |= idx << (i * 2);
    }
    indices
}

/// Quantize an endpoint [R,G,B] to RGB565, return (rgb565, dequantized [R,G,B]).
/// Matches FUN_0067c0b0.
fn quantize_endpoint_rf(ep: &[f32; 3]) -> (u16, [f32; 3]) {
    let r5 = quantize_channel_rf(ep[0], 0.12156863, 31.0); // 31/255
    let g6 = quantize_channel_rf(ep[1], 0.24705882, 63.0); // 63/255
    let b5 = quantize_channel_rf(ep[2], 0.12156863, 31.0);
    let rgb565 = ((r5 << 11) | (g6 << 5) | b5) as u16;
    let deq = [
        ((r5 << 3) | (r5 >> 2)) as f32,
        ((g6 << 2) | (g6 >> 4)) as f32,
        ((b5 << 3) | (b5 >> 2)) as f32,
    ];
    (rgb565, deq)
}

/// Encode 16 RGBA pixels → DXT1 block using range-fit with inertia extension
/// and least-squares refinement.
///
/// Matches the original's FUN_0067CF20 pipeline exactly:
///   FUN_0067b5e0 (pixel extract) → FUN_0067bd60 (bounding box) →
///   FUN_0067be80 (inertia extension) → FUN_0067bfc0 (inset) →
///   FUN_0067c0b0 (quantize) → FUN_0067c260 (indices) →
///   FUN_0067c860 (least-squares refinement)
fn encode_dxt1_range_fit(pixels: &[[u8; 4]; 16]) -> [u8; 8] {
    // Step 0: Monochrome check (FUN_0067b5a0)
    let first_rgb = (pixels[0][0], pixels[0][1], pixels[0][2]);
    if pixels.iter().all(|p| (p[0], p[1], p[2]) == first_rgb) {
        return encode_dxt1_solid(first_rgb.0, first_rgb.1, first_rgb.2);
    }

    // Step 1: Convert to float RGB [0,255] (FUN_0067b5e0)
    // Original extracts from BGRA uint32 as: >>16=R, >>8=G, &0xff=B
    // Our pixels are RGBA, channels 0=R, 1=G, 2=B — same order.
    let mut fp = [[0.0f32; 3]; 16];
    for (i, p) in pixels.iter().enumerate() {
        fp[i] = [p[0] as f32, p[1] as f32, p[2] as f32];
    }

    // Step 2: Find bounding box (FUN_0067bd60)
    let mut maxc = [0.0f32; 3];   // "max" endpoint
    let mut minc = [255.0f32; 3]; // "min" endpoint
    for p in &fp {
        for c in 0..3 {
            if p[c] > maxc[c] { maxc[c] = p[c]; }
            if p[c] < minc[c] { minc[c] = p[c]; }
        }
    }

    // Step 3: Inertia extension (FUN_0067be80)
    // Compute midpoint of bounding box, then cross-correlations
    let mid = [
        (minc[0] + maxc[0]) * 0.5,
        (minc[1] + maxc[1]) * 0.5,
        (minc[2] + maxc[2]) * 0.5,
    ];
    let mut cross_rb = 0.0f32; // sum((R - midR) * (B - midB))
    let mut cross_bg = 0.0f32; // sum((B - midB) * (G - midG))
    for p in &fp {
        let dr = p[0] - mid[0];
        let dg = p[1] - mid[1];
        let db = p[2] - mid[2];
        cross_rb += dr * db;
        cross_bg += db * dg;
    }
    // Swap min/max R if cross_rb < 0, swap min/max G if cross_bg < 0
    if cross_rb < 0.0 { std::mem::swap(&mut minc[0], &mut maxc[0]); }
    if cross_bg < 0.0 { std::mem::swap(&mut minc[1], &mut maxc[1]); }

    // Step 4: Inset endpoints (FUN_0067bfc0)
    // step = (max - min) / 16 - 0.5/255
    let bias: f32 = 0.0019607844; // ≈ 0.5/255, exact f32 value from binary
    for c in 0..3 {
        let step = (maxc[c] - minc[c]) * 0.0625 - bias;
        maxc[c] -= step;
        minc[c] += step;
        // Clamp to [0, 255] (FUN_0067bcb0)
        maxc[c] = maxc[c].max(0.0).min(255.0);
        minc[c] = minc[c].max(0.0).min(255.0);
    }

    // Step 5: Quantize to RGB565 and dequantize (FUN_0067c0b0)
    let (mut c0, mut ep0) = quantize_endpoint_rf(&maxc);
    let (mut c1, mut ep1) = quantize_endpoint_rf(&minc);

    // Ensure c0 > c1 for 4-color mode
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
        std::mem::swap(&mut ep0, &mut ep1);
    }

    // Step 6: Assign indices by nearest distance (FUN_0067c260)
    let indices = assign_indices_distance(&fp, &ep0, &ep1);

    // Step 7: Least-squares refinement (FUN_0067c860)
    // Build 2x2 normal equation from index weights
    let (c0_final, c1_final, indices_final) = refine_endpoints_lsq(&fp, c0, c1, ep0, ep1, indices);

    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0_final.to_le_bytes());
    out[2..4].copy_from_slice(&c1_final.to_le_bytes());
    out[4..8].copy_from_slice(&indices_final.to_le_bytes());
    out
}

/// Least-squares endpoint refinement (FUN_0067c860).
/// Uses f64 accumulators to match x87 53-bit precision (CW=0x027F).
/// The original accumulates 16 weight×pixel products at 53-bit intermediate
/// precision, which differs from f32 (23-bit) for the matrix solve.
fn refine_endpoints_lsq(
    fp: &[[f32; 3]; 16],
    c0_in: u16, c1_in: u16,
    _ep0_in: [f32; 3], _ep1_in: [f32; 3],
    indices_in: u32,
) -> (u16, u16, u32) {
    // Weight constant from binary: 1/3 = 0x3EAAAAAB as f32
    let one_third: f64 = f32::from_bits(0x3EAAAAAB) as f64;

    // Accumulate at f64 (53-bit mantissa = x87 CW=0x027F)
    let mut aa = 0.0f64;
    let mut bb = 0.0f64;
    let mut ab = 0.0f64;
    let mut a_pixel = [0.0f64; 3];
    let mut b_pixel = [0.0f64; 3];

    for i in 0..16 {
        let idx = (indices_in >> (i * 2)) & 3;
        // Index→weight matching FUN_0067c860:
        // bit0 = idx & 1, if bit1: beta = (bit0 + 1) * one_third
        let bit0 = (idx & 1) as f64;
        let beta = if (idx & 2) != 0 { (bit0 + 1.0) * one_third } else { bit0 };
        let alpha = 1.0 - beta;

        aa += alpha * alpha;
        bb += beta * beta;
        ab += alpha * beta;

        for c in 0..3 {
            let pv = fp[i][c] as f64;
            a_pixel[c] += alpha * pv;
            b_pixel[c] += beta * pv;
        }
    }

    let det = aa * bb - ab * ab;
    if det.abs() < 0.0001 {
        return (c0_in, c1_in, indices_in);
    }

    let inv_det = 1.0f64 / det;

    let mut new_ep0 = [0.0f32; 3];
    let mut new_ep1 = [0.0f32; 3];
    for c in 0..3 {
        new_ep0[c] = ((bb * a_pixel[c] - ab * b_pixel[c]) * inv_det).max(0.0).min(255.0) as f32;
        new_ep1[c] = ((aa * b_pixel[c] - ab * a_pixel[c]) * inv_det).max(0.0).min(255.0) as f32;
    }

    let (mut c0, mut ep0) = quantize_endpoint_rf(&new_ep0);
    let (mut c1, mut ep1) = quantize_endpoint_rf(&new_ep1);

    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
        std::mem::swap(&mut ep0, &mut ep1);
    }

    let indices = assign_indices_distance(fp, &ep0, &ep1);

    (c0, c1, indices)
}

/// Encode 16 RGBA pixels → DXT1 block (8 bytes).
///
/// Uses WeightedClusterFit matching the game's encoder. When `force_3color`
/// is false, tries both 3-color and 4-color modes for opaque blocks and picks
/// the lower error; when true, always uses 3-color mode (required for DXT1a).
fn encode_dxt1_block(pixels: &[[u8; 4]; 16], _force_3color: bool, for_mip: bool) -> [u8; 8] {
    // Check for transparent pixels. DXT1 3-color mode (c0 <= c1) uses index 3
    // for transparent black. After box filtering, transparent+opaque pixels blend
    // to intermediate alpha — threshold at 128 to preserve transparency at edges.
    let has_transparent = pixels.iter().any(|p| p[3] < 128);

    if has_transparent {
        let opaque_count = pixels.iter().filter(|p| p[3] > 0).count();

        if opaque_count == 0 {
            // All transparent — 3-color mode, all index 3.
            // c0=0 < c1=1 guarantees 3-color mode.
            let mut out = [0u8; 8];
            out[2] = 1; // c1 = 0x0001
            out[4..8].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
            return out;
        }

        // Mixed: some opaque, some transparent.
        // Bounding box of opaque pixels for endpoints.
        let (mut min_r, mut min_g, mut min_b) = (255u8, 255u8, 255u8);
        let (mut max_r, mut max_g, mut max_b) = (0u8, 0u8, 0u8);
        for p in pixels.iter().filter(|p| p[3] > 0) {
            min_r = min_r.min(p[0]); min_g = min_g.min(p[1]); min_b = min_b.min(p[2]);
            max_r = max_r.max(p[0]); max_g = max_g.max(p[1]); max_b = max_b.max(p[2]);
        }

        let mut c0 = encode_rgb565(min_r, min_g, min_b);
        let mut c1 = encode_rgb565(max_r, max_g, max_b);

        // Ensure c0 <= c1 for 3-color mode
        if c0 > c1 { std::mem::swap(&mut c0, &mut c1); }
        if c0 == c1 && c1 < 0xFFFF { c1 += 1; }

        // Build 3-color palette
        let (r0, g0, b0) = decode_rgb565(c0);
        let (r1, g1, b1) = decode_rgb565(c1);
        let palette: [(i16, i16, i16); 3] = [
            (r0 as i16, g0 as i16, b0 as i16),
            (r1 as i16, g1 as i16, b1 as i16),
            (
                ((r0 as i16 + r1 as i16) / 2),
                ((g0 as i16 + g1 as i16) / 2),
                ((b0 as i16 + b1 as i16) / 2),
            ),
        ];

        // Assign indices: transparent → 3, opaque → nearest palette entry
        let mut indices = 0u32;
        for (i, p) in pixels.iter().enumerate() {
            if p[3] < 128 {
                indices |= 3 << (i * 2);
            } else {
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
                    if dist < best_dist { best_dist = dist; best_sel = sel as u32; }
                }
                indices |= best_sel << (i * 2);
            }
        }

        let mut out = [0u8; 8];
        out[0..2].copy_from_slice(&c0.to_le_bytes());
        out[2..4].copy_from_slice(&c1.to_le_bytes());
        out[4..8].copy_from_slice(&indices.to_le_bytes());
        return out;
    }

    // ── All opaque: existing path ──

    // Range-fit encoder (FUN_0067cf20) = param_4[1]==0 = Quality_Fastest.
    // Tested alternatives: ClusterFit dedup=false (10.72%), ClusterFit dedup=true (11.22%).
    // Range-fit gives best results (13.10%).
    if for_mip {
        return encode_dxt1_range_fit(pixels);
    }

    // Non-mip path: ClusterFit (for initial DXT1 encoding)
    let first_rgb = (pixels[0][0], pixels[0][1], pixels[0][2]);
    let all_solid = pixels.iter().all(|p| (p[0], p[1], p[2]) == first_rgb);
    if all_solid {
        return encode_dxt1_solid(first_rgb.0, first_rgb.1, first_rgb.2);
    }

    let (block_4, _) = cluster_fit_4color(pixels, true);
    block_4
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
            ((6 * a0 + 1 * a1) / 7) as u8,
            ((5 * a0 + 2 * a1) / 7) as u8,
            ((4 * a0 + 3 * a1) / 7) as u8,
            ((3 * a0 + 4 * a1) / 7) as u8,
            ((2 * a0 + 5 * a1) / 7) as u8,
            ((1 * a0 + 6 * a1) / 7) as u8,
        ]
    } else {
        [
            a0 as u8,
            a1 as u8,
            ((4 * a0 + 1 * a1) / 5) as u8,
            ((3 * a0 + 2 * a1) / 5) as u8,
            ((2 * a0 + 3 * a1) / 5) as u8,
            ((1 * a0 + 4 * a1) / 5) as u8,
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
    // Extract alpha values and encode as BC4 block (same structure).
    // Uses encode_bc4_block for exhaustive endpoint search + squared error
    // index assignment, matching the game's encoder.
    let mut alpha_values = [0u8; 16];
    for (i, p) in pixels.iter().enumerate() {
        alpha_values[i] = p[3];
    }
    let alpha_block = encode_bc4_block(&alpha_values);

    // Color portion (DXT1) — DXT5 never uses binary alpha
    let color_block = encode_dxt1_block(pixels, false, false);

    // DXT5 = alpha(8) + color(8)
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&alpha_block);
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

    // Use truncating division (no +3 rounding bias) to match the game engine's
    // BC4 interpolation. Verified: this produces byte-identical output for ~53%
    // of generated mip blocks vs the game's output, up from ~9% with standard rounding.
    let table: [u8; 8] = if a0 > a1 {
        [
            a0 as u8,
            a1 as u8,
            ((6 * a0 + 1 * a1) / 7) as u8,
            ((5 * a0 + 2 * a1) / 7) as u8,
            ((4 * a0 + 3 * a1) / 7) as u8,
            ((3 * a0 + 4 * a1) / 7) as u8,
            ((2 * a0 + 5 * a1) / 7) as u8,
            ((1 * a0 + 6 * a1) / 7) as u8,
        ]
    } else {
        [
            a0 as u8,
            a1 as u8,
            ((4 * a0 + 1 * a1) / 5) as u8,
            ((3 * a0 + 2 * a1) / 5) as u8,
            ((2 * a0 + 3 * a1) / 5) as u8,
            ((1 * a0 + 4 * a1) / 5) as u8,
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

/// Sum of squared errors for BC4 block with given endpoints.
/// Squared-error distance metric, per-pixel min across palette.
fn bc4_sse(values: &[u8; 16], a0: u8, a1: u8) -> u32 {
    let a0w = a0 as u16;
    let a1w = a1 as u16;
    // Build palette with truncating division (matches the game's encoder)
    let table: [u8; 8] = if a0 > a1 {
        [a0, a1,
         ((6 * a0w + 1 * a1w) / 7) as u8, ((5 * a0w + 2 * a1w) / 7) as u8,
         ((4 * a0w + 3 * a1w) / 7) as u8, ((3 * a0w + 4 * a1w) / 7) as u8,
         ((2 * a0w + 5 * a1w) / 7) as u8, ((1 * a0w + 6 * a1w) / 7) as u8]
    } else {
        [a0, a1,
         ((4 * a0w + 1 * a1w) / 5) as u8, ((3 * a0w + 2 * a1w) / 5) as u8,
         ((2 * a0w + 3 * a1w) / 5) as u8, ((1 * a0w + 4 * a1w) / 5) as u8,
         0, 255]
    };
    let mut total = 0u32;
    for &v in values {
        let mut min_err = u32::MAX;
        for &tv in &table {
            let d = v as i32 - tv as i32;
            let err = (d * d) as u32;
            if err < min_err { min_err = err; }
        }
        total += min_err;
    }
    total
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

    // Constant block: all pixels are the same value. The game engine uses
    // index 1 (a1) for all pixels. This is visually identical to any other
    // index (all palette entries decode to the same value), but must match
    // for byte-identical output.
    if a0 == a1 {
        let mut out = [0u8; 8];
        out[0] = a0;
        out[1] = a1;
        // All 16 pixels get index 1: each 3-bit group = 001
        // 16 × 3 bits = 48 bits = 0x249249249249
        let bits: u64 = 0x249249249249;
        out[2..8].copy_from_slice(&bits.to_le_bytes()[0..6]);
        return out;
    }

    // Exhaustive endpoint search (matching the game's encoder):
    // When range > 8, search (ep0, ep1) pairs for minimum sum-of-squared-errors.
    let (mut best_a0, mut best_a1) = (a0, a1);

    if (a0 as i32 - a1 as i32) > 8 {
        let range = a0 as i32 - a1 as i32;
        let mut best_error = bc4_sse(values, a0, a1) as i32;

        let mut ep0 = a1 as i32 + 9;
        let mut ep1_max = a1 as i32;
        let mut range_remaining = range - 9; // decreases as ep0 increases
        while ep0 <= a0 as i32 {
            ep1_max += 1;
            for ep1 in (a1 as i32)..ep1_max {
                // Pruning: skip pairs that can't beat current best
                if (range_remaining + ep1) <= best_error {
                    let err = bc4_sse(values, ep0 as u8, ep1 as u8) as i32;
                    if err < best_error {
                        best_error = err;
                        best_a0 = ep0 as u8;
                        best_a1 = ep1 as u8;
                    }
                }
            }
            range_remaining -= 1;
            ep0 += 1;
        }
    }

    let (a0, a1) = (best_a0, best_a1);

    // Build palette with truncating division (matches the game's encoder)
    let table: [u8; 8] = if a0 > a1 {
        let a0w = a0 as u16;
        let a1w = a1 as u16;
        [
            a0,
            a1,
            ((6 * a0w + 1 * a1w) / 7) as u8,
            ((5 * a0w + 2 * a1w) / 7) as u8,
            ((4 * a0w + 3 * a1w) / 7) as u8,
            ((3 * a0w + 4 * a1w) / 7) as u8,
            ((2 * a0w + 5 * a1w) / 7) as u8,
            ((1 * a0w + 6 * a1w) / 7) as u8,
        ]
    } else {
        let a0w = a0 as u16;
        let a1w = a1 as u16;
        [
            a0,
            a1,
            ((4 * a0w + 1 * a1w) / 5) as u8,
            ((3 * a0w + 2 * a1w) / 5) as u8,
            ((2 * a0w + 3 * a1w) / 5) as u8,
            ((1 * a0w + 4 * a1w) / 5) as u8,
            0,
            255,
        ]
    };

    // Assign indices: strict < tie-breaking, squared-error distance
    let mut bits: u64 = 0;
    for (i, &v) in values.iter().enumerate() {
        let mut best_dist = u32::MAX;
        let mut best_idx = 0u64;
        for (j, &tv) in table.iter().enumerate() {
            let d = (v as i32 - tv as i32) * (v as i32 - tv as i32);
            let d = d as u32;
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

/// Encode 16 channel values → BC4 block using iterative refinement.
/// Used for ATI2/BC5 channels (normal maps), matching the game's encoder.
fn encode_bc4_iterative(values: &[u8; 16], max_iters: i32) -> [u8; 8] {
    let mut min_v = 255u8;
    let mut max_v = 0u8;
    for &v in values { min_v = min_v.min(v); max_v = max_v.max(v); }

    if max_v == min_v {
        let mut out = [0u8; 8];
        out[0] = max_v;
        out[1] = min_v;
        let bits: u64 = 0x249249249249;
        out[2..8].copy_from_slice(&bits.to_le_bytes()[0..6]);
        return out;
    }

    // Initial endpoints: inset by range/34
    let inset = ((max_v as i32 - min_v as i32) / 34) as u8;
    let mut ep0 = max_v - inset;
    let mut ep1 = min_v + inset;
    if ep0 < ep1 { std::mem::swap(&mut ep0, &mut ep1); }
    if ep0 == ep1 { if ep0 < 255 { ep0 += 1; } else { ep1 -= 1; } }

    let mut indices = [0u8; 16];
    let mut sse = bc4_sse_with_indices(values, ep0, ep1, &mut indices);

    for _ in 0..max_iters {
        let prev_ep0 = ep0;
        let prev_ep1 = ep1;
        let prev_indices = indices;
        let prev_sse = sse;

        let (new_ep0, new_ep1) = bc4_least_squares_refine(values, &indices);
        ep0 = new_ep0;
        ep1 = new_ep1;

        sse = bc4_sse_with_indices(values, ep0, ep1, &mut indices);

        if sse >= prev_sse {
            ep0 = prev_ep0;
            ep1 = prev_ep1;
            indices = prev_indices;
            break;
        }
        if ep0 == prev_ep0 && ep1 == prev_ep1 { break; }
    }

    let mut bits: u64 = 0;
    for (i, &idx) in indices.iter().enumerate() {
        bits |= (idx as u64) << (i * 3);
    }
    let mut out = [0u8; 8];
    out[0] = ep0;
    out[1] = ep1;
    out[2..8].copy_from_slice(&bits.to_le_bytes()[0..6]);
    out
}

/// Compute SSE and assign best indices for each pixel.
/// Matches the game's SSE-with-index-assignment function.
fn bc4_sse_with_indices(values: &[u8; 16], a0: u8, a1: u8, indices: &mut [u8; 16]) -> u32 {
    let a0w = a0 as u16;
    let a1w = a1 as u16;
    let table: [u8; 8] = if a0 > a1 {
        [a0, a1,
         ((6 * a0w + 1 * a1w) / 7) as u8, ((5 * a0w + 2 * a1w) / 7) as u8,
         ((4 * a0w + 3 * a1w) / 7) as u8, ((3 * a0w + 4 * a1w) / 7) as u8,
         ((2 * a0w + 5 * a1w) / 7) as u8, ((1 * a0w + 6 * a1w) / 7) as u8]
    } else {
        [a0, a1,
         ((4 * a0w + 1 * a1w) / 5) as u8, ((3 * a0w + 2 * a1w) / 5) as u8,
         ((2 * a0w + 3 * a1w) / 5) as u8, ((1 * a0w + 4 * a1w) / 5) as u8,
         0, 255]
    };
    let mut total = 0u32;
    for (i, &v) in values.iter().enumerate() {
        let mut best_err = u32::MAX;
        let mut best_idx = 0u8;
        for (j, &tv) in table.iter().enumerate() {
            let d = v as i32 - tv as i32;
            let err = (d * d) as u32;
            if err < best_err { best_err = err; best_idx = j as u8; }
        }
        indices[i] = best_idx;
        total += best_err;
    }
    total
}

/// Least-squares refinement of BC4 endpoints.
/// Matches the game's least-squares refinement.
fn bc4_least_squares_refine(values: &[u8; 16], indices: &[u8; 16]) -> (u8, u8) {
    let mut sum_w0w0 = 0.0f32;
    let mut sum_w1w1 = 0.0f32;
    let mut sum_w0w1 = 0.0f32;
    let mut sum_vw0 = 0.0f32;
    let mut sum_vw1 = 0.0f32;

    for (&v, &idx) in values.iter().zip(indices.iter()) {
        let w0: f32 = match idx {
            0 => 1.0,
            1 => 0.0,
            i => (8.0 - i as f32) / 7.0,
        };
        let w1 = 1.0 - w0;
        let vf = v as f32;
        sum_w0w0 += w0 * w0;
        sum_w1w1 += w1 * w1;
        sum_w0w1 += w0 * w1;
        sum_vw0 += vf * w0;
        sum_vw1 += vf * w1;
    }

    let det = sum_w0w0 * sum_w1w1 - sum_w0w1 * sum_w0w1;
    if det.abs() < 1e-10 {
        let min_v = *values.iter().min().unwrap();
        let max_v = *values.iter().max().unwrap();
        return (max_v, min_v);
    }

    let inv_det = 1.0 / det;
    let new_a0 = ((sum_vw0 * sum_w1w1 - sum_vw1 * sum_w0w1) * inv_det)
        .round().clamp(0.0, 255.0) as u8;
    let new_a1 = ((sum_vw1 * sum_w0w0 - sum_vw0 * sum_w0w1) * inv_det)
        .round().clamp(0.0, 255.0) as u8;

    if new_a0 >= new_a1 { (new_a0, new_a1) } else { (new_a1, new_a0) }
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

    let r_block = encode_bc4_iterative(&red, 8);
    let g_block = encode_bc4_iterative(&green, 8);

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
        let re_encoded = encode_dxt1_block(&pixels, false, false);
        assert_eq!(re_encoded.len(), 8);
    }

    #[test]
    fn test_dxt1_gradient_block_quality() {
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            let t = (i as f32) / 15.0;
            pixels[i] = [
                (255.0 * (1.0 - t)) as u8,
                0,
                (255.0 * t) as u8,
                255,
            ];
        }
        let encoded = encode_dxt1_block(&pixels, false, false);
        let decoded = decode_dxt1_block(&encoded);
        let mut total_err = 0u64;
        for i in 0..16 {
            for c in 0..3 {
                let d = pixels[i][c] as i64 - decoded[i][c] as i64;
                total_err += (d * d) as u64;
            }
        }
        // DXT1 encodes 4 palette colors for 16 pixels; a red-to-blue gradient has
        // unavoidable quantisation error of ~8000–11000 SSE even with optimal endpoints.
        // This threshold verifies WeightedClusterFit finds near-optimal endpoints
        // (bounding-box gives ~11561; a poor encoder can reach 50000+).
        assert!(total_err < 15000, "DXT1 total squared error {} too high for gradient", total_err);
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
    fn test_dxt1a_transparency_preserved() {
        // 3-color mode block: c0=0x1082 < c1=0xef7d, all index 3 = transparent
        let block = [0x82, 0x10, 0x7d, 0xef, 0xff, 0xff, 0xff, 0xff];
        let pixels = decode_dxt1_block(&block);
        assert_eq!(pixels[0][3], 0, "Should decode as transparent");

        let re_encoded = encode_dxt1_block(&pixels, false, false);
        let re_decoded = decode_dxt1_block(&re_encoded);
        for i in 0..16 {
            assert_eq!(re_decoded[i][3], 0,
                "Pixel {} should stay transparent after roundtrip, got alpha={}", i, re_decoded[i][3]);
        }
    }

    #[test]
    fn test_dxt1a_mixed_transparency() {
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..8 { pixels[i] = [255, 0, 0, 255]; }
        for i in 8..16 { pixels[i] = [0, 0, 0, 0]; }

        let encoded = encode_dxt1_block(&pixels, false, false);
        let c0 = u16::from_le_bytes([encoded[0], encoded[1]]);
        let c1 = u16::from_le_bytes([encoded[2], encoded[3]]);
        assert!(c0 <= c1, "Should be 3-color mode, got c0={} c1={}", c0, c1);

        let decoded = decode_dxt1_block(&encoded);
        for i in 8..16 {
            assert_eq!(decoded[i][3], 0, "Pixel {} should be transparent", i);
        }
        for i in 0..8 {
            assert_eq!(decoded[i][3], 255, "Pixel {} should be opaque", i);
        }
    }

    #[test]
    fn test_dxt1_opaque_unchanged() {
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 { pixels[i] = [128, 64, 32, 255]; }
        let encoded = encode_dxt1_block(&pixels, false, false);
        let decoded = decode_dxt1_block(&encoded);
        for i in 0..16 {
            assert_eq!(decoded[i][3], 255, "Opaque pixel {} got alpha={}", i, decoded[i][3]);
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
    fn test_dxt5_alpha_truncating_division() {
        let mut block = [0u8; 16];
        block[0] = 200; // a0
        block[1] = 50;  // a1
        // All alpha indices = 2 (palette entry 2)
        let idx_bits: u64 = 0x492492492492;
        block[2..8].copy_from_slice(&idx_bits.to_le_bytes()[0..6]);
        block[8..16].copy_from_slice(&[0u8; 8]);

        let pixels = decode_dxt5_block(&block);
        // Truncating: (6*200 + 1*50) / 7 = 1250/7 = 178
        assert_eq!(pixels[0][3], 178, "DXT5 alpha should use truncating division, got {}", pixels[0][3]);
    }

    #[test]
    fn test_dxt5_alpha_uses_bc4_encoder() {
        // DXT5 alpha should produce the same output as BC4 for the same values
        let mut pixels = [[0u8; 4]; 16];
        for i in 0..16 {
            pixels[i] = [128, 64, 32, (i as u8) * 16]; // varying alpha
        }
        let dxt5 = encode_dxt5_block(&pixels);

        // Extract alpha values and encode with BC4 directly
        let mut alpha_vals = [0u8; 16];
        for i in 0..16 { alpha_vals[i] = pixels[i][3]; }
        let bc4 = encode_bc4_block(&alpha_vals);

        // DXT5 alpha portion (first 8 bytes) should match BC4 output
        assert_eq!(&dxt5[0..8], &bc4[..], "DXT5 alpha should match BC4 encoder output");
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

    #[test]
    fn test_bc4_iterative_refine_basic() {
        let values: [u8; 16] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160];
        let encoded = encode_bc4_iterative(&values, 8);
        let decoded = decode_bc4_block(&encoded);
        for i in 0..16 {
            assert!((decoded[i] as i16 - values[i] as i16).abs() <= 12,
                "Iterative BC4 pixel {}: expected ~{}, got {}", i, values[i], decoded[i]);
        }
    }

    #[test]
    fn test_bc4_iterative_constant_block() {
        let values = [128u8; 16];
        let encoded = encode_bc4_iterative(&values, 8);
        assert!((encoded[0] as i16 - encoded[1] as i16).abs() <= 1,
            "Constant block endpoints should be near-equal: {} vs {}", encoded[0], encoded[1]);
    }

    #[test]
    fn test_mip_float_rounding() {
        // Values where integer sum/4 truncates differently than float round:
        // 3 + 3 + 3 + 2 = 11. Integer: 11/4 = 2. Float: 11/4 = 2.75 → round = 3.
        let mut src = vec![0u8; 8 * 4]; // 2x2 BC4 blocks = 8x8 pixels
        // Block 0,1,2: all pixels = 3
        for i in 0..3 {
            src[i * 8] = 3; src[i * 8 + 1] = 3;
            src[i * 8 + 2..i * 8 + 8].copy_from_slice(&[0x49, 0x92, 0x24, 0x49, 0x92, 0x24]);
        }
        // Block 3: all pixels = 2
        src[24] = 2; src[25] = 2;
        src[26..32].copy_from_slice(&[0x49, 0x92, 0x24, 0x49, 0x92, 0x24]);

        let mip = generate_mip_bc4(&src, 8, 8, 4, 4);
        let decoded = decode_bc4_block(&mip);
        // Float rounding: (3+3+3+2)/4 = 2.75 → round → 3
        assert_eq!(decoded[0], 3, "Mip should use float rounding, got {}", decoded[0]);
    }

    #[test]
    fn test_cluster_fit_4color_output_snapshot() {
        // Block 1: red-to-blue gradient (exercises partition search heavily)
        let mut gradient = [[0u8; 4]; 16];
        for i in 0..16 {
            let t = i as f32 / 15.0;
            gradient[i] = [(255.0 * (1.0 - t)) as u8, 0, (255.0 * t) as u8, 255];
        }
        let (block, _err) = cluster_fit_4color(&gradient, true);
        let c0 = u16::from_le_bytes([block[0], block[1]]);
        let c1 = u16::from_le_bytes([block[2], block[3]]);
        // 4-color mode: c0 > c1
        assert!(c0 >= c1, "4-color mode requires c0 >= c1, got c0={} c1={}", c0, c1);
        // Print for manual tracking across layers
        eprintln!("gradient block: {:02x?}", block);

        // Block 2: smooth green gradient (single-channel, tests degenerate partition)
        let mut green_grad = [[0u8; 4]; 16];
        for i in 0..16 {
            green_grad[i] = [0, (i as u8) * 17, 0, 255];
        }
        let (block2, _) = cluster_fit_4color(&green_grad, true);
        let c0_2 = u16::from_le_bytes([block2[0], block2[1]]);
        let c1_2 = u16::from_le_bytes([block2[2], block2[3]]);
        assert!(c0_2 >= c1_2, "4-color mode requires c0 >= c1");
        eprintln!("green_grad block: {:02x?}", block2);

        // Block 3: mixed color noise (worst case for partition search precision)
        let noise: [[u8; 4]; 16] = [
            [200, 50, 100, 255], [180, 60, 110, 255], [160, 70, 120, 255], [140, 80, 130, 255],
            [120, 90, 140, 255], [100, 100, 150, 255], [80, 110, 160, 255], [60, 120, 170, 255],
            [50, 130, 180, 255], [70, 140, 170, 255], [90, 150, 160, 255], [110, 160, 150, 255],
            [130, 170, 140, 255], [150, 180, 130, 255], [170, 190, 120, 255], [190, 200, 110, 255],
        ];
        let (block3, _) = cluster_fit_4color(&noise, true);
        eprintln!("noise block: {:02x?}", block3);
    }

    #[test]
    #[ignore] // Run with: cargo test -- --ignored encoder_replication_comparison
    fn encoder_replication_comparison() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let our_dir = base.join("game-installs/ours-newest/The Secret World/RDB");

        // Parse both le.idx files
        let ref_idx = parse_le_index(&ref_dir.join("le.idx"))
            .expect("Failed to parse reference le.idx");
        let our_idx = parse_le_index(&our_dir.join("le.idx"))
            .expect("Failed to parse our le.idx");

        let test_ids: &[u32] = &[19767, 27137, 30186, 105336, 105339, 109115, 111092, 117998, 143403, 143762];

        let mut total_bytes = 0usize;
        let mut matching_bytes = 0usize;
        let mut total_resources = 0usize;
        let mut matching_resources = 0usize;

        for &id in test_ids {
            // Find entry in reference index (search all entries for this id)
            let ref_entry = ref_idx.entries.iter().find(|e| e.id == id);
            let our_entry = our_idx.entries.iter().find(|e| e.id == id);

            let (ref_entry, our_entry) = match (ref_entry, our_entry) {
                (Some(r), Some(o)) => (r, o),
                _ => {
                    eprintln!("  ID {}: not found in one or both indexes, skipping", id);
                    continue;
                }
            };

            eprintln!("  ID {}: type={}, ref=file{:02}@{} len={}, our=file{:02}@{} len={}",
                id, ref_entry.rdb_type,
                ref_entry.file_num, ref_entry.offset, ref_entry.length,
                our_entry.file_num, our_entry.offset, our_entry.length);

            // Read reference resource
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", ref_entry.file_num));
            let our_path = our_dir.join(format!("{:02}.rdbdata", our_entry.file_num));

            let read_resource = |path: &std::path::Path, offset: u32, length: u32| -> Option<Vec<u8>> {
                let mut file = std::fs::File::open(path).ok()?;
                file.seek(SeekFrom::Start(offset as u64)).ok()?;
                let mut buf = vec![0u8; length as usize];
                file.read_exact(&mut buf).ok()?;
                Some(buf)
            };

            let ref_data = match read_resource(&ref_path, ref_entry.offset, ref_entry.length) {
                Some(d) => d,
                None => { eprintln!("    -> failed to read reference data"); continue; }
            };
            let our_data = match read_resource(&our_path, our_entry.offset, our_entry.length) {
                Some(d) => d,
                None => { eprintln!("    -> failed to read our data"); continue; }
            };

            total_resources += 1;

            if ref_data.len() != our_data.len() {
                eprintln!("    -> SIZE MISMATCH: ref={} our={}", ref_data.len(), our_data.len());
                continue;
            }

            let len = ref_data.len();
            let matching = ref_data.iter().zip(our_data.iter()).filter(|(a, b)| a == b).count();
            let differing = len - matching;
            total_bytes += len;
            matching_bytes += matching;

            if differing == 0 {
                matching_resources += 1;
                eprintln!("    -> IDENTICAL ({} bytes)", len);
            } else {
                // Find first differing byte offset
                let first_diff = ref_data.iter().zip(our_data.iter())
                    .position(|(a, b)| a != b).unwrap();
                eprintln!("    -> {} differing bytes out of {} ({:.1}% match), first diff at offset {}",
                    differing, len, (matching as f64 / len as f64) * 100.0, first_diff);
            }
        }

        eprintln!("\n  SUMMARY:");
        eprintln!("    Resources compared: {}", total_resources);
        eprintln!("    Fully identical: {}/{}", matching_resources, total_resources);
        if total_bytes > 0 {
            eprintln!("    Total bytes: {} matching / {} total ({:.2}%)",
                matching_bytes, total_bytes,
                (matching_bytes as f64 / total_bytes as f64) * 100.0);
        }
    }

    #[test]
    #[ignore] // Run with: cargo test -- --ignored encoder_replication_mip_regen
    fn encoder_replication_mip_regen() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");

        let ref_idx = parse_le_index(&ref_dir.join("le.idx"))
            .expect("Failed to parse reference le.idx");

        // Test textures: (id, width, height)
        let test_textures: &[(u32, usize, usize)] = &[
            (19767, 512, 512),
            (27137, 256, 256),
            (30186, 512, 512),
            (105339, 256, 256),
            (117998, 512, 512),
        ];

        let fctx_header_size = 24usize;
        let block_size = 8usize; // DXT1

        let mut total_blocks = 0usize;
        let mut matching_blocks = 0usize;
        let mut first_dump = true;

        for &(id, width, height) in test_textures {
            let entry = ref_idx.entries.iter().find(|e| e.id == id)
                .unwrap_or_else(|| panic!("Texture {} not found in le.idx", id));

            // Read reference FCTX data
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let ref_data = {
                let mut file = std::fs::File::open(&ref_path)
                    .unwrap_or_else(|e| panic!("Failed to open {:?}: {}", ref_path, e));
                file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
                let mut buf = vec![0u8; entry.length as usize];
                file.read_exact(&mut buf).unwrap();
                buf
            };

            // Diagnostic: verify mip0 extraction for texture 19767
            if id == 19767 {
                eprintln!("  Header bytes: {:02x?}", &ref_data[0..8]);
                eprintln!("  Data length: {}", ref_data.len());

                // Check if header is FCTX
                let is_fctx = &ref_data[0..4] == b"FCTX";
                eprintln!("  Is FCTX: {}", is_fctx);

                // Read bytes at assumed mip0 start (offset = total - mip0_size)
                let mip0_size = ((512+3)/4) * ((512+3)/4) * 8;  // 131072
                let mip0_offset = ref_data.len() - mip0_size;
                eprintln!("  Assumed mip0 offset: {} (header would be {} bytes)", mip0_offset, mip0_offset - 0);
                eprintln!("  First mip0 DXT1 block: {:02x?}", &ref_data[mip0_offset..mip0_offset+8]);

                // Also check: are the first bytes after any plausible header a small mip (like 4x4 = 8 bytes)?
                // A 4x4 DXT1 block is 8 bytes. If stored smallest-first, offset 40 would be the 4x4 mip.
                for hdr_try in [24, 40, 48] {
                    if hdr_try + 8 <= ref_data.len() {
                        eprintln!("  After {:2} byte header: {:02x?} (smallest mip candidate)", hdr_try, &ref_data[hdr_try..hdr_try+8]);
                    }
                }

                // Verify by checking mip0 block count: 128*128 blocks, each 8 bytes
                // Last block of mip0 should be at end of file
                let last_mip0_block = &ref_data[ref_data.len()-8..];
                eprintln!("  Last 8 bytes (last mip0 block): {:02x?}", last_mip0_block);
            }

            // Compute mip sizes for this texture
            let mut mip_sizes = Vec::new();
            let mut w = width;
            let mut h = height;
            loop {
                let blocks_w = (w + 3) / 4;
                let blocks_h = (h + 3) / 4;
                mip_sizes.push(blocks_w * blocks_h * block_size);
                if w <= 1 && h <= 1 { break; }
                w = (w / 2).max(1);
                h = (h / 2).max(1);
            }
            let mip_count = mip_sizes.len();

            // Verify total size
            let total_mip_bytes: usize = mip_sizes.iter().sum();
            assert_eq!(ref_data.len(), fctx_header_size + total_mip_bytes,
                "ID {}: size mismatch: {} != {} + {}", id, ref_data.len(), fctx_header_size, total_mip_bytes);

            // Extract mips from reference (stored smallest-first after header)
            let mut ref_mips: Vec<&[u8]> = Vec::new();
            let mut offset = fctx_header_size;
            for i in (0..mip_count).rev() {
                ref_mips.push(&ref_data[offset..offset + mip_sizes[i]]);
                offset += mip_sizes[i];
            }
            // ref_mips[0] = smallest mip, ref_mips[mip_count-1] = mip0
            // Reverse so ref_mips[0] = mip0, ref_mips[1] = mip1, etc.
            ref_mips.reverse();

            // mip0 from the reference (CDN source, should be identical)
            let mip0 = ref_mips[0];

            // Regenerate mips using x87 float cascading
            // (matching the original: f32 float buffer across mips, x87 80-bit box
            // filter, fistp round-to-nearest for encoding)
            let mut current_w = width;
            let mut current_h = height;

            // Decode mip0 to pixel buffer → convert to f32
            let prev_bx = (current_w / 4).max(1);
            let prev_by = (current_h / 4).max(1);
            let mut decoded_u8: Vec<[u8; 4]> = vec![[0u8; 4]; current_w * current_h];
            for by in 0..prev_by {
                for bx in 0..prev_bx {
                    let off = (by * prev_bx + bx) * block_size;
                    if off + block_size > mip0.len() { continue; }
                    let pixels = decode_dxt1_block(&mip0[off..off + block_size]);
                    for py in 0..4 {
                        for px in 0..4 {
                            let x = bx * 4 + px;
                            let y = by * 4 + py;
                            if x < current_w && y < current_h {
                                decoded_u8[y * current_w + x] = pixels[py * 4 + px];
                            }
                        }
                    }
                }
            }
            let r255: f32 = 1.0 / 255.0;
            let mut cur_f: Vec<[f32; 4]> = decoded_u8.iter().map(|p| {
                [p[0] as f32 * r255, p[1] as f32 * r255,
                 p[2] as f32 * r255, p[3] as f32 * r255]
            }).collect();

            let saved_mip0_f = cur_f.clone();
            let saved_mip0_w = current_w;

            let mut tex_total = 0usize;
            let mut tex_match = 0usize;

            for mip_idx in 1..mip_count {
                let new_w = (current_w / 2).max(1);
                let new_h = (current_h / 2).max(1);

                // x87 box filter in float space
                let mut new_f = vec![[0.0f32; 4]; new_w * new_h];
                for y in 0..new_h {
                    for x in 0..new_w {
                        let x0 = (x*2).min(current_w-1);
                        let y0 = (y*2).min(current_h-1);
                        let x1 = (x*2+1).min(current_w-1);
                        let y1 = (y*2+1).min(current_h-1);
                        for c in 0..4 {
                            new_f[y*new_w+x][c] = x87_box_filter_f32(
                                cur_f[y0*current_w+x0][c], cur_f[y0*current_w+x1][c],
                                cur_f[y1*current_w+x0][c], cur_f[y1*current_w+x1][c],
                            );
                        }
                    }
                }

                // Encode: convert to uint8 via fistp, then encode blocks
                let nbx = (new_w/4).max(1);
                let nby = (new_h/4).max(1);
                let mut our_mip = Vec::with_capacity(nbx * nby * block_size);
                for by in 0..nby {
                    for bx in 0..nbx {
                        let mut bp = [[0u8; 4]; 16];
                        for py in 0..4 { for px in 0..4 {
                            let fx = (bx*4+px).min(new_w-1);
                            let fy = (by*4+py).min(new_h-1);
                            let fv = &new_f[fy*new_w+fx];
                            bp[py*4+px] = [
                                x87_float_to_u8(fv[0]), x87_float_to_u8(fv[1]),
                                x87_float_to_u8(fv[2]), x87_float_to_u8(fv[3]),
                            ];
                        }}
                        our_mip.extend_from_slice(&encode_dxt1_block(&bp, false, true));
                    }
                }

                let ref_mip = ref_mips[mip_idx];
                assert_eq!(our_mip.len(), ref_mip.len(),
                    "ID {} mip{}: size mismatch {} vs {}", id, mip_idx, our_mip.len(), ref_mip.len());

                // Compare block by block
                let num_blocks = our_mip.len() / block_size;
                let mut mip_match = 0usize;
                let mut mip_solid_match = 0usize;
                let mut mip_solid_total = 0usize;
                let mut mip_nonsolid_match = 0usize;
                for b in 0..num_blocks {
                    let our_block = &our_mip[b * block_size..(b + 1) * block_size];
                    let ref_block = &ref_mip[b * block_size..(b + 1) * block_size];
                    let rc0 = u16::from_le_bytes([ref_block[0], ref_block[1]]);
                    let rc1 = u16::from_le_bytes([ref_block[2], ref_block[3]]);
                    let is_solid = rc0 == rc1;
                    let matched = our_block == ref_block;
                    if matched { mip_match += 1; }
                    if is_solid {
                        mip_solid_total += 1;
                        if matched { mip_solid_match += 1; }
                    } else if matched {
                        mip_nonsolid_match += 1;
                    }

                    // For the first non-solid mismatch of texture 19767 mip1:
                    if id == 19767 && mip_idx == 1 && !matched && !is_solid && first_dump {
                        first_dump = false;
                        let bx = b % nbx;
                        let by = b / nbx;
                        eprintln!("\n  === PIXEL DIAGNOSTIC: block ({},{}) ===", bx, by);

                        // Our u8 pixels (re-derive from new_f)
                        let mut bp_dump = [[0u8; 4]; 16];
                        for py in 0..4 { for px in 0..4 {
                            let fx = (bx*4+px).min(new_w-1);
                            let fy = (by*4+py).min(new_h-1);
                            let fv = &new_f[fy*new_w+fx];
                            bp_dump[py*4+px] = [
                                x87_float_to_u8(fv[0]), x87_float_to_u8(fv[1]),
                                x87_float_to_u8(fv[2]), x87_float_to_u8(fv[3]),
                            ];
                            eprintln!("    px[{},{}]: u8=[{:3},{:3},{:3},{:3}]  f32=[{:.6},{:.6},{:.6},{:.6}]",
                                px, py,
                                bp_dump[py*4+px][0], bp_dump[py*4+px][1], bp_dump[py*4+px][2], bp_dump[py*4+px][3],
                                fv[0], fv[1], fv[2], fv[3]);
                        }}

                        // Reference block
                        let rc0 = u16::from_le_bytes([ref_block[0], ref_block[1]]);
                        let rc1 = u16::from_le_bytes([ref_block[2], ref_block[3]]);
                        let rix = u32::from_le_bytes([ref_block[4], ref_block[5], ref_block[6], ref_block[7]]);
                        eprintln!("    Ref: c0=0x{:04X} c1=0x{:04X} ix=0x{:08X}", rc0, rc1, rix);

                        let oc0 = u16::from_le_bytes([our_block[0], our_block[1]]);
                        let oc1 = u16::from_le_bytes([our_block[2], our_block[3]]);
                        let oix = u32::from_le_bytes([our_block[4], our_block[5], our_block[6], our_block[7]]);
                        eprintln!("    Ours: c0=0x{:04X} c1=0x{:04X} ix=0x{:08X}", oc0, oc1, oix);

                        // Decode reference and our block to compare pixels
                        let ref_px = decode_dxt1_block(ref_block);
                        let our_px = decode_dxt1_block(our_block);
                        eprintln!("    --- Decoded reference vs our block ---");
                        for i in 0..16 {
                            let ppx = i % 4;
                            let ppy = i / 4;
                            if ref_px[i] != our_px[i] {
                                eprintln!("    DIFF [{},{}]: ref=[{:3},{:3},{:3},{:3}] ours=[{:3},{:3},{:3},{:3}] input=[{:3},{:3},{:3},{:3}]",
                                    ppx, ppy,
                                    ref_px[i][0], ref_px[i][1], ref_px[i][2], ref_px[i][3],
                                    our_px[i][0], our_px[i][1], our_px[i][2], our_px[i][3],
                                    bp_dump[i][0], bp_dump[i][1], bp_dump[i][2], bp_dump[i][3]);
                            }
                        }

                        // Dump source mip0 pixels for px(2,2) of this block
                        let mpx = bx*4+2;
                        let mpy = by*4+2;
                        let x0 = (mpx*2).min(saved_mip0_w-1);
                        let y0 = (mpy*2).min(saved_mip0_w-1);
                        let x1 = (mpx*2+1).min(saved_mip0_w-1);
                        let y1 = (mpy*2+1).min(saved_mip0_w-1);
                        eprintln!("    Source mip0 pixels for px(2,2):");
                        for &(sx,sy) in &[(x0,y0),(x1,y0),(x0,y1),(x1,y1)] {
                            let u8px = decoded_u8[sy*saved_mip0_w+sx];
                            let fpx = saved_mip0_f[sy*saved_mip0_w+sx];
                            eprintln!("      ({:3},{:3}): u8=[{:3},{:3},{:3},{:3}] f32=[{:.8},{:.8},{:.8},{:.8}]",
                                sx, sy, u8px[0], u8px[1], u8px[2], u8px[3], fpx[0], fpx[1], fpx[2], fpx[3]);
                        }
                        // Box filter computation for each channel
                        let f00 = saved_mip0_f[y0*saved_mip0_w+x0];
                        let f10 = saved_mip0_f[y0*saved_mip0_w+x1];
                        let f01 = saved_mip0_f[y1*saved_mip0_w+x0];
                        let f11 = saved_mip0_f[y1*saved_mip0_w+x1];
                        for c in 0..3 {
                            let bf = x87_box_filter_f32(f00[c], f10[c], f01[c], f11[c]);
                            let u8val = x87_float_to_u8(bf);
                            let simple = (f00[c] + f10[c] + f01[c] + f11[c]) * 0.25;
                            let simple_u8 = (simple * 255.0) as u8;
                            eprintln!("      ch{}: box_f32={:.8}  u8={}  simple_f32={:.8}  simple_u8={}",
                                c, bf, u8val, simple, simple_u8);
                        }
                    }
                }

                eprintln!("  ID {} mip{} ({}x{}): {}/{} blocks match ({:.1}%) [solid {}/{}, nonsolid {}]",
                    id, mip_idx, new_w, new_h, mip_match, num_blocks,
                    (mip_match as f64 / num_blocks as f64) * 100.0,
                    mip_solid_match, mip_solid_total, mip_nonsolid_match);

                tex_total += num_blocks;
                tex_match += mip_match;

                // Keep float buffer for next level (matching original's float cascade)
                cur_f = new_f;
                current_w = new_w;
                current_h = new_h;
            }

            eprintln!("  ID {} TOTAL: {}/{} blocks ({:.1}%)\n",
                id, tex_match, tex_total, (tex_match as f64 / tex_total as f64) * 100.0);
            total_blocks += tex_total;
            matching_blocks += tex_match;
        }

        // === REF-CASCADED TEST: decode ref mipN, generate mipN+1, compare vs ref mipN+1 ===
        // This tests whether reference mips are cascaded (each derived from previous)
        // or independently generated (pre-generated from DDS)
        eprintln!("\n  === Ref-cascaded test (decode ref mipN → generate mipN+1) ===");
        for &(id, width, height) in &[(19767u32, 512usize, 512usize)] {
            let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let ref_data = {
                let mut file = std::fs::File::open(&ref_path).unwrap();
                file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
                let mut buf = vec![0u8; entry.length as usize];
                file.read_exact(&mut buf).unwrap();
                buf
            };
            let mut ms = Vec::new(); let mut w = width; let mut h = height;
            loop { ms.push(((w+3)/4)*((h+3)/4)*block_size); if w<=1&&h<=1{break;} w=(w/2).max(1); h=(h/2).max(1); }
            let mc = ms.len();
            let mut rm: Vec<&[u8]> = Vec::new();
            let mut off = fctx_header_size;
            for i in (0..mc).rev() { rm.push(&ref_data[off..off+ms[i]]); off+=ms[i]; }
            rm.reverse();

            let mut cw = width; let mut ch = height;
            for mi in 1..mc.min(5) {
                let nw = (cw/2).max(1); let nh = (ch/2).max(1);
                // Decode ref mip[mi-1] → pixels
                let pbx = (cw/4).max(1); let pby = (ch/4).max(1);
                let mut pix = vec![[0u8;4]; cw*ch];
                for by in 0..pby { for bx in 0..pbx {
                    let o = (by*pbx+bx)*block_size;
                    if o+block_size > rm[mi-1].len() { continue; }
                    let px = decode_dxt1_block(&rm[mi-1][o..o+block_size]);
                    for py in 0..4 { for ppx in 0..4 {
                        let (xx,yy) = (bx*4+ppx, by*4+py);
                        if xx<cw && yy<ch { pix[yy*cw+xx] = px[py*4+ppx]; }
                    }}
                }}
                // Generate mipN+1 from decoded ref mipN
                let (gen, _) = generate_mip_from_pixels(&pix, cw, ch, nw, nh, TextureCodec::Dxt1, false);
                let r = rm[mi];
                let nb = r.len() / block_size;
                let mut mm = 0;
                for b in 0..nb {
                    if gen[b*8..(b+1)*8] == r[b*8..(b+1)*8] { mm += 1; }
                }
                eprintln!("  [ref-cascade] ID {} mip{} from ref-mip{}: {}/{} ({:.1}%)",
                    id, mi, mi-1, mm, nb, mm as f64/nb as f64*100.0);
                cw = nw; ch = nh;
            }
        }

        // Separate scan: count solid vs non-solid block matches across all textures
        let mut solid_match = 0usize;
        let mut solid_total = 0usize;
        let mut nonsolid_match = 0usize;
        let mut nonsolid_total = 0usize;
        let mut nonsolid_same_endpoints = 0usize;
        let mut nonsolid_diff_endpoints = 0usize;

        for &(id, width, height) in test_textures {
            let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let ref_data = {
                let mut file = std::fs::File::open(&ref_path).unwrap();
                file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
                let mut buf = vec![0u8; entry.length as usize];
                file.read_exact(&mut buf).unwrap();
                buf
            };

            let mut mip_sizes = Vec::new();
            let mut w = width;
            let mut h = height;
            loop {
                let bw = (w + 3) / 4;
                let bh = (h + 3) / 4;
                mip_sizes.push(bw * bh * block_size);
                if w <= 1 && h <= 1 { break; }
                w = (w / 2).max(1);
                h = (h / 2).max(1);
            }
            let mip_count = mip_sizes.len();

            // Extract ref mips (stored smallest-first after header)
            let mut ref_mips: Vec<&[u8]> = Vec::new();
            let mut offset = fctx_header_size;
            for i in (0..mip_count).rev() {
                ref_mips.push(&ref_data[offset..offset + mip_sizes[i]]);
                offset += mip_sizes[i];
            }
            ref_mips.reverse(); // ref_mips[0] = mip0

            // Regenerate mips using cascaded pixel buffers, tracking solid vs non-solid
            let mut current_w = width;
            let mut current_h = height;

            // Decode mip0 to pixel buffer
            let pbx = (current_w / 4).max(1);
            let pby = (current_h / 4).max(1);
            let mut cur_pix: Vec<[u8; 4]> = vec![[0u8; 4]; current_w * current_h];
            for by in 0..pby {
                for bx in 0..pbx {
                    let off = (by * pbx + bx) * block_size;
                    if off + block_size > ref_mips[0].len() { continue; }
                    let pixels = decode_dxt1_block(&ref_mips[0][off..off + block_size]);
                    for py in 0..4 { for px in 0..4 {
                        let x = bx * 4 + px;
                        let y = by * 4 + py;
                        if x < current_w && y < current_h {
                            cur_pix[y * current_w + x] = pixels[py * 4 + px];
                        }
                    }}
                }
            }

            for mip_idx in 1..mip_count {
                let new_w = (current_w / 2).max(1);
                let new_h = (current_h / 2).max(1);

                let (our_mip, new_pix) = generate_mip_from_pixels(
                    &cur_pix, current_w, current_h, new_w, new_h,
                    TextureCodec::Dxt1, false,
                );

                let ref_mip = ref_mips[mip_idx];
                let num_blocks = ref_mip.len() / block_size;

                for b in 0..num_blocks {
                    let ref_block = &ref_mip[b * block_size..(b + 1) * block_size];
                    let our_block = &our_mip[b * block_size..(b + 1) * block_size];
                    let c0 = u16::from_le_bytes([ref_block[0], ref_block[1]]);
                    let c1 = u16::from_le_bytes([ref_block[2], ref_block[3]]);
                    let solid = c0 == c1;
                    let matched = our_block == ref_block;
                    if solid {
                        solid_total += 1;
                        if matched { solid_match += 1; }
                    } else {
                        nonsolid_total += 1;
                        if matched {
                            nonsolid_match += 1;
                        } else {
                            if our_block[0..4] == ref_block[0..4] {
                                nonsolid_same_endpoints += 1;
                            } else {
                                nonsolid_diff_endpoints += 1;
                            }
                        }
                    }
                }

                // Diagnostic: for ID 19767 mip1, show first 5 mismatched non-solid blocks
                if id == 19767 && mip_idx == 1 {
                    let nbx = (new_w / 4).max(1);
                    let mut diag_count = 0;
                    for b in 0..num_blocks {
                        if diag_count >= 5 { break; }
                        let ref_block = &ref_mip[b*block_size..(b+1)*block_size];
                        let our_block = &our_mip[b*block_size..(b+1)*block_size];
                        let rc0 = u16::from_le_bytes([ref_block[0], ref_block[1]]);
                        let rc1 = u16::from_le_bytes([ref_block[2], ref_block[3]]);
                        if rc0 == rc1 || ref_block == our_block { continue; }
                        diag_count += 1;
                        let oc0 = u16::from_le_bytes([our_block[0], our_block[1]]);
                        let oc1 = u16::from_le_bytes([our_block[2], our_block[3]]);
                        let bx = b % nbx; let by = b / nbx;
                        // Show pixel values for this block
                        let mut pixels = [[0u8;4];16];
                        for py in 0..4 { for px in 0..4 {
                            let x = (bx*4+px).min(new_w-1);
                            let y = (by*4+py).min(new_h-1);
                            pixels[py*4+px] = new_pix[y*new_w+x];
                        }}
                        let (rr0,rg0,rb0) = decode_rgb565(rc0);
                        let (rr1,rg1,rb1) = decode_rgb565(rc1);
                        let (or0,og0,ob0) = decode_rgb565(oc0);
                        let (or1,og1,ob1) = decode_rgb565(oc1);
                        eprintln!("  DIAG block ({},{}) ref c0={:04X}({},{},{}) c1={:04X}({},{},{}) idx={:08X}",
                            bx, by, rc0, rr0, rg0, rb0, rc1, rr1, rg1, rb1,
                            u32::from_le_bytes([ref_block[4],ref_block[5],ref_block[6],ref_block[7]]));
                        eprintln!("              ours c0={:04X}({},{},{}) c1={:04X}({},{},{}) idx={:08X}",
                            oc0, or0, og0, ob0, oc1, or1, og1, ob1,
                            u32::from_le_bytes([our_block[4],our_block[5],our_block[6],our_block[7]]));
                        // Show pixel bounding box and a few pixel values
                        let mut pmin = [255u8;3]; let mut pmax = [0u8;3];
                        for p in &pixels { for c in 0..3 {
                            pmin[c] = pmin[c].min(p[c]); pmax[c] = pmax[c].max(p[c]);
                        }}
                        eprintln!("              bbox min=({},{},{}) max=({},{},{})  px[0]=({},{},{}) px[15]=({},{},{})",
                            pmin[0],pmin[1],pmin[2], pmax[0],pmax[1],pmax[2],
                            pixels[0][0],pixels[0][1],pixels[0][2],
                            pixels[15][0],pixels[15][1],pixels[15][2]);
                        // Dump ALL 16 pixels as BGRA u32 for Unicorn testing (block 3,0 only)
                        if bx == 3 && by == 0 {
                            eprintln!("              ALL16 (BGRA u32 for Unicorn):");
                            for pi in 0..16 {
                                let p = pixels[pi];
                                let bgra = (p[3] as u32) << 24 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | (p[2] as u32);
                                eprint!(" 0x{:08X}", bgra);
                                if pi % 4 == 3 { eprintln!(); }
                            }
                        }
                    }
                }

                cur_pix = new_pix;
                current_w = new_w;
                current_h = new_h;
            }
        }

        eprintln!("  Solid blocks:     {}/{} match ({:.2}%)",
            solid_match, solid_total,
            if solid_total > 0 { (solid_match as f64 / solid_total as f64) * 100.0 } else { 0.0 });
        eprintln!("  Non-solid blocks: {}/{} match ({:.2}%)",
            nonsolid_match, nonsolid_total,
            if nonsolid_total > 0 { (nonsolid_match as f64 / nonsolid_total as f64) * 100.0 } else { 0.0 });

        let nonsolid_mismatch = nonsolid_total - nonsolid_match;
        eprintln!("  Non-solid endpoint analysis:");
        eprintln!("    Same endpoints, different indices: {}/{}",
            nonsolid_same_endpoints, nonsolid_mismatch);
        eprintln!("    Different endpoints: {}/{}",
            nonsolid_diff_endpoints, nonsolid_mismatch);

        // Count 3-color mode blocks in reference (c0 < c1)
        // If the reference has 3-color blocks but we only produce 4-color,
        // that explains some mismatches
        let mut ref_3color = 0usize;
        let mut our_3color = 0usize;
        // Re-scan to check
        for &(id, width, height) in test_textures {
            let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let ref_data = {
                let mut file = std::fs::File::open(&ref_path).unwrap();
                file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
                let mut buf = vec![0u8; entry.length as usize];
                file.read_exact(&mut buf).unwrap();
                buf
            };
            let mut ms = Vec::new();
            let mut w = width; let mut h = height;
            loop {
                ms.push(((w+3)/4) * ((h+3)/4) * block_size);
                if w <= 4 && h <= 4 { break; }
                w = (w/2).max(1); h = (h/2).max(1);
            }
            let mut rm: Vec<&[u8]> = Vec::new();
            let mut off = fctx_header_size;
            for i in (0..ms.len()).rev() { rm.push(&ref_data[off..off+ms[i]]); off += ms[i]; }
            rm.reverse();

            let mut cw = width; let mut ch = height;
            let pbx2 = (cw/4).max(1); let pby2 = (ch/4).max(1);
            let mut cp: Vec<[u8;4]> = vec![[0u8;4]; cw*ch];
            for by in 0..pby2 { for bx in 0..pbx2 {
                let o = (by*pbx2+bx)*block_size;
                if o+block_size > rm[0].len() { continue; }
                let px = decode_dxt1_block(&rm[0][o..o+block_size]);
                for py in 0..4 { for ppx in 0..4 {
                    let xx = bx*4+ppx; let yy = by*4+py;
                    if xx < cw && yy < ch { cp[yy*cw+xx] = px[py*4+ppx]; }
                }}
            }}

            for mi in 1..ms.len() {
                let nw = (cw/2).max(1); let nh = (ch/2).max(1);
                let (om, np) = generate_mip_from_pixels(&cp, cw, ch, nw, nh, TextureCodec::Dxt1, false);
                let rr = rm[mi];
                let nb = rr.len() / block_size;
                for b in 0..nb {
                    let rc0 = u16::from_le_bytes([rr[b*8], rr[b*8+1]]);
                    let rc1 = u16::from_le_bytes([rr[b*8+2], rr[b*8+3]]);
                    if rc0 != rc1 && rc0 < rc1 { ref_3color += 1; }
                    let oc0 = u16::from_le_bytes([om[b*8], om[b*8+1]]);
                    let oc1 = u16::from_le_bytes([om[b*8+2], om[b*8+3]]);
                    if oc0 != oc1 && oc0 < oc1 { our_3color += 1; }
                }
                cp = np; cw = nw; ch = nh;
            }
        }
        eprintln!("  3-color mode blocks: ref={} ours={}", ref_3color, our_3color);

        eprintln!("  OVERALL: {}/{} blocks match ({:.2}%)",
            matching_blocks, total_blocks,
            (matching_blocks as f64 / total_blocks as f64) * 100.0);

        // === GAMMA-CORRECT pipeline comparison ===
        // === FLOAT CASCADING (no gamma, float precision between mips) ===
        eprintln!("\n  === Float cascading (no gamma) ===");
        {
            let mut float_total = 0usize;
            let mut float_match = 0usize;
            for &(id, width, height) in test_textures {
                let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
                let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
                let ref_data = {
                    let mut file = std::fs::File::open(&ref_path).unwrap();
                    file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
                    let mut buf = vec![0u8; entry.length as usize];
                    file.read_exact(&mut buf).unwrap();
                    buf
                };
                let mut ms = Vec::new();
                let (mut w, mut h) = (width, height);
                loop { ms.push(((w+3)/4)*((h+3)/4)*block_size); if w<=1&&h<=1{break;} w=(w/2).max(1); h=(h/2).max(1); }
                let mc = ms.len();
                let mut rm: Vec<&[u8]> = Vec::new();
                let mut off = fctx_header_size;
                for i in (0..mc).rev() { rm.push(&ref_data[off..off+ms[i]]); off+=ms[i]; }
                rm.reverse();

                let (mut cw, mut ch) = (width, height);
                let (pbx, pby) = ((cw/4).max(1), (ch/4).max(1));
                let mut decoded = vec![[0u8;4]; cw*ch];
                for by in 0..pby { for bx in 0..pbx {
                    let o2 = (by*pbx+bx)*block_size;
                    if o2+block_size > rm[0].len() { continue; }
                    let px = decode_dxt1_block(&rm[0][o2..o2+block_size]);
                    for py in 0..4 { for ppx in 0..4 {
                        let (xx,yy) = (bx*4+ppx, by*4+py);
                        if xx<cw && yy<ch { decoded[yy*cw+xx] = px[py*4+ppx]; }
                    }}
                }}
                let r255: f32 = 1.0/255.0;
                let mut cur_f: Vec<[f32;4]> = decoded.iter().map(|p| {
                    [p[0] as f32 * r255, p[1] as f32 * r255,
                     p[2] as f32 * r255, p[3] as f32 * r255]
                }).collect();

                for mi in 1..mc {
                    let (nw, nh) = ((cw/2).max(1), (ch/2).max(1));
                    let mut new_f = vec![[0.0f32;4]; nw*nh];
                    for y in 0..nh { for x in 0..nw {
                        let (x0,y0) = ((x*2).min(cw-1), (y*2).min(ch-1));
                        let (x1,y1) = ((x*2+1).min(cw-1), (y*2+1).min(ch-1));
                        for c in 0..4 { new_f[y*nw+x][c] = x87_box_filter_f32(cur_f[y0*cw+x0][c],cur_f[y0*cw+x1][c],cur_f[y1*cw+x0][c],cur_f[y1*cw+x1][c]); }
                    }}
                    let mut dst_u8 = vec![[0u8;4]; nw*nh];
                    for i in 0..nw*nh { for c in 0..4 { dst_u8[i][c] = x87_float_to_u8(new_f[i][c]); }}
                    let nbx=(nw/4).max(1); let nby=(nh/4).max(1);
                    let mut mip = Vec::with_capacity(nbx*nby*block_size);
                    for by2 in 0..nby { for bx2 in 0..nbx {
                        let mut bp=[[0u8;4];16];
                        for py in 0..4{for px in 0..4{bp[py*4+px]=dst_u8[((by2*4+py).min(nh-1))*nw+(bx2*4+px).min(nw-1)];}}
                        mip.extend_from_slice(&encode_dxt1_block(&bp,false,true));
                    }}
                    let r=rm[mi]; let nb=r.len()/block_size;
                    let mut mm=0;
                    for b in 0..nb{if mip[b*8..(b+1)*8]==r[b*8..(b+1)*8]{mm+=1;}}
                    if id==19767{ eprintln!("  [float] mip{} ({}x{}): {}/{} ({:.1}%)",mi,nw,nh,mm,nb,mm as f64/nb as f64*100.0); }
                    float_total+=nb; float_match+=mm;
                    cur_f=new_f; cw=nw; ch=nh;
                }
            }
            eprintln!("  [float] OVERALL: {}/{} ({:.2}%)", float_match,float_total,float_match as f64/float_total as f64*100.0);
        }

        eprintln!("\n  === GAMMA 2.2 (linear-space filtering) ===");
        let mut gamma_total = 0usize;
        let mut gamma_match = 0usize;
        let recip255g: f32 = 1.0 / 255.0;
        let gamma_val: f32 = 2.2;

        for &(id, width, height) in test_textures {
            let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let ref_data = {
                let mut file = std::fs::File::open(&ref_path).unwrap();
                file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
                let mut buf = vec![0u8; entry.length as usize];
                file.read_exact(&mut buf).unwrap();
                buf
            };
            let mut ms = Vec::new();
            let (mut w, mut h) = (width, height);
            loop { ms.push(((w+3)/4)*((h+3)/4)*block_size); if w<=1&&h<=1{break;} w=(w/2).max(1); h=(h/2).max(1); }
            let mc = ms.len();
            let mut rm: Vec<&[u8]> = Vec::new();
            let mut off = fctx_header_size;
            for i in (0..mc).rev() { rm.push(&ref_data[off..off+ms[i]]); off+=ms[i]; }
            rm.reverse();

            // Decode mip0 → uint8 → float → linearize
            let (mut cw, mut ch) = (width, height);
            let (pbx, pby) = ((cw/4).max(1), (ch/4).max(1));
            let mut decoded = vec![[0u8;4]; cw*ch];
            for by in 0..pby { for bx in 0..pbx {
                let o2 = (by*pbx+bx)*block_size;
                if o2+block_size > rm[0].len() { continue; }
                let px = decode_dxt1_block(&rm[0][o2..o2+block_size]);
                for py in 0..4 { for ppx in 0..4 {
                    let (xx,yy) = (bx*4+ppx, by*4+py);
                    if xx<cw && yy<ch { decoded[yy*cw+xx] = px[py*4+ppx]; }
                }}
            }}
            // Convert to linear float
            let mut cur_lin: Vec<[f32;4]> = decoded.iter().map(|p| {
                [x87_powf(p[0] as f32 * recip255g, gamma_val),
                 x87_powf(p[1] as f32 * recip255g, gamma_val),
                 x87_powf(p[2] as f32 * recip255g, gamma_val),
                 p[3] as f32 * recip255g]
            }).collect();

            let mut t_total = 0usize;
            let mut t_match = 0usize;
            for mi in 1..mc {
                let (nw, nh) = ((cw/2).max(1), (ch/2).max(1));
                let (mip, new_lin) = generate_mip_from_linear(
                    &cur_lin, cw, ch, nw, nh, TextureCodec::Dxt1, false);
                let r = rm[mi];
                let nb = r.len() / block_size;
                let mut mm = 0;
                for b in 0..nb {
                    if mip[b*8..(b+1)*8] == r[b*8..(b+1)*8] { mm += 1; }
                }
                if id == 19767 {
                    eprintln!("  [gamma] ID {} mip{} ({}x{}): {}/{} ({:.1}%)",
                        id, mi, nw, nh, mm, nb, mm as f64/nb as f64*100.0);
                }
                t_total += nb; t_match += mm;
                cur_lin = new_lin; cw = nw; ch = nh;
            }
            gamma_total += t_total; gamma_match += t_match;
        }
        eprintln!("  [gamma] OVERALL: {}/{} ({:.2}%)",
            gamma_match, gamma_total, gamma_match as f64/gamma_total as f64*100.0);

        // === NVTT GAMMA (polynomial fast-path linearization) ===
        eprintln!("\n  === NVTT gamma (nvtt_powf_11_5 / nvtt_powf_5_11) ===");
        let mut nvtt_total = 0usize;
        let mut nvtt_match = 0usize;

        for &(id, width, height) in test_textures {
            let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let ref_data = {
                let mut file = std::fs::File::open(&ref_path).unwrap();
                file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
                let mut buf = vec![0u8; entry.length as usize];
                file.read_exact(&mut buf).unwrap();
                buf
            };
            let mut ms = Vec::new();
            let (mut w, mut h) = (width, height);
            loop { ms.push(((w+3)/4)*((h+3)/4)*block_size); if w<=1&&h<=1{break;} w=(w/2).max(1); h=(h/2).max(1); }
            let mc = ms.len();
            let mut rm: Vec<&[u8]> = Vec::new();
            let mut off = fctx_header_size;
            for i in (0..mc).rev() { rm.push(&ref_data[off..off+ms[i]]); off+=ms[i]; }
            rm.reverse();

            // Decode mip0 -> uint8 -> linearize with nvtt_powf_11_5
            let (mut cw, mut ch) = (width, height);
            let (pbx, pby) = ((cw/4).max(1), (ch/4).max(1));
            let mut decoded = vec![[0u8;4]; cw*ch];
            for by in 0..pby { for bx in 0..pbx {
                let o2 = (by*pbx+bx)*block_size;
                if o2+block_size > rm[0].len() { continue; }
                let px = decode_dxt1_block(&rm[0][o2..o2+block_size]);
                for py in 0..4 { for ppx in 0..4 {
                    let (xx,yy) = (bx*4+ppx, by*4+py);
                    if xx<cw && yy<ch { decoded[yy*cw+xx] = px[py*4+ppx]; }
                }}
            }}
            let r255nvtt: f32 = 1.0 / 255.0;
            let mut cur_lin: Vec<[f32;4]> = decoded.iter().map(|p| {
                [nvtt_powf_11_5(p[0] as f32 * r255nvtt),
                 nvtt_powf_11_5(p[1] as f32 * r255nvtt),
                 nvtt_powf_11_5(p[2] as f32 * r255nvtt),
                 p[3] as f32 * r255nvtt]
            }).collect();

            let mut t_total = 0usize;
            let mut t_match = 0usize;
            for mi in 1..mc {
                let (nw, nh) = ((cw/2).max(1), (ch/2).max(1));
                // Box filter in linear space
                let mut new_lin = vec![[0.0f32;4]; nw*nh];
                for y in 0..nh { for x in 0..nw {
                    let (x0,y0) = ((x*2).min(cw-1), (y*2).min(ch-1));
                    let (x1,y1) = ((x*2+1).min(cw-1), (y*2+1).min(ch-1));
                    for c in 0..4 {
                        new_lin[y*nw+x][c] = x87_box_filter_f32(
                            cur_lin[y0*cw+x0][c], cur_lin[y0*cw+x1][c],
                            cur_lin[y1*cw+x0][c], cur_lin[y1*cw+x1][c],
                        );
                    }
                }}
                // De-linearize with nvtt_powf_5_11, scale by 255, floor
                let mut dst_u8 = vec![[0u8;4]; nw*nh];
                for i in 0..nw*nh {
                    for c in 0..3 {
                        let srgb = nvtt_powf_5_11(new_lin[i][c]);
                        let v = (srgb * 255.0).floor();
                        dst_u8[i][c] = v.clamp(0.0, 255.0) as u8;
                    }
                    dst_u8[i][3] = (new_lin[i][3] * 255.0).max(0.0).min(255.0) as u8;
                }
                // Encode DXT1
                let nbx = (nw/4).max(1); let nby = (nh/4).max(1);
                let mut mip = Vec::with_capacity(nbx*nby*block_size);
                for by2 in 0..nby { for bx2 in 0..nbx {
                    let mut bp = [[0u8;4];16];
                    for py in 0..4 { for px in 0..4 {
                        bp[py*4+px] = dst_u8[((by2*4+py).min(nh-1))*nw + (bx2*4+px).min(nw-1)];
                    }}
                    mip.extend_from_slice(&encode_dxt1_block(&bp, false, true));
                }}
                let r = rm[mi]; let nb = r.len()/block_size;
                let mut mm = 0;
                for b in 0..nb {
                    if mip[b*8..(b+1)*8] == r[b*8..(b+1)*8] { mm += 1; }
                }
                if id == 19767 {
                    eprintln!("  [nvtt-gamma] ID {} mip{} ({}x{}): {}/{} ({:.1}%)",
                        id, mi, nw, nh, mm, nb, mm as f64/nb as f64*100.0);
                }
                t_total += nb; t_match += mm;
                cur_lin = new_lin; cw = nw; ch = nh;
            }
            nvtt_total += t_total; nvtt_match += t_match;
        }
        eprintln!("  [nvtt-gamma] OVERALL: {}/{} ({:.2}%)",
            nvtt_match, nvtt_total, nvtt_match as f64/nvtt_total as f64*100.0);
    }

    #[test]
    #[ignore]
    fn encoder_replication_diagnose_block() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        // Read texture 19767 (512x512 DXT1)
        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut file = std::fs::File::open(&ref_path).unwrap();
            file.seek(std::io::SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            file.read_exact(&mut buf).unwrap();
            buf
        };

        let fctx_hdr_size = 40;
        let block_size = 8; // DXT1
        let width = 512;
        let height = 512;

        // Mip sizes for 512x512 DXT1
        let mut mip_sizes = Vec::new();
        let mut w = width;
        let mut h = height;
        loop {
            let bw = (w + 3) / 4;
            let bh = (h + 3) / 4;
            mip_sizes.push(bw * bh * block_size);
            if w <= 4 && h <= 4 { break; }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }

        // Extract mip0 and mip1 from reference (stored smallest-first)
        let mut offsets = vec![0usize; mip_sizes.len()];
        let mut off = fctx_hdr_size;
        for i in (0..mip_sizes.len()).rev() {
            offsets[i] = off;
            off += mip_sizes[i];
        }
        let ref_mip0 = &ref_data[offsets[0]..offsets[0] + mip_sizes[0]];
        let ref_mip1 = &ref_data[offsets[1]..offsets[1] + mip_sizes[1]];

        // Generate mip1 from mip0 using our pipeline
        let our_mip1 = generate_mip(ref_mip0, 512, 512, 256, 256, TextureCodec::Dxt1, false);

        // Find first mismatching block
        let num_blocks = our_mip1.len() / block_size;
        let mut first_diff = None;
        for b in 0..num_blocks {
            let ours = &our_mip1[b*block_size..(b+1)*block_size];
            let refs = &ref_mip1[b*block_size..(b+1)*block_size];
            if ours != refs {
                first_diff = Some(b);
                break;
            }
        }

        let b = first_diff.expect("No differences found");
        let our_block = &our_mip1[b*block_size..(b+1)*block_size];
        let ref_block = &ref_mip1[b*block_size..(b+1)*block_size];

        eprintln!("First mismatching block index: {} (of {})", b, num_blocks);
        eprintln!("  Ref block: {:02x?}", ref_block);
        eprintln!("  Our block: {:02x?}", our_block);

        // Decode both blocks to see the pixel difference
        let ref_pixels = decode_dxt1_block(ref_block);
        let our_pixels = decode_dxt1_block(our_block);
        eprintln!("  Ref decoded pixels (first 4):");
        for i in 0..4 { eprintln!("    [{}: {:?}]", i, ref_pixels[i]); }
        eprintln!("  Our decoded pixels (first 4):");
        for i in 0..4 { eprintln!("    [{}: {:?}]", i, our_pixels[i]); }

        // Now decode the 2x2 source blocks from mip0 that feed into this mip1 block
        // Block b in mip1 (256x256) corresponds to a 2x2 group of blocks in mip0 (512x512)
        let mip1_blocks_w = 256 / 4; // 64 blocks wide
        let block_y = b / mip1_blocks_w;
        let block_x = b % mip1_blocks_w;

        // The 4 source blocks in mip0
        let mip0_blocks_w = 512 / 4; // 128 blocks wide
        let src_blocks = [
            (block_y * 2) * mip0_blocks_w + (block_x * 2),
            (block_y * 2) * mip0_blocks_w + (block_x * 2 + 1),
            (block_y * 2 + 1) * mip0_blocks_w + (block_x * 2),
            (block_y * 2 + 1) * mip0_blocks_w + (block_x * 2 + 1),
        ];

        eprintln!("\n  Source mip0 blocks for this mip1 block:");
        for (i, &sb) in src_blocks.iter().enumerate() {
            let src = &ref_mip0[sb*block_size..(sb+1)*block_size];
            eprintln!("    src[{}] (block {}): {:02x?}", i, sb, src);
        }

        // Now generate the input pixels by decoding mip0 and box-filtering
        // This is what generate_mip does internally. Let's trace it.
        // Decode the 4 source blocks to get 4x4 pixels each = 8x8 pixel region
        let mut src_pixels = [[0u8; 4]; 64]; // 8x8
        for (qi, &sb) in src_blocks.iter().enumerate() {
            let src_block = &ref_mip0[sb*block_size..(sb+1)*block_size];
            let decoded = decode_dxt1_block(src_block);
            let qy = qi / 2; // 0 or 1
            let qx = qi % 2; // 0 or 1
            for py in 0..4 {
                for px in 0..4 {
                    src_pixels[(qy * 4 + py) * 8 + (qx * 4 + px)] = decoded[py * 4 + px];
                }
            }
        }

        // Box-filter 2x2 to get 4x4 block for the encoder
        let mut filtered = [[0u8; 4]; 16];
        for by in 0..4 {
            for bx in 0..4 {
                let mut r = 0.0f32;
                let mut g = 0.0f32;
                let mut b_val = 0.0f32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let p = src_pixels[(by * 2 + dy) * 8 + (bx * 2 + dx)];
                        r += p[0] as f32;
                        g += p[1] as f32;
                        b_val += p[2] as f32;
                    }
                }
                filtered[by * 4 + bx] = [
                    (r / 4.0 + 0.5) as u8,
                    (g / 4.0 + 0.5) as u8,
                    (b_val / 4.0 + 0.5) as u8,
                    255,
                ];
            }
        }

        eprintln!("\n  Box-filtered 4x4 pixels (input to encoder):");
        for row in 0..4 {
            eprintln!("    row {}: {:?} {:?} {:?} {:?}",
                row,
                filtered[row*4], filtered[row*4+1], filtered[row*4+2], filtered[row*4+3]);
        }

        // Now encode with our encoder and the reference encoder (which we have the output of)
        let (our_encoded, _) = cluster_fit_4color(&filtered, true);
        eprintln!("\n  Encoding the filtered pixels:");
        eprintln!("    Ref mip1 block: {:02x?}", ref_block);
        eprintln!("    Our re-encoded: {:02x?}", our_encoded);
        eprintln!("    Match: {}", our_encoded == ref_block);

        // If our re-encoded matches, the issue is in how generate_mip box-filters
        // If our re-encoded doesn't match, the issue is in the encoder
    }

    #[test]
    #[ignore]
    fn encoder_replication_diagnose_nonsolid() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut file = std::fs::File::open(&ref_path).unwrap();
            file.seek(std::io::SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            file.read_exact(&mut buf).unwrap();
            buf
        };

        let width = 512usize;
        let height = 512usize;
        let block_size = 8;
        let fctx_hdr = 40;

        // Compute mip offsets (stored smallest-first)
        let mut mip_sizes = Vec::new();
        let (mut w, mut h) = (width, height);
        loop {
            mip_sizes.push(((w+3)/4) * ((h+3)/4) * block_size);
            if w <= 4 && h <= 4 { break; }
            w = (w/2).max(1); h = (h/2).max(1);
        }
        let mut offsets = vec![0usize; mip_sizes.len()];
        let mut off = fctx_hdr;
        for i in (0..mip_sizes.len()).rev() { offsets[i] = off; off += mip_sizes[i]; }

        let ref_mip0 = &ref_data[offsets[0]..offsets[0]+mip_sizes[0]];
        let ref_mip1 = &ref_data[offsets[1]..offsets[1]+mip_sizes[1]];

        // Generate our mip1
        let our_mip1 = generate_mip(ref_mip0, 512, 512, 256, 256, TextureCodec::Dxt1, false);

        // Now replicate the internal pixel generation to get the input pixels
        // Step 1: Decode mip0 to pixel buffer
        let prev_bx = 512 / 4;
        let prev_by = 512 / 4;
        let mut src_pixels = vec![[0u8; 4]; 512 * 512];
        for by in 0..prev_by {
            for bx in 0..prev_bx {
                let block_off = (by * prev_bx + bx) * block_size;
                let block = &ref_mip0[block_off..block_off+block_size];
                let block_pixels = decode_dxt1_block(block);
                for py in 0..4 {
                    for px in 0..4 {
                        src_pixels[(by*4+py)*512 + bx*4+px] = block_pixels[py*4+px];
                    }
                }
            }
        }

        // Step 2: Box-filter to 256x256
        let mut dst_pixels = vec![[0u8; 4]; 256 * 256];
        for y in 0..256 {
            for x in 0..256 {
                let p00 = src_pixels[(y*2)*512 + x*2];
                let p10 = src_pixels[(y*2)*512 + x*2+1];
                let p01 = src_pixels[(y*2+1)*512 + x*2];
                let p11 = src_pixels[(y*2+1)*512 + x*2+1];
                for c in 0..3 {
                    let avg = (p00[c] as f32 / 255.0 + p10[c] as f32 / 255.0
                              + p01[c] as f32 / 255.0 + p11[c] as f32 / 255.0) / 4.0;
                    dst_pixels[y * 256 + x][c] = (avg * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
                }
                dst_pixels[y * 256 + x][3] = 255;
            }
        }

        // Find first 5 non-solid mismatching blocks
        let mip1_bx = 256 / 4; // 64
        let num_blocks = our_mip1.len() / block_size;
        let mut found = 0;
        for b in 0..num_blocks {
            let our_block = &our_mip1[b*block_size..(b+1)*block_size];
            let ref_block = &ref_mip1[b*block_size..(b+1)*block_size];
            if our_block == ref_block { continue; }

            // Extract 4x4 pixel block
            let bx_pos = b % mip1_bx;
            let by_pos = b / mip1_bx;
            let mut pixels = [[0u8; 4]; 16];
            for py in 0..4 {
                for px in 0..4 {
                    pixels[py*4+px] = dst_pixels[(by_pos*4+py)*256 + bx_pos*4+px];
                }
            }

            // Check if solid
            let first = (pixels[0][0], pixels[0][1], pixels[0][2]);
            let is_solid = pixels.iter().all(|p| (p[0], p[1], p[2]) == first);
            if is_solid { continue; }

            // Encode with our encoder
            let our_encoded = encode_dxt1_block(&pixels, false, false);

            // Count unique colors
            let mut unique = std::collections::HashSet::new();
            for p in &pixels { unique.insert((p[0], p[1], p[2])); }

            eprintln!("\n  Block {} (pos {},{}) — {} unique colors:", b, bx_pos, by_pos, unique.len());
            eprintln!("    Ref block:   {:02x?}", ref_block);
            eprintln!("    Our mip1:    {:02x?}", our_block);
            eprintln!("    Our encode:  {:02x?}", &our_encoded[..]);
            eprintln!("    Our==OurMip: {}", our_block == &our_encoded[..]);

            // Show first few pixels
            for row in 0..2 {
                eprintln!("    pixels row{}: {:?} {:?} {:?} {:?}",
                    row, pixels[row*4], pixels[row*4+1], pixels[row*4+2], pixels[row*4+3]);
            }

            // Decode ref and our blocks to see RGB difference
            let ref_decoded = decode_dxt1_block(ref_block);
            let our_decoded = decode_dxt1_block(our_block);
            let mut max_diff = 0i32;
            for i in 0..16 {
                for c in 0..3 {
                    let d = (ref_decoded[i][c] as i32 - our_decoded[i][c] as i32).abs();
                    max_diff = max_diff.max(d);
                }
            }
            eprintln!("    Max pixel diff (ref vs our decoded): {}", max_diff);

            found += 1;
            if found >= 5 { break; }
        }

        if found == 0 {
            eprintln!("No non-solid mismatching blocks found!");
        }
    }

    #[test]
    #[ignore]
    fn encoder_replication_pixel_compare() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let fctx_hdr = 40;
        let block_size = 8;
        // Extract mip0 (at end, smallest-first storage)
        let mip0_size = 128 * 128 * block_size; // 131072
        let mip0_offset = ref_data.len() - mip0_size;
        let ref_mip0 = &ref_data[mip0_offset..];

        // Extract reference mip1 (256x256 = 64*64 blocks = 32768 bytes, right before mip0)
        let mip1_size = 64 * 64 * block_size; // 32768
        let mip1_offset = mip0_offset - mip1_size;
        let ref_mip1 = &ref_data[mip1_offset..mip1_offset + mip1_size];

        // Decode mip0 to 512x512 pixel buffer
        let mut src_pixels = vec![[0u8; 4]; 512 * 512];
        for by in 0..128 {
            for bx in 0..128 {
                let off = (by * 128 + bx) * block_size;
                let block = &ref_mip0[off..off + block_size];
                let pixels = decode_dxt1_block(block);
                for py in 0..4 {
                    for px in 0..4 {
                        src_pixels[(by * 4 + py) * 512 + bx * 4 + px] = pixels[py * 4 + px];
                    }
                }
            }
        }

        // Box-filter to 256x256 (integer averaging)
        let mut dst_pixels = vec![[0u8; 4]; 256 * 256];
        for y in 0..256 {
            for x in 0..256 {
                let p00 = src_pixels[(y * 2) * 512 + x * 2];
                let p10 = src_pixels[(y * 2) * 512 + x * 2 + 1];
                let p01 = src_pixels[(y * 2 + 1) * 512 + x * 2];
                let p11 = src_pixels[(y * 2 + 1) * 512 + x * 2 + 1];
                for c in 0..4 {
                    let sum = p00[c] as u32 + p10[c] as u32 + p01[c] as u32 + p11[c] as u32;
                    dst_pixels[y * 256 + x][c] = ((sum + 2) / 4) as u8;
                }
            }
        }

        // Compare against reference solid blocks
        let mip1_bw = 64; // blocks wide
        let mut total_solid = 0;
        let mut exact_match = 0;
        let mut off_by_one = 0;  // all channels differ by <= 1
        let mut off_by_more = 0;
        let mut total_channel_diff = 0u64;
        let mut max_diff = 0i32;

        for b in 0..(64 * 64) {
            let block = &ref_mip1[b * block_size..(b + 1) * block_size];
            let c0 = u16::from_le_bytes([block[0], block[1]]);
            let c1 = u16::from_le_bytes([block[2], block[3]]);
            if c0 != c1 { continue; }  // skip non-solid

            total_solid += 1;
            let (rr, rg, rb) = decode_rgb565(c0);

            // Get our pixel at the top-left of this block
            let bx = b % mip1_bw;
            let by = b / mip1_bw;
            let our = dst_pixels[by * 4 * 256 + bx * 4];

            let dr = (our[0] as i32 - rr as i32).abs();
            let dg = (our[1] as i32 - rg as i32).abs();
            let db = (our[2] as i32 - rb as i32).abs();
            let block_max = dr.max(dg).max(db);
            total_channel_diff += (dr + dg + db) as u64;
            max_diff = max_diff.max(block_max);

            if dr == 0 && dg == 0 && db == 0 {
                exact_match += 1;
            } else if block_max <= 1 {
                off_by_one += 1;
            } else {
                off_by_more += 1;
                if off_by_more <= 5 {
                    eprintln!("  Block {} ({},{}): ours=({},{},{}) ref_decoded=({},{},{}) diff=({},{},{})",
                        b, bx, by, our[0], our[1], our[2], rr, rg, rb, dr, dg, db);
                }
            }
        }

        eprintln!("\n  Solid block pixel comparison (texture 19767):");
        eprintln!("    Total solid blocks: {}", total_solid);
        eprintln!("    Exact pixel match: {} ({:.1}%)", exact_match, exact_match as f64 / total_solid as f64 * 100.0);
        eprintln!("    Off by 1: {} ({:.1}%)", off_by_one, off_by_one as f64 / total_solid as f64 * 100.0);
        eprintln!("    Off by >1: {} ({:.1}%)", off_by_more, off_by_more as f64 / total_solid as f64 * 100.0);
        eprintln!("    Max per-channel diff: {}", max_diff);
        eprintln!("    Avg total channel diff: {:.2}", total_channel_diff as f64 / total_solid as f64);
    }

    #[test]
    #[ignore]
    fn encoder_replication_determine_decode() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let mip0_size = 128 * 128 * 8;
        let mip0_offset = ref_data.len() - mip0_size;
        let ref_mip0 = &ref_data[mip0_offset..];
        let mip1_size = 64 * 64 * 8;
        let mip1_offset = mip0_offset - mip1_size;
        let ref_mip1 = &ref_data[mip1_offset..mip1_offset + mip1_size];

        // Define decode variants
        fn expand5_mul(v: u8) -> u8 { (v as u32 * 255 / 31) as u8 }
        fn expand6_mul(v: u8) -> u8 { (v as u32 * 255 / 63) as u8 }
        fn expand5_shift(v: u8) -> u8 { (v << 3) | (v >> 2) }
        fn expand6_shift(v: u8) -> u8 { (v << 2) | (v >> 4) }

        fn interp_plus1(a: u8, b: u8) -> u8 { ((2 * a as u32 + b as u32 + 1) / 3) as u8 }
        fn interp_trunc(a: u8, b: u8) -> u8 { ((2 * a as u32 + b as u32) / 3) as u8 }

        // Test all 4 combinations: (mul vs shift) x (plus1 vs trunc)
        let configs: &[(&str, fn(u8)->u8, fn(u8)->u8, fn(u8,u8)->u8)] = &[
            ("mul+plus1",   expand5_mul,   expand6_mul,   interp_plus1),
            ("mul+trunc",   expand5_mul,   expand6_mul,   interp_trunc),
            ("shift+plus1", expand5_shift, expand6_shift, interp_plus1),
            ("shift+trunc", expand5_shift, expand6_shift, interp_trunc),
        ];

        for &(name, exp5, exp6, interp) in configs {
            // Decode mip0 with this config
            let mut pixels = vec![[0u8; 4]; 512 * 512];
            for by in 0..128 {
                for bx in 0..128 {
                    let off = (by * 128 + bx) * 8;
                    let block = &ref_mip0[off..off+8];
                    let c0 = u16::from_le_bytes([block[0], block[1]]);
                    let c1 = u16::from_le_bytes([block[2], block[3]]);
                    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);

                    let r0 = exp5(((c0 >> 11) & 0x1F) as u8);
                    let g0 = exp6(((c0 >> 5) & 0x3F) as u8);
                    let b0 = exp5((c0 & 0x1F) as u8);
                    let r1 = exp5(((c1 >> 11) & 0x1F) as u8);
                    let g1 = exp6(((c1 >> 5) & 0x3F) as u8);
                    let b1 = exp5((c1 & 0x1F) as u8);

                    let palette: [[u8; 4]; 4] = if c0 > c1 {
                        [
                            [r0, g0, b0, 255],
                            [r1, g1, b1, 255],
                            [interp(r0, r1), interp(g0, g1), interp(b0, b1), 255],
                            [interp(r1, r0), interp(g1, g0), interp(b1, b0), 255],
                        ]
                    } else {
                        [
                            [r0, g0, b0, 255],
                            [r1, g1, b1, 255],
                            [((r0 as u16 + r1 as u16) / 2) as u8, ((g0 as u16 + g1 as u16) / 2) as u8, ((b0 as u16 + b1 as u16) / 2) as u8, 255],
                            [0, 0, 0, 0],
                        ]
                    };

                    for i in 0..16 {
                        let sel = ((indices >> (i * 2)) & 3) as usize;
                        let px = bx * 4 + (i % 4);
                        let py = by * 4 + (i / 4);
                        pixels[py * 512 + px] = palette[sel];
                    }
                }
            }

            // Box-filter to 256x256 (truncating)
            let mut filtered = vec![[0u8; 4]; 256 * 256];
            for y in 0..256 {
                for x in 0..256 {
                    let p00 = pixels[(y*2)*512 + x*2];
                    let p10 = pixels[(y*2)*512 + x*2+1];
                    let p01 = pixels[(y*2+1)*512 + x*2];
                    let p11 = pixels[(y*2+1)*512 + x*2+1];
                    for c in 0..4 {
                        filtered[y*256+x][c] = ((p00[c] as u32 + p10[c] as u32 + p01[c] as u32 + p11[c] as u32) / 4) as u8;
                    }
                }
            }

            // For each SOLID reference mip1 block (c0==c1), check if our filtered pixel
            // encodes to the same RGB565 value
            let mut total = 0;
            let mut matching = 0;
            for b in 0..(64*64) {
                let block = &ref_mip1[b*8..(b+1)*8];
                let rc0 = u16::from_le_bytes([block[0], block[1]]);
                let rc1 = u16::from_le_bytes([block[2], block[3]]);
                if rc0 != rc1 { continue; }

                total += 1;
                let bx = b % 64;
                let by = b / 64;
                let p = filtered[by * 4 * 256 + bx * 4]; // top-left pixel of block

                // Encode our pixel to RGB565 (rounding)
                let our_r5 = ((p[0] as u16 * 31 + 127) / 255) as u16;
                let our_g6 = ((p[1] as u16 * 63 + 127) / 255) as u16;
                let our_b5 = ((p[2] as u16 * 31 + 127) / 255) as u16;
                let our_c = (our_r5 << 11) | (our_g6 << 5) | our_b5;

                if our_c == rc0 { matching += 1; }
            }
            eprintln!("  {}: {}/{} solid blocks match ({:.1}%)", name, matching, total, matching as f64 / total as f64 * 100.0);
        }

        // Also test with rounding box filter for comparison
        eprintln!("\n  With ROUNDING box filter ((sum+2)/4):");
        for &(name, exp5, exp6, interp) in configs {
            let mut pixels = vec![[0u8; 4]; 512 * 512];
            for by in 0..128 {
                for bx in 0..128 {
                    let off = (by * 128 + bx) * 8;
                    let block = &ref_mip0[off..off+8];
                    let c0 = u16::from_le_bytes([block[0], block[1]]);
                    let c1 = u16::from_le_bytes([block[2], block[3]]);
                    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
                    let r0 = exp5(((c0 >> 11) & 0x1F) as u8);
                    let g0 = exp6(((c0 >> 5) & 0x3F) as u8);
                    let b0 = exp5((c0 & 0x1F) as u8);
                    let r1 = exp5(((c1 >> 11) & 0x1F) as u8);
                    let g1 = exp6(((c1 >> 5) & 0x3F) as u8);
                    let b1 = exp5((c1 & 0x1F) as u8);
                    let palette: [[u8; 4]; 4] = if c0 > c1 {
                        [[r0,g0,b0,255],[r1,g1,b1,255],
                         [interp(r0,r1),interp(g0,g1),interp(b0,b1),255],
                         [interp(r1,r0),interp(g1,g0),interp(b1,b0),255]]
                    } else {
                        [[r0,g0,b0,255],[r1,g1,b1,255],
                         [((r0 as u16+r1 as u16)/2) as u8,((g0 as u16+g1 as u16)/2) as u8,((b0 as u16+b1 as u16)/2) as u8,255],
                         [0,0,0,0]]
                    };
                    for i in 0..16 {
                        let sel = ((indices >> (i*2)) & 3) as usize;
                        pixels[(by*4+i/4)*512 + bx*4+i%4] = palette[sel];
                    }
                }
            }
            let mut filtered = vec![[0u8; 4]; 256*256];
            for y in 0..256 { for x in 0..256 {
                let p00 = pixels[(y*2)*512+x*2];
                let p10 = pixels[(y*2)*512+x*2+1];
                let p01 = pixels[(y*2+1)*512+x*2];
                let p11 = pixels[(y*2+1)*512+x*2+1];
                for c in 0..4 {
                    filtered[y*256+x][c] = ((p00[c] as u32+p10[c] as u32+p01[c] as u32+p11[c] as u32+2)/4) as u8;
                }
            }}
            let mut total = 0; let mut matching = 0;
            for b in 0..(64*64) {
                let block = &ref_mip1[b*8..(b+1)*8];
                let rc0 = u16::from_le_bytes([block[0],block[1]]);
                let rc1 = u16::from_le_bytes([block[2],block[3]]);
                if rc0 != rc1 { continue; }
                total += 1;
                let bx = b%64; let by = b/64;
                let p = filtered[by*4*256+bx*4];
                let our_r5 = ((p[0] as u16*31+127)/255) as u16;
                let our_g6 = ((p[1] as u16*63+127)/255) as u16;
                let our_b5 = ((p[2] as u16*31+127)/255) as u16;
                let our_c = (our_r5<<11)|(our_g6<<5)|our_b5;
                if our_c == rc0 { matching += 1; }
            }
            eprintln!("  {}: {}/{} solid blocks match ({:.1}%)", name, matching, total, matching as f64/total as f64*100.0);
        }
    }

    #[test]
    #[ignore]
    fn encoder_replication_pixel_nonsolid() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let mip0_size = 128 * 128 * 8;
        let mip0_offset = ref_data.len() - mip0_size;
        let ref_mip0 = &ref_data[mip0_offset..];
        let mip1_size = 64 * 64 * 8;
        let mip1_offset = mip0_offset - mip1_size;
        let ref_mip1 = &ref_data[mip1_offset..mip1_offset + mip1_size];

        // Decode mip0 to pixels
        let mut src_pixels = vec![[0u8; 4]; 512 * 512];
        for by in 0..128 {
            for bx in 0..128 {
                let off = (by * 128 + bx) * 8;
                let block = &ref_mip0[off..off + 8];
                let pixels = decode_dxt1_block(block);
                for py in 0..4 {
                    for px in 0..4 {
                        src_pixels[(by * 4 + py) * 512 + bx * 4 + px] = pixels[py * 4 + px];
                    }
                }
            }
        }

        // Box-filter to 256x256 (truncating)
        let mut dst_pixels = vec![[0u8; 4]; 256 * 256];
        for y in 0..256 {
            for x in 0..256 {
                let p00 = src_pixels[(y * 2) * 512 + x * 2];
                let p10 = src_pixels[(y * 2) * 512 + x * 2 + 1];
                let p01 = src_pixels[(y * 2 + 1) * 512 + x * 2];
                let p11 = src_pixels[(y * 2 + 1) * 512 + x * 2 + 1];
                for c in 0..4 {
                    dst_pixels[y * 256 + x][c] = ((p00[c] as u32 + p10[c] as u32 + p01[c] as u32 + p11[c] as u32) / 4) as u8;
                }
            }
        }

        // For non-solid blocks: compare ALL 16 pixel positions
        let mut total_pixels = 0u64;
        let mut exact_match = 0u64;
        let mut off_by_1 = 0u64;
        let mut off_by_2plus = 0u64;
        let mut channel_diffs = [0i64; 3]; // signed sum of (ours - ref_decoded) per channel
        let mut abs_diffs = [0u64; 3];
        let mut max_diff = 0i32;
        let mut blocks_checked = 0u32;
        let mut sample_printed = 0;

        for b in 0..(64 * 64) {
            let block = &ref_mip1[b * 8..(b + 1) * 8];
            let c0 = u16::from_le_bytes([block[0], block[1]]);
            let c1 = u16::from_le_bytes([block[2], block[3]]);
            if c0 == c1 { continue; } // skip solid

            blocks_checked += 1;
            let ref_decoded = decode_dxt1_block(block);
            let bx = b % 64;
            let by = b / 64;

            for py in 0..4 {
                for px in 0..4 {
                    let our_pixel = dst_pixels[(by * 4 + py) * 256 + bx * 4 + px];
                    let ref_pixel = ref_decoded[py * 4 + px];
                    total_pixels += 1;

                    for c in 0..3 {
                        let diff = our_pixel[c] as i32 - ref_pixel[c] as i32;
                        channel_diffs[c] += diff as i64;
                        abs_diffs[c] += diff.unsigned_abs() as u64;
                        max_diff = max_diff.max(diff.abs());
                    }

                    let pixel_max = (0..3).map(|c| (our_pixel[c] as i32 - ref_pixel[c] as i32).abs()).max().unwrap();
                    if pixel_max == 0 { exact_match += 1; }
                    else if pixel_max == 1 { off_by_1 += 1; }
                    else { off_by_2plus += 1; }
                }
            }

            // Print first 3 non-solid blocks in detail
            if sample_printed < 3 {
                sample_printed += 1;
                eprintln!("\n  Non-solid block {} ({},{}):", b, bx, by);
                eprintln!("    Ref block: {:02x?}", block);
                for row in 0..4 {
                    let mut our_row = String::new();
                    let mut ref_row = String::new();
                    for col in 0..4 {
                        let op = dst_pixels[(by*4+row)*256 + bx*4+col];
                        let rp = ref_decoded[row*4+col];
                        our_row += &format!("({:3},{:3},{:3}) ", op[0], op[1], op[2]);
                        ref_row += &format!("({:3},{:3},{:3}) ", rp[0], rp[1], rp[2]);
                    }
                    eprintln!("    Our row{}: {}", row, our_row);
                    eprintln!("    Ref row{}: {}", row, ref_row);
                }
            }
        }

        eprintln!("\n  Non-solid pixel comparison (texture 19767):");
        eprintln!("    Blocks checked: {}", blocks_checked);
        eprintln!("    Total pixels: {}", total_pixels);
        eprintln!("    Exact match: {} ({:.1}%)", exact_match, exact_match as f64 / total_pixels as f64 * 100.0);
        eprintln!("    Off by 1: {} ({:.1}%)", off_by_1, off_by_1 as f64 / total_pixels as f64 * 100.0);
        eprintln!("    Off by 2+: {} ({:.1}%)", off_by_2plus, off_by_2plus as f64 / total_pixels as f64 * 100.0);
        eprintln!("    Max per-channel diff: {}", max_diff);
        eprintln!("    Mean signed diff R/G/B: {:.3} / {:.3} / {:.3}",
            channel_diffs[0] as f64 / total_pixels as f64,
            channel_diffs[1] as f64 / total_pixels as f64,
            channel_diffs[2] as f64 / total_pixels as f64);
        eprintln!("    Mean abs diff R/G/B: {:.3} / {:.3} / {:.3}",
            abs_diffs[0] as f64 / total_pixels as f64,
            abs_diffs[1] as f64 / total_pixels as f64,
            abs_diffs[2] as f64 / total_pixels as f64);
    }

    #[test]
    #[ignore]
    fn encoder_replication_mip2_from_ref_mip1() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        // Test texture 19767 (512x512 DXT1)
        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let block_size = 8;
        // Mip layout (smallest-first, 40-byte header):
        // mip7(8) mip6(32) mip5(128) mip4(512) mip3(2048) mip2(8192) mip1(32768) mip0(131072)
        let mip0_offset = ref_data.len() - 131072;
        let mip1_offset = mip0_offset - 32768;
        let mip2_offset = mip1_offset - 8192;

        let ref_mip1 = &ref_data[mip1_offset..mip1_offset + 32768];
        let ref_mip2 = &ref_data[mip2_offset..mip2_offset + 8192];

        // Generate mip2 from REFERENCE mip1 using our pipeline
        let our_mip2 = generate_mip(ref_mip1, 256, 256, 128, 128, TextureCodec::Dxt1, false);
        assert_eq!(our_mip2.len(), 8192);

        // Compare block by block
        let num_blocks = 8192 / block_size;
        let mut matching = 0;
        let mut solid_match = 0;
        let mut solid_total = 0;
        let mut nonsolid_match = 0;
        let mut nonsolid_total = 0;

        for b in 0..num_blocks {
            let our_block = &our_mip2[b * block_size..(b + 1) * block_size];
            let ref_block = &ref_mip2[b * block_size..(b + 1) * block_size];

            let rc0 = u16::from_le_bytes([ref_block[0], ref_block[1]]);
            let rc1 = u16::from_le_bytes([ref_block[2], ref_block[3]]);
            let is_solid = rc0 == rc1;

            if our_block == ref_block {
                matching += 1;
                if is_solid { solid_match += 1; } else { nonsolid_match += 1; }
            }
            if is_solid { solid_total += 1; } else { nonsolid_total += 1; }
        }

        eprintln!("  Mip2 from REFERENCE mip1 (texture 19767):");
        eprintln!("    Total: {}/{} blocks match ({:.1}%)", matching, num_blocks,
            matching as f64 / num_blocks as f64 * 100.0);
        eprintln!("    Solid: {}/{} ({:.1}%)", solid_match, solid_total,
            if solid_total > 0 { solid_match as f64 / solid_total as f64 * 100.0 } else { 0.0 });
        eprintln!("    Non-solid: {}/{} ({:.1}%)", nonsolid_match, nonsolid_total,
            if nonsolid_total > 0 { nonsolid_match as f64 / nonsolid_total as f64 * 100.0 } else { 0.0 });

        // Also generate mip2 from OUR mip1 (generated from mip0) for comparison
        let ref_mip0 = &ref_data[mip0_offset..mip0_offset + 131072];
        let our_mip1 = generate_mip(ref_mip0, 512, 512, 256, 256, TextureCodec::Dxt1, false);
        let our_mip2_from_ours = generate_mip(&our_mip1, 256, 256, 128, 128, TextureCodec::Dxt1, false);

        let mut matching2 = 0;
        for b in 0..num_blocks {
            if our_mip2_from_ours[b*block_size..(b+1)*block_size] == ref_mip2[b*block_size..(b+1)*block_size] {
                matching2 += 1;
            }
        }
        eprintln!("    (Comparison: mip2 from OUR mip1: {}/{} ({:.1}%))",
            matching2, num_blocks, matching2 as f64 / num_blocks as f64 * 100.0);
    }

    #[test]
    #[ignore]
    fn encoder_replication_verify_interp() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let mip0_size = 128 * 128 * 8;
        let mip0_offset = ref_data.len() - mip0_size;
        let ref_mip0 = &ref_data[mip0_offset..];
        let mip1_size = 64 * 64 * 8;
        let mip1_offset = mip0_offset - mip1_size;
        let ref_mip1 = &ref_data[mip1_offset..mip1_offset + mip1_size];

        // For each output pixel in mip1, it comes from a 2x2 region of source pixels.
        // Each source pixel comes from a specific mip0 block at a specific index.
        // If all 4 source pixels have the same block (c0, c1) AND the same index,
        // the box-filtered result is exactly one palette color.

        // Scan for uniform-index 2x2 regions in mip0.
        // A mip1 pixel at (x,y) comes from mip0 pixels at (2x,2y), (2x+1,2y), (2x,2y+1), (2x+1,2y+1).
        // These 4 source pixels may span up to 4 different mip0 blocks.
        // For simplicity, check within a single mip0 block: 4x4 block contains 2x2 output pixels.
        // An output pixel at block-relative (ox,oy) uses source (2ox, 2oy)...(2ox+1, 2oy+1).

        let mut counts_by_index = [0u32; 4]; // how many pixels verified per index
        let mut matches_by_index = [0u32; 4];

        // Check output pixels that are entirely within one mip0 block
        // (i.e., all 4 source pixels come from the same block)
        for mip0_by in 0..128 {
            for mip0_bx in 0..128 {
                let blk_off = (mip0_by * 128 + mip0_bx) * 8;
                let block = &ref_mip0[blk_off..blk_off + 8];
                let c0 = u16::from_le_bytes([block[0], block[1]]);
                let c1 = u16::from_le_bytes([block[2], block[3]]);
                let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);

                if c0 <= c1 { continue; } // skip 3-color mode blocks

                // Each mip0 4x4 block contains 2x2 groups of 2x2 source pixels
                // Output pixel at (ox, oy) within this block uses source pixels:
                // (2*ox, 2*oy), (2*ox+1, 2*oy), (2*ox, 2*oy+1), (2*ox+1, 2*oy+1)
                for oy in 0..2 {
                    for ox in 0..2 {
                        let src_indices = [
                            ((indices >> (((oy*2)*4 + ox*2) * 2)) & 3) as usize,
                            ((indices >> (((oy*2)*4 + ox*2+1) * 2)) & 3) as usize,
                            ((indices >> (((oy*2+1)*4 + ox*2) * 2)) & 3) as usize,
                            ((indices >> (((oy*2+1)*4 + ox*2+1) * 2)) & 3) as usize,
                        ];

                        // Check if all 4 use the same index
                        if src_indices[0] != src_indices[1] || src_indices[0] != src_indices[2] || src_indices[0] != src_indices[3] {
                            continue;
                        }
                        let idx = src_indices[0];

                        // Decode the palette color for this index
                        let decoded = decode_dxt1_block(block);
                        let pixel = decoded[(oy * 2) * 4 + ox * 2]; // any of the 4 source pixels (all same)

                        // This pixel, after box filter (identity since all 4 are same), should encode
                        // to a specific RGB565 value. Check against reference mip1.
                        let our_r5 = ((pixel[0] as u16 * 31 + 127) / 255) as u16;
                        let our_g6 = ((pixel[1] as u16 * 63 + 127) / 255) as u16;
                        let our_b5 = ((pixel[2] as u16 * 31 + 127) / 255) as u16;
                        let our_c = (our_r5 << 11) | (our_g6 << 5) | our_b5;

                        // Find the corresponding mip1 block and check if it's solid (c0==c1)
                        let mip1_x = mip0_bx * 2 + ox; // pixel position in mip1
                        let mip1_y = mip0_by * 2 + oy;
                        let mip1_bx = mip1_x / 4; // mip1 block position
                        let mip1_by = mip1_y / 4;
                        let mip1_blk = &ref_mip1[(mip1_by * 64 + mip1_bx) * 8..];
                        let ref_c0 = u16::from_le_bytes([mip1_blk[0], mip1_blk[1]]);
                        let ref_c1 = u16::from_le_bytes([mip1_blk[2], mip1_blk[3]]);

                        // Only check if the mip1 block is solid (c0==c1)
                        if ref_c0 != ref_c1 { continue; }

                        counts_by_index[idx] += 1;
                        if our_c == ref_c0 {
                            matches_by_index[idx] += 1;
                        } else if counts_by_index[idx] <= 3 {
                            let (rr, rg, rb) = decode_rgb565(ref_c0);
                            eprintln!("  Idx {} mismatch: pixel=({},{},{}) our_c={:04x} ref_c={:04x} ref_decoded=({},{},{})",
                                idx, pixel[0], pixel[1], pixel[2], our_c, ref_c0, rr, rg, rb);
                        }
                    }
                }
            }
        }

        eprintln!("\n  Interpolated color verification (texture 19767):");
        for idx in 0..4 {
            if counts_by_index[idx] > 0 {
                eprintln!("    Index {}: {}/{} match ({:.1}%)", idx,
                    matches_by_index[idx], counts_by_index[idx],
                    matches_by_index[idx] as f64 / counts_by_index[idx] as f64 * 100.0);
            } else {
                eprintln!("    Index {}: no samples", idx);
            }
        }
    }

    #[test]
    #[ignore]
    fn encoder_replication_trace_block() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let mip1_offset = ref_data.len() - 131072 - 32768;
        let ref_mip1 = &ref_data[mip1_offset..mip1_offset + 32768];
        let mip2_offset = mip1_offset - 8192;
        let ref_mip2 = &ref_data[mip2_offset..mip2_offset + 8192];

        // Decode ref mip1 to pixel buffer (256x256)
        let mut mip1_pixels = vec![[0u8; 4]; 256 * 256];
        for by in 0..64 {
            for bx in 0..64 {
                let off = (by * 64 + bx) * 8;
                let block = &ref_mip1[off..off + 8];
                let pixels = decode_dxt1_block(block);
                for py in 0..4 { for px in 0..4 {
                    mip1_pixels[(by*4+py)*256 + bx*4+px] = pixels[py*4+px];
                }}
            }
        }

        // Box-filter to 128x128
        let mut mip2_pixels = vec![[0u8; 4]; 128 * 128];
        for y in 0..128 { for x in 0..128 {
            let p00 = mip1_pixels[(y*2)*256 + x*2];
            let p10 = mip1_pixels[(y*2)*256 + x*2+1];
            let p01 = mip1_pixels[(y*2+1)*256 + x*2];
            let p11 = mip1_pixels[(y*2+1)*256 + x*2+1];
            for c in 0..4 {
                mip2_pixels[y*128+x][c] = ((p00[c] as u32 + p10[c] as u32 + p01[c] as u32 + p11[c] as u32) / 4) as u8;
            }
        }}

        // Find a non-solid block in ref_mip2
        let target_block = {
            let mut found = None;
            for b in 0..(32*32) {
                let blk = &ref_mip2[b*8..(b+1)*8];
                let c0 = u16::from_le_bytes([blk[0], blk[1]]);
                let c1 = u16::from_le_bytes([blk[2], blk[3]]);
                if c0 != c1 { found = Some(b); break; }
            }
            found.unwrap()
        };

        let bx = target_block % 32;
        let by = target_block / 32;
        eprintln!("  Target block {} ({},{}) in mip2:", target_block, bx, by);
        eprintln!("  Ref mip2 block: {:02x?}", &ref_mip2[target_block*8..(target_block+1)*8]);

        // Extract 4x4 pixels for this block
        let mut pixels = [[0u8; 4]; 16];
        for py in 0..4 { for px in 0..4 {
            pixels[py*4+px] = mip2_pixels[(by*4+py)*128 + bx*4+px];
        }}
        eprintln!("  Input pixels:");
        for row in 0..4 {
            eprintln!("    row{}: ({},{},{}) ({},{},{}) ({},{},{}) ({},{},{})", row,
                pixels[row*4][0], pixels[row*4][1], pixels[row*4][2],
                pixels[row*4+1][0], pixels[row*4+1][1], pixels[row*4+1][2],
                pixels[row*4+2][0], pixels[row*4+2][1], pixels[row*4+2][2],
                pixels[row*4+3][0], pixels[row*4+3][1], pixels[row*4+3][2]);
        }

        // Count unique colors
        let mut unique: Vec<[u8;3]> = Vec::new();
        for p in &pixels {
            let rgb = [p[0], p[1], p[2]];
            if !unique.contains(&rgb) { unique.push(rgb); }
        }
        eprintln!("  Unique colors: {}", unique.len());

        // Run through our encoder
        let our_block = encode_dxt1_block(&pixels, false, true);
        eprintln!("  Our encoded: {:02x?}", our_block);
        eprintln!("  Match: {}", our_block == ref_mip2[target_block*8..(target_block+1)*8]);

        // Also encode with dedup=true for comparison
        let our_block_dedup = encode_dxt1_block(&pixels, false, false);
        eprintln!("  Our (with dedup): {:02x?}", our_block_dedup);
        eprintln!("  Match (dedup): {}", our_block_dedup == ref_mip2[target_block*8..(target_block+1)*8]);

        // Decode both to see visual difference
        let ref_decoded = decode_dxt1_block(&ref_mip2[target_block*8..(target_block+1)*8]);
        let our_decoded = decode_dxt1_block(&our_block);
        let mut max_diff = 0;
        for i in 0..16 { for c in 0..3 {
            max_diff = max_diff.max((ref_decoded[i][c] as i32 - our_decoded[i][c] as i32).abs());
        }}
        eprintln!("  Max decoded pixel diff: {}", max_diff);

        // Detailed trace of encoder internals
        let (colors, weights, _order, n) = build_color_set(&pixels, false);
        eprintln!("\n  build_color_set output:");
        eprintln!("    n = {}", n);

        // Show PCA axis (recompute)
        let total_weight: f32 = weights.iter().sum();
        let mut centroid = [0.0f32; 3];
        for (c, &w) in colors.iter().zip(weights.iter()) {
            for k in 0..3 { centroid[k] += c[k] * w; }
        }
        for k in 0..3 { centroid[k] /= total_weight; }
        eprintln!("    centroid: ({:.6}, {:.6}, {:.6})", centroid[0], centroid[1], centroid[2]);

        // Show first few sorted colors
        for i in 0..n.min(4) {
            eprintln!("    sorted[{}]: ({:.6}, {:.6}, {:.6}) w={:.4}", i,
                colors[i][0], colors[i][1], colors[i][2], weights[i]);
        }
        if n > 4 { eprintln!("    ... ({} more)", n - 4); }

        // Run partition search to find best (s,t,u) and endpoints
        let mut wt_colors = vec![[0.0f32; 3]; n];
        for i in 0..n { for k in 0..3 { wt_colors[i][k] = colors[i][k] * weights[i]; } }
        let mut total_rgb = [0.0f32; 3];
        for i in 0..n { for k in 0..3 { total_rgb[k] += wt_colors[i][k]; } }

        let mut best_err = f32::MAX;
        let mut best_s = 0; let mut best_t = 0; let mut best_u = 0;
        let mut best_ep_a = [0.0f32; 3]; let mut best_ep_b = [0.0f32; 3];
        let mut best_ep_a_raw = [0.0f32; 3]; let mut best_ep_b_raw = [0.0f32; 3];

        let mut outer_rgb = [0.0f32; 3]; let mut outer_w = 0.0f32;
        for s in 0..=n {
            let mut mid_rgb = [0.0f32; 3]; let mut mid_w = 0.0f32;
            for t in s..=n {
                let mut inner_rgb = [0.0f32; 3]; let mut inner_w = 0.0f32;
                for u in t..=n {
                    let c49: f32 = 4.0/9.0; let c19: f32 = 1.0/9.0; let c29: f32 = 2.0/9.0;
                    let c23: f32 = 2.0/3.0; let c13: f32 = 1.0/3.0;
                    let aa = inner_w * c19 + mid_w * c49 + outer_w;
                    let bb = inner_w * c49 + (total_weight - outer_w - mid_w - inner_w) + mid_w * c19;
                    let ab = (inner_w + mid_w) * c29;
                    let det = aa * bb - ab * ab;
                    if det.abs() >= f32::MIN_POSITIVE {
                        let inv = 1.0f32 / det;
                        let mut ep_a = [0.0f32; 3]; let mut ep_b = [0.0f32; 3];
                        let mut err = 0.0f32;
                        for k in 0..3 {
                            let ba = mid_rgb[k] * c23 + outer_rgb[k] + inner_rgb[k] * c13;
                            let bb_val = total_rgb[k] - ba;
                            let a_k = (ba * bb - bb_val * ab) * inv;
                            let b_k = (bb_val * aa - ba * ab) * inv;
                            ep_a[k] = a_k.clamp(0.0, 1.0);
                            ep_b[k] = b_k.clamp(0.0, 1.0);
                        }
                        let raw_a = ep_a; let raw_b = ep_b;
                        // grid-snap at f64
                        let grids = [31.0f64, 63.0, 31.0];
                        let inv_grids = [1.0f64/31.0, 1.0/63.0, 1.0/31.0];
                        for k in 0..3 {
                            let qa = (ep_a[k] as f64 * grids[k] + 0.5).floor();
                            ep_a[k] = (qa * inv_grids[k]) as f32;
                            let qb = (ep_b[k] as f64 * grids[k] + 0.5).floor();
                            ep_b[k] = (qb * inv_grids[k]) as f32;
                        }
                        for k in 0..3 {
                            let ba = mid_rgb[k] * c23 + outer_rgb[k] + inner_rgb[k] * c13;
                            let bb_val = total_rgb[k] - ba;
                            err += aa * ep_a[k]*ep_a[k] + bb * ep_b[k]*ep_b[k]
                                + 2.0 * ab * ep_a[k]*ep_b[k]
                                - 2.0 * (ba * ep_a[k] + bb_val * ep_b[k]);
                        }
                        if err < best_err {
                            best_err = err; best_s = s; best_t = t; best_u = u;
                            best_ep_a = ep_a; best_ep_b = ep_b;
                            best_ep_a_raw = raw_a; best_ep_b_raw = raw_b;
                        }
                    }
                    if u < n { for k in 0..3 { inner_rgb[k] += wt_colors[u][k]; } inner_w += weights[u]; }
                }
                if t < n { for k in 0..3 { mid_rgb[k] += wt_colors[t][k]; } mid_w += weights[t]; }
            }
            if s < n { for k in 0..3 { outer_rgb[k] += wt_colors[s][k]; } outer_w += weights[s]; }
        }

        eprintln!("\n  Best partition: s={}, t={}, u={}", best_s, best_t, best_u);
        eprintln!("    Raw ep_a: ({:.8}, {:.8}, {:.8})", best_ep_a_raw[0], best_ep_a_raw[1], best_ep_a_raw[2]);
        eprintln!("    Raw ep_b: ({:.8}, {:.8}, {:.8})", best_ep_b_raw[0], best_ep_b_raw[1], best_ep_b_raw[2]);

        // Quantize
        for k in 0..3 {
            let grid = [31.0f64, 63.0, 31.0][k];
            let qa = (best_ep_a_raw[k] as f64 * grid + 0.5).floor();
            let qb = (best_ep_b_raw[k] as f64 * grid + 0.5).floor();
            eprintln!("    Channel {}: ep_a raw={:.8} quant={} | ep_b raw={:.8} quant={}",
                k, best_ep_a_raw[k], qa, best_ep_b_raw[k], qb);
        }

        eprintln!("    Snapped ep_a: ({:.8}, {:.8}, {:.8})", best_ep_a[0], best_ep_a[1], best_ep_a[2]);
        eprintln!("    Snapped ep_b: ({:.8}, {:.8}, {:.8})", best_ep_b[0], best_ep_b[1], best_ep_b[2]);
        eprintln!("    Best err: {:.10}", best_err);
    }

    #[test]
    #[ignore]
    fn encoder_replication_direct_mip() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let mip0_size = 128 * 128 * 8;
        let mip0_offset = ref_data.len() - mip0_size;
        let ref_mip0 = &ref_data[mip0_offset..];
        let mip2_size = 32 * 32 * 8;
        let mip2_offset = mip0_offset - (64*64*8) - mip2_size;
        let ref_mip2 = &ref_data[mip2_offset..mip2_offset + mip2_size];

        // Decode mip0 to 512x512 pixels
        let mut src = vec![[0u8; 4]; 512 * 512];
        for by in 0..128 {
            for bx in 0..128 {
                let off = (by * 128 + bx) * 8;
                let pixels = decode_dxt1_block(&ref_mip0[off..off+8]);
                for py in 0..4 { for px in 0..4 {
                    src[(by*4+py)*512 + bx*4+px] = pixels[py*4+px];
                }}
            }
        }

        // === Direct path: mip0 512x512 → 128x128 (4x downscale, 4x4 average) ===
        let mut direct_128 = vec![[0u8; 4]; 128 * 128];
        for y in 0..128 {
            for x in 0..128 {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for dy in 0..4 { for dx in 0..4 {
                        sum += src[(y*4+dy)*512 + x*4+dx][c] as u32;
                    }}
                    direct_128[y*128+x][c] = (sum / 16) as u8;
                }
            }
        }

        // Encode direct 128x128 to DXT1 and compare
        let mut direct_match = 0;
        let mut direct_solid_match = 0;
        let mut direct_solid_total = 0;
        let mut direct_nonsolid_match = 0;
        let mut direct_nonsolid_total = 0;
        for by in 0..32 {
            for bx in 0..32 {
                let mut block_pixels = [[0u8; 4]; 16];
                for py in 0..4 { for px in 0..4 {
                    block_pixels[py*4+px] = direct_128[(by*4+py)*128 + bx*4+px];
                }}
                let our = encode_dxt1_block(&block_pixels, false, true);
                let b = by * 32 + bx;
                let ref_block = &ref_mip2[b*8..(b+1)*8];
                let rc0 = u16::from_le_bytes([ref_block[0], ref_block[1]]);
                let rc1 = u16::from_le_bytes([ref_block[2], ref_block[3]]);
                let is_solid = rc0 == rc1;
                if our == ref_block {
                    direct_match += 1;
                    if is_solid { direct_solid_match += 1; } else { direct_nonsolid_match += 1; }
                }
                if is_solid { direct_solid_total += 1; } else { direct_nonsolid_total += 1; }
            }
        }

        // === Sequential path: mip0 → mip1 → mip2 ===
        let our_mip1 = generate_mip(ref_mip0, 512, 512, 256, 256, TextureCodec::Dxt1, false);
        let our_mip2 = generate_mip(&our_mip1, 256, 256, 128, 128, TextureCodec::Dxt1, false);
        let mut seq_match = 0;
        for b in 0..1024 {
            if our_mip2[b*8..(b+1)*8] == ref_mip2[b*8..(b+1)*8] { seq_match += 1; }
        }

        eprintln!("  Direct (mip0→mip2, 4x4 avg):");
        eprintln!("    Total: {}/1024 ({:.1}%)", direct_match, direct_match as f64 / 1024.0 * 100.0);
        eprintln!("    Solid: {}/{} ({:.1}%)", direct_solid_match, direct_solid_total,
            if direct_solid_total > 0 { direct_solid_match as f64 / direct_solid_total as f64 * 100.0 } else { 0.0 });
        eprintln!("    Non-solid: {}/{} ({:.1}%)", direct_nonsolid_match, direct_nonsolid_total,
            if direct_nonsolid_total > 0 { direct_nonsolid_match as f64 / direct_nonsolid_total as f64 * 100.0 } else { 0.0 });
        eprintln!("  Sequential (mip0→mip1→mip2):");
        eprintln!("    Total: {}/1024 ({:.1}%)", seq_match, seq_match as f64 / 1024.0 * 100.0);
    }

    #[test]
    #[ignore]
    fn encoder_replication_filter_search() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let mip0_size = 128 * 128 * 8;
        let mip0_offset = ref_data.len() - mip0_size;
        let ref_mip0 = &ref_data[mip0_offset..];
        let mip1_size = 64 * 64 * 8;
        let mip1_offset = mip0_offset - mip1_size;
        let ref_mip1 = &ref_data[mip1_offset..mip1_offset + mip1_size];

        // Decode mip0 to 512x512 pixels
        let mut src = vec![[0u8; 4]; 512 * 512];
        for by in 0..128 {
            for bx in 0..128 {
                let off = (by * 128 + bx) * 8;
                let pixels = decode_dxt1_block(&ref_mip0[off..off + 8]);
                for py in 0..4 {
                    for px in 0..4 {
                        src[(by * 4 + py) * 512 + bx * 4 + px] = pixels[py * 4 + px];
                    }
                }
            }
        }

        // Collect solid block reference pixels from mip1
        struct SolidRef {
            bx: usize,
            by: usize,
            r: u8, g: u8, b: u8,
        }
        let mut solid_refs = Vec::new();
        for b in 0..(64 * 64) {
            let block = &ref_mip1[b * 8..(b + 1) * 8];
            let c0 = u16::from_le_bytes([block[0], block[1]]);
            let c1 = u16::from_le_bytes([block[2], block[3]]);
            if c0 != c1 { continue; }
            let (r, g, b_val) = decode_rgb565(c0);
            solid_refs.push(SolidRef { bx: b % 64, by: b / 64, r, g, b: b_val });
        }
        eprintln!("  Solid blocks in ref mip1: {}", solid_refs.len());

        // Helper: apply a filter to produce 256x256 from 512x512, return match count
        let test_filter = |name: &str, filter: &dyn Fn(&Vec<[u8;4]>, usize, usize) -> [u8;4]| {
            // For solid blocks, all 16 pixels should be identical after filtering.
            // Check top-left pixel of each solid block.
            let mut exact = 0u32;
            let mut off1 = 0u32;
            let mut off2plus = 0u32;
            let mut max_diff = 0i32;
            let mut total_ch_diff = [0i64; 3];
            for sr in &solid_refs {
                let x = sr.bx * 4;
                let y = sr.by * 4;
                let p = filter(&src, x, y);
                let dr = (p[0] as i32 - sr.r as i32).abs();
                let dg = (p[1] as i32 - sr.g as i32).abs();
                let db = (p[2] as i32 - sr.b as i32).abs();
                total_ch_diff[0] += p[0] as i64 - sr.r as i64;
                total_ch_diff[1] += p[1] as i64 - sr.g as i64;
                total_ch_diff[2] += p[2] as i64 - sr.b as i64;
                let m = dr.max(dg).max(db);
                max_diff = max_diff.max(m);
                if m == 0 { exact += 1; }
                else if m == 1 { off1 += 1; }
                else { off2plus += 1; }
            }
            let n = solid_refs.len() as f64;
            eprintln!("  {}: exact={}/{} ({:.1}%) off1={} off2+={} max={} bias=({:.2},{:.2},{:.2})",
                name, exact, solid_refs.len(), exact as f64/n*100.0, off1, off2plus, max_diff,
                total_ch_diff[0] as f64/n, total_ch_diff[1] as f64/n, total_ch_diff[2] as f64/n);
            exact
        };

        // Note: src is stack-allocated slice, but we pass by ref
        // We need to work around the array size in closure signatures
        let sw = 512usize;

        // Filter 1: Box filter (truncating)
        test_filter("box_trunc", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let sum = src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32
                    + src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32;
                out[c] = (sum / 4) as u8;
            }
            out
        });

        // Filter 2: Box filter (rounding +2)
        test_filter("box_round", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let sum = src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32
                    + src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32;
                out[c] = ((sum + 2) / 4) as u8;
            }
            out
        });

        // Filter 3: Box filter (float, truncate to u8)
        test_filter("box_float_trunc", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let sum = src[sy*sw+sx][c] as f32 + src[sy*sw+sx+1][c] as f32
                    + src[(sy+1)*sw+sx][c] as f32 + src[(sy+1)*sw+sx+1][c] as f32;
                out[c] = (sum * 0.25) as u8;
            }
            out
        });

        // Filter 4: Box filter using pmulhuw-style fixed-point (>> 8 rounding)
        // pmulhuw(a, b) = (a * b) >> 16. If weight = 0x4000 (= 0.25 in Q16), then
        // result = (pixel * 0x4000) >> 16 = pixel >> 2
        // But with 4 pixels summed first... let's try the SSE2 approach:
        // sum all 4 as u16, then pmulhuw with 0x4000
        test_filter("box_pmulhuw", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let sum = src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32
                    + src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32;
                // pmulhuw: (sum * 0x4000) >> 16 = sum >> 2
                let result = ((sum as u64 * 0x4000) >> 16) as u8;
                out[c] = result;
            }
            out
        });

        // Filter 5: sRGB-aware (gamma correct): convert to linear, average, back to sRGB
        test_filter("srgb_linear", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            let to_linear = |v: u8| -> f32 { (v as f32 / 255.0).powf(2.2) };
            let to_srgb = |v: f32| -> u8 { (v.powf(1.0/2.2) * 255.0 + 0.5).clamp(0.0, 255.0) as u8 };
            for c in 0..3 {
                let sum = to_linear(src[sy*sw+sx][c]) + to_linear(src[sy*sw+sx+1][c])
                    + to_linear(src[(sy+1)*sw+sx][c]) + to_linear(src[(sy+1)*sw+sx+1][c]);
                out[c] = to_srgb(sum * 0.25);
            }
            out[3] = ((src[sy*sw+sx][3] as u32 + src[sy*sw+sx+1][3] as u32
                + src[(sy+1)*sw+sx][3] as u32 + src[(sy+1)*sw+sx+1][3] as u32) / 4) as u8;
            out
        });

        // Filter 6: Nearest neighbor (top-left pixel)
        test_filter("nearest_tl", &|src, x, y| {
            src[y*2*sw + x*2]
        });

        // Filter 7: Nearest neighbor (center = bottom-right of top-left)
        test_filter("nearest_br", &|src, x, y| {
            src[(y*2+1)*sw + x*2+1]
        });

        // Filter 8: Box filter with +1 bias (matches some HW implementations)
        test_filter("box_plus1", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let sum = src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32
                    + src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32;
                out[c] = ((sum + 1) / 4) as u8;
            }
            out
        });

        // Filter 9: Average as u16 fixed-point (multiply by 16384 = 0x4000, then >>16)
        // This simulates pmulhuw rounding behavior
        test_filter("avg_u16_fixed", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                // Each pixel is extended to u16 via punpcklbw (zero-extend)
                // Then we add bias of 1 (paddw with 0x0001)
                // Then sum pairs via pmulhuw blend
                let p00 = src[sy*sw+sx][c] as u16;
                let p10 = src[sy*sw+sx+1][c] as u16;
                let p01 = src[(sy+1)*sw+sx][c] as u16;
                let p11 = src[(sy+1)*sw+sx+1][c] as u16;
                // Horizontal blend: avg(p00, p10) via (p00 + p10 + 1) >> 1
                let h0 = (p00 + p10 + 1) >> 1;
                let h1 = (p01 + p11 + 1) >> 1;
                // Vertical blend: avg(h0, h1) via (h0 + h1 + 1) >> 1
                let v = (h0 + h1 + 1) >> 1;
                out[c] = v as u8;
            }
            out
        });

        // Filter 10: Separable H-then-V with truncation
        test_filter("sep_h_then_v_trunc", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let h0 = (src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32) / 2;
                let h1 = (src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32) / 2;
                out[c] = ((h0 + h1) / 2) as u8;
            }
            out
        });

        // Filter 11: Separable V-then-H with truncation
        test_filter("sep_v_then_h_trunc", &|src, x, y| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let v0 = (src[sy*sw+sx][c] as u32 + src[(sy+1)*sw+sx][c] as u32) / 2;
                let v1 = (src[sy*sw+sx+1][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32) / 2;
                out[c] = ((v0 + v1) / 2) as u8;
            }
            out
        });

        // Now test full pipeline match rates with top filter candidates
        eprintln!("\n  === Full pipeline match rates (filter -> encode -> compare) ===");

        // Helper to run full pipeline with a given filter function
        let test_pipeline = |name: &str, filter_fn: &dyn Fn(&Vec<[u8;4]>, usize, usize, usize, usize) -> [u8;4]| {
            let mut dst = vec![[0u8; 4]; 256 * 256];
            for y in 0..256usize {
                for x in 0..256usize {
                    dst[y * 256 + x] = filter_fn(&src, x, y, 512, 256);
                }
            }
            // Encode and compare
            let mut match_count = 0u32;
            let mut solid_match = 0u32;
            let mut solid_total = 0u32;
            let mut nonsolid_match = 0u32;
            let mut nonsolid_total = 0u32;
            for by in 0..64 {
                for bx in 0..64 {
                    let mut block_pixels = [[0u8; 4]; 16];
                    for py in 0..4 { for px in 0..4 {
                        let x = (bx * 4 + px).min(255);
                        let y = (by * 4 + py).min(255);
                        block_pixels[py * 4 + px] = dst[y * 256 + x];
                    }}
                    let our = encode_dxt1_block(&block_pixels, false, true);
                    let b = by * 64 + bx;
                    let ref_block = &ref_mip1[b*8..(b+1)*8];
                    let rc0 = u16::from_le_bytes([ref_block[0], ref_block[1]]);
                    let rc1 = u16::from_le_bytes([ref_block[2], ref_block[3]]);
                    let is_solid = rc0 == rc1;
                    if is_solid { solid_total += 1; } else { nonsolid_total += 1; }
                    if our == ref_block {
                        match_count += 1;
                        if is_solid { solid_match += 1; } else { nonsolid_match += 1; }
                    }
                }
            }
            let total = 64*64;
            eprintln!("  {}: {}/{} ({:.1}%) solid={}/{} nonsolid={}/{}",
                name, match_count, total, match_count as f64/total as f64*100.0,
                solid_match, solid_total, nonsolid_match, nonsolid_total);
        };

        test_pipeline("pipe_box_trunc", &|src, x, y, sw, _dw| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let sum = src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32
                    + src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32;
                out[c] = (sum / 4) as u8;
            }
            out
        });

        test_pipeline("pipe_box_round", &|src, x, y, sw, _dw| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let sum = src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32
                    + src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32;
                out[c] = ((sum + 2) / 4) as u8;
            }
            out
        });

        test_pipeline("pipe_srgb_linear", &|src, x, y, sw, _dw| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            let to_lin = |v: u8| -> f32 { (v as f32 / 255.0).powf(2.2) };
            let to_srgb = |v: f32| -> u8 { (v.powf(1.0/2.2) * 255.0 + 0.5).clamp(0.0, 255.0) as u8 };
            for c in 0..3 {
                let sum = to_lin(src[sy*sw+sx][c]) + to_lin(src[sy*sw+sx+1][c])
                    + to_lin(src[(sy+1)*sw+sx][c]) + to_lin(src[(sy+1)*sw+sx+1][c]);
                out[c] = to_srgb(sum * 0.25);
            }
            out[3] = ((src[sy*sw+sx][3] as u32 + src[sy*sw+sx+1][3] as u32
                + src[(sy+1)*sw+sx][3] as u32 + src[(sy+1)*sw+sx+1][3] as u32) / 4) as u8;
            out
        });

        // sRGB-linear with TRUNCATION (not +0.5 rounding) and f64 pow for x87 precision
        test_pipeline("pipe_srgb_trunc_f64", &|src, x, y, sw, _dw| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            let to_lin = |v: u8| -> f64 { (v as f64 / 255.0).powf(2.2) };
            let to_srgb = |v: f64| -> u8 { (v.powf(1.0/2.2) * 255.0).clamp(0.0, 255.0) as u8 };
            for c in 0..3 {
                let sum = to_lin(src[sy*sw+sx][c]) + to_lin(src[sy*sw+sx+1][c])
                    + to_lin(src[(sy+1)*sw+sx][c]) + to_lin(src[(sy+1)*sw+sx+1][c]);
                out[c] = to_srgb(sum * 0.25);
            }
            out[3] = ((src[sy*sw+sx][3] as u32 + src[sy*sw+sx+1][3] as u32
                + src[(sy+1)*sw+sx][3] as u32 + src[(sy+1)*sw+sx+1][3] as u32) / 4) as u8;
            out
        });

        // sRGB-linear with f32 values converted to f64 for pow (matching x87 precision better)
        test_pipeline("pipe_srgb_f32_trunc", &|src, x, y, sw, _dw| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            let recip255 = 1.0f32 / 255.0;
            for c in 0..3 {
                // Match original: val * (1/255f) as f32, then pow as f64, store back as f32
                let f0 = (src[sy*sw+sx][c] as f32 * recip255) as f64;
                let f1 = (src[sy*sw+sx+1][c] as f32 * recip255) as f64;
                let f2 = (src[(sy+1)*sw+sx][c] as f32 * recip255) as f64;
                let f3 = (src[(sy+1)*sw+sx+1][c] as f32 * recip255) as f64;
                let l0 = f0.powf(2.2) as f32;
                let l1 = f1.powf(2.2) as f32;
                let l2 = f2.powf(2.2) as f32;
                let l3 = f3.powf(2.2) as f32;
                let avg = (l0 + l1 + l2 + l3) as f32 * 0.25f32;
                // De-linearize: pow(f32_avg, 1/2.2) with f64 precision, truncate to f32, then *255
                let srgb = (avg as f64).powf(1.0/2.2) as f32;
                out[c] = (srgb * 255.0f32).max(0.0).min(255.0) as u8;
            }
            out[3] = ((src[sy*sw+sx][3] as u32 + src[sy*sw+sx+1][3] as u32
                + src[(sy+1)*sw+sx][3] as u32 + src[(sy+1)*sw+sx+1][3] as u32) / 4) as u8;
            out
        });

        // Gamma-correct with x87 pow (matching original's FUN_682EE0 precision)
        test_pipeline("pipe_gamma_x87", &|src, x, y, sw, _dw| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            let recip255: f32 = 1.0 / 255.0;
            let gamma: f32 = 2.2;
            let inv_gamma: f32 = 1.0 / gamma; // computed at f32 precision
            for c in 0..3 {
                // Linearize each pixel with x87 pow
                let f0 = src[sy*sw+sx][c] as f32 * recip255;
                let f1 = src[sy*sw+sx+1][c] as f32 * recip255;
                let f2 = src[(sy+1)*sw+sx][c] as f32 * recip255;
                let f3 = src[(sy+1)*sw+sx+1][c] as f32 * recip255;
                let l0 = x87_powf(f0, gamma);
                let l1 = x87_powf(f1, gamma);
                let l2 = x87_powf(f2, gamma);
                let l3 = x87_powf(f3, gamma);
                // Box filter in linear space
                let avg = (l0 + l1 + l2 + l3) * 0.25;
                // De-linearize with x87 pow
                let srgb = x87_powf(avg, inv_gamma);
                out[c] = (srgb * 255.0).max(0.0).min(255.0) as u8;
            }
            out[3] = ((src[sy*sw+sx][3] as u32 + src[sy*sw+sx+1][3] as u32
                + src[(sy+1)*sw+sx][3] as u32 + src[(sy+1)*sw+sx+1][3] as u32) / 4) as u8;
            out
        });

        test_pipeline("pipe_sep_hv_round", &|src, x, y, sw, _dw| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let h0 = (src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32 + 1) / 2;
                let h1 = (src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32 + 1) / 2;
                out[c] = ((h0 + h1 + 1) / 2) as u8;
            }
            out
        });

        // Test with R/B swap: original stores BGRA, might have R↔B swap in float conversion
        test_pipeline("pipe_rb_swap", &|src, x, y, sw, _dw| {
            let sx = x * 2; let sy = y * 2;
            let mut out = [0u8; 4];
            for c in 0..4 {
                let sum = src[sy*sw+sx][c] as u32 + src[sy*sw+sx+1][c] as u32
                    + src[(sy+1)*sw+sx][c] as u32 + src[(sy+1)*sw+sx+1][c] as u32;
                out[c] = (sum / 4) as u8;
            }
            // Swap R and B
            out.swap(0, 2);
            out
        });
    }

    #[test]
    #[ignore]
    fn encoder_replication_pixel_deep_analysis() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();

        let entry = ref_idx.entries.iter().find(|e| e.id == 19767).unwrap();
        let ref_data = {
            let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            f.read_exact(&mut buf).unwrap();
            buf
        };

        let block_size = 8usize;

        // Extract mip0 (512x512) and mip1 (256x256), stored smallest-first
        let mip0_size = 128 * 128 * block_size; // 131072
        let mip0_offset = ref_data.len() - mip0_size;
        let ref_mip0 = &ref_data[mip0_offset..];

        let mip1_size = 64 * 64 * block_size; // 32768
        let mip1_offset = mip0_offset - mip1_size;
        let ref_mip1 = &ref_data[mip1_offset..mip1_offset + mip1_size];

        // ── Step 1: Decode reference mip0 (512x512) to pixels ──
        let mut mip0_pixels = vec![[0u8; 4]; 512 * 512];
        for by in 0..128usize {
            for bx in 0..128usize {
                let off = (by * 128 + bx) * block_size;
                let block = &ref_mip0[off..off + block_size];
                let decoded = decode_dxt1_block(block);
                for py in 0..4 {
                    for px in 0..4 {
                        mip0_pixels[(by * 4 + py) * 512 + bx * 4 + px] = decoded[py * 4 + px];
                    }
                }
            }
        }

        // ── Step 2: Box-filter (truncating integer division) to 256x256 ──
        let mut our_pixels = vec![[0u8; 4]; 256 * 256];
        for y in 0..256usize {
            for x in 0..256usize {
                let p00 = mip0_pixels[(y * 2) * 512 + x * 2];
                let p10 = mip0_pixels[(y * 2) * 512 + x * 2 + 1];
                let p01 = mip0_pixels[(y * 2 + 1) * 512 + x * 2];
                let p11 = mip0_pixels[(y * 2 + 1) * 512 + x * 2 + 1];
                for c in 0..4 {
                    let sum = p00[c] as u32 + p10[c] as u32 + p01[c] as u32 + p11[c] as u32;
                    our_pixels[y * 256 + x][c] = (sum / 4) as u8;
                }
            }
        }

        // ── Step 3: Decode reference mip1 (256x256) to pixels ──
        let mut ref_pixels = vec![[0u8; 4]; 256 * 256];
        // Also store per-pixel palette info: which block, which palette index
        let mut ref_palette_idx = vec![0u8; 256 * 256];
        // Store DXT1 block palette for each block
        let mut ref_block_palettes: Vec<[[u8; 4]; 4]> = Vec::with_capacity(64 * 64);
        for by in 0..64usize {
            for bx in 0..64usize {
                let b = by * 64 + bx;
                let off = b * block_size;
                let block = &ref_mip1[off..off + block_size];
                let c0 = u16::from_le_bytes([block[0], block[1]]);
                let c1 = u16::from_le_bytes([block[2], block[3]]);
                let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);

                let (r0, g0, b0) = decode_rgb565(c0);
                let (r1, g1, b1) = decode_rgb565(c1);
                let palette: [[u8; 4]; 4] = if c0 > c1 {
                    [
                        [r0, g0, b0, 255],
                        [r1, g1, b1, 255],
                        [
                            ((2 * r0 as u16 + r1 as u16) / 3) as u8,
                            ((2 * g0 as u16 + g1 as u16) / 3) as u8,
                            ((2 * b0 as u16 + b1 as u16) / 3) as u8,
                            255,
                        ],
                        [
                            ((r0 as u16 + 2 * r1 as u16) / 3) as u8,
                            ((g0 as u16 + 2 * g1 as u16) / 3) as u8,
                            ((b0 as u16 + 2 * b1 as u16) / 3) as u8,
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
                ref_block_palettes.push(palette);

                for py in 0..4 {
                    for px in 0..4 {
                        let i = py * 4 + px;
                        let sel = ((indices >> (i * 2)) & 3) as usize;
                        ref_pixels[(by * 4 + py) * 256 + bx * 4 + px] = palette[sel];
                        ref_palette_idx[(by * 4 + py) * 256 + bx * 4 + px] = sel as u8;
                    }
                }
            }
        }

        // ── Step 4: Pixel-by-pixel comparison ──
        let total_pixels = 256 * 256;
        let mut exact_match_count = 0u64;

        // Absolute difference histogram (per pixel max channel diff)
        let mut abs_diff_hist = [0u64; 256];

        // Per-channel signed difference histograms: index = diff + 255 (range 0..511)
        let mut r_diff_hist = vec![0u64; 511];
        let mut g_diff_hist = vec![0u64; 511];
        let mut b_diff_hist = vec![0u64; 511];

        // Per-channel squared error sums for RMSE
        let mut r_sq_err = 0.0f64;
        let mut g_sq_err = 0.0f64;
        let mut b_sq_err = 0.0f64;

        // Position-within-block correlation: [py][px] -> (mismatch_count, total_abs_diff)
        let mut block_pos_mismatch = [[0u64; 4]; 4];
        let mut block_pos_total_diff = [[0u64; 4]; 4];

        // Edge vs center correlation
        // Edge = first/last 16 pixels in each dimension
        let edge_margin = 16usize;
        let mut edge_mismatch = 0u64;
        let mut edge_total = 0u64;
        let mut center_mismatch = 0u64;
        let mut center_total = 0u64;

        // Palette membership check: is every ref pixel one of the 4 palette colors?
        let mut palette_member_yes = 0u64;
        let mut palette_member_no = 0u64;

        // Track first 10 mismatched pixels for detailed output
        struct MismatchDetail {
            x: usize,
            y: usize,
            our: [u8; 4],
            reference: [u8; 4],
            sources: [[u8; 4]; 4],
        }
        let mut mismatch_details: Vec<MismatchDetail> = Vec::new();

        for y in 0..256usize {
            for x in 0..256usize {
                let idx = y * 256 + x;
                let ours = our_pixels[idx];
                let refs = ref_pixels[idx];

                // Per-channel signed diffs (ours - ref)
                let dr = ours[0] as i32 - refs[0] as i32;
                let dg = ours[1] as i32 - refs[1] as i32;
                let db = ours[2] as i32 - refs[2] as i32;

                r_diff_hist[(dr + 255) as usize] += 1;
                g_diff_hist[(dg + 255) as usize] += 1;
                b_diff_hist[(db + 255) as usize] += 1;

                r_sq_err += (dr * dr) as f64;
                g_sq_err += (dg * dg) as f64;
                b_sq_err += (db * db) as f64;

                let abs_max = dr.abs().max(dg.abs()).max(db.abs()) as usize;
                abs_diff_hist[abs_max] += 1;

                if abs_max == 0 {
                    exact_match_count += 1;
                }

                // Position within 4x4 block
                let px_in_block = x % 4;
                let py_in_block = y % 4;
                if abs_max > 0 {
                    block_pos_mismatch[py_in_block][px_in_block] += 1;
                    block_pos_total_diff[py_in_block][px_in_block] += abs_max as u64;
                }

                // Edge vs center
                let is_edge = x < edge_margin || x >= 256 - edge_margin
                    || y < edge_margin || y >= 256 - edge_margin;
                if is_edge {
                    edge_total += 1;
                    if abs_max > 0 { edge_mismatch += 1; }
                } else {
                    center_total += 1;
                    if abs_max > 0 { center_mismatch += 1; }
                }

                // Palette membership check
                let blk_x = x / 4;
                let blk_y = y / 4;
                let blk_idx = blk_y * 64 + blk_x;
                let palette = &ref_block_palettes[blk_idx];
                let is_palette_member = palette.iter().any(|p| p[0] == refs[0] && p[1] == refs[1] && p[2] == refs[2]);
                if is_palette_member {
                    palette_member_yes += 1;
                } else {
                    palette_member_no += 1;
                }

                // Collect first 10 mismatched pixels with source detail
                if abs_max > 0 && mismatch_details.len() < 10 {
                    let sy = y * 2;
                    let sx = x * 2;
                    let sources = [
                        mip0_pixels[sy * 512 + sx],
                        mip0_pixels[sy * 512 + sx + 1],
                        mip0_pixels[(sy + 1) * 512 + sx],
                        mip0_pixels[(sy + 1) * 512 + sx + 1],
                    ];
                    mismatch_details.push(MismatchDetail {
                        x, y,
                        our: ours,
                        reference: refs,
                        sources,
                    });
                }
            }
        }

        // ── Output results ──

        eprintln!("\n  === DEEP PIXEL ANALYSIS: texture 19767 mip0->mip1 ===");
        eprintln!("  Total pixels: {}", total_pixels);
        eprintln!("  Exact matches: {} ({:.2}%)", exact_match_count,
            exact_match_count as f64 / total_pixels as f64 * 100.0);

        // Absolute difference histogram
        eprintln!("\n  Absolute max-channel difference histogram:");
        for d in 0..=20 {
            if abs_diff_hist[d] > 0 {
                eprintln!("    diff={:3}: {:7} pixels ({:.2}%)", d, abs_diff_hist[d],
                    abs_diff_hist[d] as f64 / total_pixels as f64 * 100.0);
            }
        }
        // Any larger diffs
        let large_diff: u64 = abs_diff_hist[21..].iter().sum();
        if large_diff > 0 {
            let max_nonzero = abs_diff_hist.iter().rposition(|&v| v > 0).unwrap_or(0);
            eprintln!("    diff>20:  {:7} pixels (max diff={})", large_diff, max_nonzero);
        }

        // Per-channel signed difference histograms (show non-zero entries near zero)
        eprintln!("\n  Per-channel signed difference histograms (ours - ref):");
        for (name, hist) in [("R", &r_diff_hist), ("G", &g_diff_hist), ("B", &b_diff_hist)] {
            eprint!("    {}: ", name);
            for d in -15i32..=15 {
                let count = hist[(d + 255) as usize];
                if count > 0 {
                    eprint!("[{:+}]={} ", d, count);
                }
            }
            // Check for anything outside [-15,+15]
            let outside: u64 = hist[..240].iter().sum::<u64>() + hist[271..].iter().sum::<u64>();
            if outside > 0 {
                eprint!("[|d|>15]={}", outside);
            }
            eprintln!();
        }

        // RMSE per channel
        let n = total_pixels as f64;
        eprintln!("\n  RMSE per channel:");
        eprintln!("    R: {:.4}", (r_sq_err / n).sqrt());
        eprintln!("    G: {:.4}", (g_sq_err / n).sqrt());
        eprintln!("    B: {:.4}", (b_sq_err / n).sqrt());
        eprintln!("    Combined: {:.4}", ((r_sq_err + g_sq_err + b_sq_err) / (3.0 * n)).sqrt());

        // Position-within-block correlation
        eprintln!("\n  Mismatch count by position within 4x4 block:");
        for py in 0..4 {
            eprintln!("    row {}: {:6} {:6} {:6} {:6}",
                py,
                block_pos_mismatch[py][0], block_pos_mismatch[py][1],
                block_pos_mismatch[py][2], block_pos_mismatch[py][3]);
        }
        eprintln!("  Avg abs diff by position within 4x4 block:");
        for py in 0..4 {
            let row: Vec<String> = (0..4).map(|px| {
                if block_pos_mismatch[py][px] > 0 {
                    format!("{:.2}", block_pos_total_diff[py][px] as f64 / block_pos_mismatch[py][px] as f64)
                } else {
                    "  n/a".to_string()
                }
            }).collect();
            eprintln!("    row {}: {:>6} {:>6} {:>6} {:>6}", py, row[0], row[1], row[2], row[3]);
        }

        // Edge vs center
        eprintln!("\n  Edge vs center mismatch (edge margin = {} px):", edge_margin);
        eprintln!("    Edge:   {}/{} mismatch ({:.2}%)", edge_mismatch, edge_total,
            if edge_total > 0 { edge_mismatch as f64 / edge_total as f64 * 100.0 } else { 0.0 });
        eprintln!("    Center: {}/{} mismatch ({:.2}%)", center_mismatch, center_total,
            if center_total > 0 { center_mismatch as f64 / center_total as f64 * 100.0 } else { 0.0 });

        // Palette membership verification
        eprintln!("\n  Palette membership verification:");
        eprintln!("    Ref pixel IS a palette color: {} ({:.2}%)", palette_member_yes,
            palette_member_yes as f64 / total_pixels as f64 * 100.0);
        eprintln!("    Ref pixel NOT a palette color: {} ({:.2}%)", palette_member_no,
            palette_member_no as f64 / total_pixels as f64 * 100.0);

        // First 10 mismatched pixels detail
        eprintln!("\n  First {} mismatched pixels detail:", mismatch_details.len());
        for (i, d) in mismatch_details.iter().enumerate() {
            eprintln!("    #{}: pixel ({},{}) in block ({},{})", i, d.x, d.y, d.x / 4, d.y / 4);
            eprintln!("       Source pixels: {:?} {:?} {:?} {:?}", d.sources[0], d.sources[1], d.sources[2], d.sources[3]);
            let sum_r = d.sources.iter().map(|s| s[0] as u32).sum::<u32>();
            let sum_g = d.sources.iter().map(|s| s[1] as u32).sum::<u32>();
            let sum_b = d.sources.iter().map(|s| s[2] as u32).sum::<u32>();
            eprintln!("       Sum R={} G={} B={}, trunc avg=({},{},{}), round avg=({},{},{})",
                sum_r, sum_g, sum_b,
                sum_r / 4, sum_g / 4, sum_b / 4,
                (sum_r + 2) / 4, (sum_g + 2) / 4, (sum_b + 2) / 4);
            eprintln!("       Our filtered: ({},{},{})  Ref decoded: ({},{},{})",
                d.our[0], d.our[1], d.our[2], d.reference[0], d.reference[1], d.reference[2]);
            eprintln!("       Diff: R={:+} G={:+} B={:+}",
                d.our[0] as i32 - d.reference[0] as i32,
                d.our[1] as i32 - d.reference[1] as i32,
                d.our[2] as i32 - d.reference[2] as i32);
        }

        // Summary: this is a pre-encoder analysis. The key insight is whether the
        // filtered pixels match the reference decoded pixels closely enough that
        // the encoder is the bottleneck, or if filtering itself introduces the gap.
        let mismatch_pct = (total_pixels as u64 - exact_match_count) as f64 / total_pixels as f64 * 100.0;
        eprintln!("\n  SUMMARY: {:.2}% of filtered pixels differ from reference decoded pixels.", mismatch_pct);
        eprintln!("  If this is high, the problem is likely in the filter or decode, not the encoder.");
        eprintln!("  If this is low, the encoder is the bottleneck.");
    }

    #[test]
    #[ignore] // Run with: cargo test inspect_cdn_dds -- --ignored --nocapture
    fn inspect_cdn_dds() {
        use crate::rdb::{cdn_url_from_hash, parse_le_index};
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");

        let le_idx = parse_le_index(&ref_dir.join("le.idx"))
            .expect("Failed to parse le.idx");

        let test_ids: &[u32] = &[19767, 27137, 30186];
        let cdn_base = "https://update.secretworld.com/tswupm";

        let client = reqwest::blocking::Client::builder()
            .user_agent("")
            .build()
            .expect("build HTTP client");

        for &tex_id in test_ids {
            let entry = le_idx.entries.iter()
                .find(|e| e.id == tex_id && e.rdb_type == 1010004)
                .unwrap_or_else(|| panic!("Texture {} not found in le.idx", tex_id));

            let url = cdn_url_from_hash(cdn_base, &entry.hash);
            eprintln!("\n=== Texture {} ===", tex_id);
            eprintln!("  CDN URL: {}", url);

            let resp = client.get(&url).send()
                .unwrap_or_else(|e| panic!("Download failed for {}: {}", tex_id, e));

            if !resp.status().is_success() {
                eprintln!("  HTTP {}: skipping", resp.status());
                continue;
            }

            let raw_bytes = resp.bytes()
                .unwrap_or_else(|e| panic!("Read body failed: {}", e));
            eprintln!("  Downloaded: {} bytes", raw_bytes.len());
            eprintln!("  First 4 bytes: {:?}", &raw_bytes[..4.min(raw_bytes.len())]);

            let iog1_data = crate::verify::decompress_ioz1(&raw_bytes)
                .expect("IOz1 decompression failed");

            eprintln!("  After IOz1: {} bytes, magic: {:?}",
                iog1_data.len(),
                std::str::from_utf8(&iog1_data[..4.min(iog1_data.len())]).unwrap_or("???"));

            if !is_iog1(&iog1_data) {
                eprintln!("  NOT IOg1 data — skipping DDS inspection");
                continue;
            }

            let info = inspect_iog1_dds(&iog1_data)
                .expect("DDS inspection failed");

            eprintln!("  DDS dimensions: {}x{}", info.width, info.height);
            eprintln!("  DDS FourCC: {:?}", std::str::from_utf8(&info.fourcc).unwrap_or("???"));
            eprintln!("  DDS dwMipMapCount: {}", info.mip_map_count);
            eprintln!("  DDS payload size: {} bytes", info.payload_size);
            eprintln!("  Expected mip0 size: {} bytes", info.mip0_size);
            eprintln!("  Extra payload beyond mip0: {} bytes", info.payload_size.saturating_sub(info.mip0_size));
            eprintln!("  IOg1 format tag: {:?}", std::str::from_utf8(&info.iog1_fmt_tag).unwrap_or("???"));
            eprintln!("  IOg1 mip count: {}", info.iog1_mip_count);
            eprintln!("  IOg1 mip sizes: {:?}", info.iog1_mip_sizes);

            if info.mip_map_count > 1 && info.payload_size > info.mip0_size {
                eprintln!("  >>> DDS HAS PRE-GENERATED MIPS! <<<");

                let our_fctx = decompress_iog1(&iog1_data)
                    .expect("decompress_iog1 failed");

                let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
                let mut file = std::fs::File::open(&ref_path).expect("open rdbdata");
                file.seek(SeekFrom::Start(entry.offset as u64)).expect("seek");
                let mut ref_data = vec![0u8; entry.length as usize];
                file.read_exact(&mut ref_data).expect("read ref");

                if our_fctx == ref_data {
                    eprintln!("  >>> BYTE-IDENTICAL MATCH WITH REFERENCE! <<<");
                } else {
                    let matching = our_fctx.iter().zip(ref_data.iter())
                        .filter(|(a, b)| a == b).count();
                    eprintln!("  Byte match: {}/{} ({:.1}%)",
                        matching, ref_data.len().min(our_fctx.len()),
                        matching as f64 / ref_data.len().min(our_fctx.len()) as f64 * 100.0);
                    for (i, (a, b)) in our_fctx.iter().zip(ref_data.iter()).enumerate() {
                        if a != b {
                            eprintln!("  First diff at byte {}: ours={:02X} ref={:02X}", i, a, b);
                            break;
                        }
                    }
                }
            } else {
                eprintln!("  DDS has mip0 only — patcher must generate mips");
            }
        }
    }

    #[test]
    #[ignore] // Run with: cargo test pixel_gamma_diagnostic -- --ignored --nocapture
    fn pixel_gamma_diagnostic() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx"))
            .expect("Failed to parse le.idx");
        let block_size = 8usize; // DXT1
        let fctx_header_size = 24usize;

        let (id, width, height) = (19767u32, 512usize, 512usize);
        let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
        let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
        let ref_data = {
            let mut file = std::fs::File::open(&ref_path).unwrap();
            file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            file.read_exact(&mut buf).unwrap();
            buf
        };

        // Extract reference mip levels
        let mut mip_sizes = Vec::new();
        let (mut w, mut h) = (width, height);
        loop {
            mip_sizes.push(((w + 3) / 4) * ((h + 3) / 4) * block_size);
            if w <= 1 && h <= 1 { break; }
            w = (w / 2).max(1); h = (h / 2).max(1);
        }
        let mut ref_mips: Vec<&[u8]> = Vec::new();
        let mut off = fctx_header_size;
        for i in (0..mip_sizes.len()).rev() {
            ref_mips.push(&ref_data[off..off + mip_sizes[i]]);
            off += mip_sizes[i];
        }
        ref_mips.reverse();

        // Decode mip0 to pixel buffer
        let (mip0_w, mip0_h) = (width, height);
        let pbx = (mip0_w / 4).max(1);
        let pby = (mip0_h / 4).max(1);
        let mut mip0_pixels = vec![[0u8; 4]; mip0_w * mip0_h];
        for by in 0..pby {
            for bx in 0..pbx {
                let o = (by * pbx + bx) * block_size;
                if o + block_size > ref_mips[0].len() { continue; }
                let px = decode_dxt1_block(&ref_mips[0][o..o + block_size]);
                for py in 0..4 {
                    for ppx in 0..4 {
                        let (xx, yy) = (bx * 4 + ppx, by * 4 + py);
                        if xx < mip0_w && yy < mip0_h {
                            mip0_pixels[yy * mip0_w + xx] = px[py * 4 + ppx];
                        }
                    }
                }
            }
        }

        // Mip1 dimensions
        let (mip1_w, mip1_h) = (width / 2, height / 2);
        let mip1_bx = (mip1_w / 4).max(1);
        let mip1_by = (mip1_h / 4).max(1);
        let ref_mip1 = ref_mips[1];

        // Precompute ALL mip1 pixels via both pipelines
        let recip255: f32 = 1.0 / 255.0;
        let gamma: f32 = 2.2;
        let inv_gamma: f32 = 1.0 / 2.2;
        let scale255: f32 = 255.0;

        let mut nogamma_pixels = vec![[0u8; 4]; mip1_w * mip1_h];
        let mut gamma_pixels = vec![[0u8; 4]; mip1_w * mip1_h];

        for y in 0..mip1_h {
            for x in 0..mip1_w {
                let sx = x * 2;
                let sy = y * 2;
                let x0 = sx.min(mip0_w - 1);
                let y0 = sy.min(mip0_h - 1);
                let x1 = (sx + 1).min(mip0_w - 1);
                let y1 = (sy + 1).min(mip0_h - 1);

                let p00 = mip0_pixels[y0 * mip0_w + x0];
                let p10 = mip0_pixels[y0 * mip0_w + x1];
                let p01 = mip0_pixels[y1 * mip0_w + x0];
                let p11 = mip0_pixels[y1 * mip0_w + x1];

                for c in 0..4 {
                    // No-gamma: u8→float, x87 box filter, x87 float→u8 (floor)
                    let f0 = p00[c] as f32 * recip255;
                    let f1 = p10[c] as f32 * recip255;
                    let f2 = p01[c] as f32 * recip255;
                    let f3 = p11[c] as f32 * recip255;
                    let avg = x87_box_filter_f32(f0, f1, f2, f3);
                    nogamma_pixels[y * mip1_w + x][c] = x87_float_to_u8(avg);

                    // Gamma: u8→float→linearize→x87 box filter→degamma→u8
                    if c < 3 {
                        let lin0 = x87_powf(f0, gamma);
                        let lin1 = x87_powf(f1, gamma);
                        let lin2 = x87_powf(f2, gamma);
                        let lin3 = x87_powf(f3, gamma);
                        let avg_lin = x87_box_filter_f32(lin0, lin1, lin2, lin3);
                        let v = x87_pow_scale_floor(avg_lin, inv_gamma, scale255);
                        gamma_pixels[y * mip1_w + x][c] = v.clamp(0, 255) as u8;
                    } else {
                        // Alpha: no gamma, same as no-gamma path
                        gamma_pixels[y * mip1_w + x][c] = x87_float_to_u8(avg);
                    }
                }
            }
        }

        // Compare solid blocks
        let mut total_solid = 0usize;
        let mut match_nogamma = 0usize;
        let mut match_gamma = 0usize;
        let mut match_neither = 0usize;
        let mut match_both = 0usize;
        let mut printed = 0usize;

        for by in 0..mip1_by {
            for bx in 0..mip1_bx {
                let b = by * mip1_bx + bx;
                let rb = &ref_mip1[b * 8..(b + 1) * 8];
                let rc0 = u16::from_le_bytes([rb[0], rb[1]]);
                let rc1 = u16::from_le_bytes([rb[2], rb[3]]);
                if rc0 != rc1 { continue; } // not a solid block in reference
                total_solid += 1;

                // Get our pixel values for this block from both pipelines
                let mut ng_block = [[0u8; 4]; 16];
                let mut gm_block = [[0u8; 4]; 16];
                for py in 0..4 {
                    for px in 0..4 {
                        let mx = (bx * 4 + px).min(mip1_w - 1);
                        let my = (by * 4 + py).min(mip1_h - 1);
                        ng_block[py * 4 + px] = nogamma_pixels[my * mip1_w + mx];
                        gm_block[py * 4 + px] = gamma_pixels[my * mip1_w + mx];
                    }
                }

                // Encode both via the same encoder path
                let ng_encoded = encode_dxt1_block(&ng_block, false, true);
                let gm_encoded = encode_dxt1_block(&gm_block, false, true);

                let ng_match = ng_encoded == rb;
                let gm_match = gm_encoded == rb;

                if ng_match && gm_match { match_both += 1; }
                else if ng_match { match_nogamma += 1; }
                else if gm_match { match_gamma += 1; }
                else { match_neither += 1; }

                // Print details for first 20 mismatched blocks
                if !ng_match && printed < 20 {
                    printed += 1;
                    let ng_c0 = u16::from_le_bytes([ng_encoded[0], ng_encoded[1]]);
                    let gm_c0 = u16::from_le_bytes([gm_encoded[0], gm_encoded[1]]);
                    // Show one representative pixel (top-left of block)
                    let mx = bx * 4;
                    let my = by * 4;
                    let ng_px = nogamma_pixels[my * mip1_w + mx];
                    let gm_px = gamma_pixels[my * mip1_w + mx];
                    // Show source mip0 pixels for that position
                    let sx = mx * 2; let sy = my * 2;
                    let s00 = mip0_pixels[sy * mip0_w + sx];
                    let s10 = mip0_pixels[sy * mip0_w + sx + 1];
                    let s01 = mip0_pixels[(sy + 1) * mip0_w + sx];
                    let s11 = mip0_pixels[(sy + 1) * mip0_w + sx + 1];
                    eprintln!("  block ({},{}) ref_c0={:04X} ng_c0={:04X}{} gm_c0={:04X}{}",
                        bx, by, rc0,
                        ng_c0, if ng_match { " MATCH" } else { "" },
                        gm_c0, if gm_match { " MATCH" } else { "" });
                    eprintln!("    src: ({},{},{}) ({},{},{}) ({},{},{}) ({},{},{})",
                        s00[0],s00[1],s00[2], s10[0],s10[1],s10[2],
                        s01[0],s01[1],s01[2], s11[0],s11[1],s11[2]);
                    eprintln!("    nogamma=({},{},{}) gamma=({},{},{})",
                        ng_px[0],ng_px[1],ng_px[2], gm_px[0],gm_px[1],gm_px[2]);
                }
            }
        }

        eprintln!("\n=== Pixel-Level Gamma Diagnostic: Texture {} mip1 ===", id);
        eprintln!("  Total solid blocks in reference: {}", total_solid);
        eprintln!("  Match with no-gamma only: {}", match_nogamma);
        eprintln!("  Match with gamma only:    {}", match_gamma);
        eprintln!("  Match with BOTH:          {}", match_both);
        eprintln!("  Match with NEITHER:       {}", match_neither);
        eprintln!("  No-gamma total match: {} ({:.1}%)", match_both + match_nogamma,
            (match_both + match_nogamma) as f64 / total_solid as f64 * 100.0);
        eprintln!("  Gamma total match:    {} ({:.1}%)", match_both + match_gamma,
            (match_both + match_gamma) as f64 / total_solid as f64 * 100.0);
    }

    #[test]
    fn test_x87_powf_matches_original() {
        // Expected values from Unicorn emulation of FUN_682EE0
        // Generated by: python3 tools/emulate_pow.py
        let gamma: f32 = 2.2;
        let recip255: f32 = 1.0 / 255.0;

        let test_cases: &[(u8, u32)] = &[
            (0, 0x00000000),
            (1, 0x36AA5B8A),
            (2, 0x37C3B081),
            (10, 0x3A52EFB7),
            (50, 0x3CE35F0C),
            (100, 0x3E02972C),
            (127, 0x3E5CF159),
            (128, 0x3E60C9C8),
            (148, 0x3E9AB032),
            (200, 0x3F160255),
            (240, 0x3F600906),
            (254, 0x3F7DCBEE),
            (255, 0x3F800000),
        ];

        let mut mismatches = 0;
        for &(v, expected_bits) in test_cases {
            let f32_input = v as f32 * recip255;
            let our_result = if f32_input <= 0.0 { 0.0f32 } else { x87_powf(f32_input, gamma) };
            let our_bits = our_result.to_bits();
            if our_bits != expected_bits {
                mismatches += 1;
                eprintln!("  MISMATCH v={}: ours=0x{:08X} expected=0x{:08X} (diff={}ULP)",
                    v, our_bits, expected_bits,
                    (our_bits as i64 - expected_bits as i64).unsigned_abs());
            }
        }
        if mismatches > 0 {
            panic!("{} values differ from original FUN_682EE0", mismatches);
        }
        eprintln!("  All {} pow values match FUN_682EE0 exactly", test_cases.len());
    }

    #[test]
    #[ignore] // Run with: cargo test gamma_overlap_analysis -- --ignored --nocapture
    fn gamma_overlap_analysis() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx"))
            .expect("Failed to parse le.idx");
        let block_size = 8usize;
        let fctx_header_size = 24usize;
        let recip255: f32 = 1.0 / 255.0;
        let gamma_val: f32 = 2.2;
        let inv_gamma: f32 = 1.0 / 2.2;
        let scale255: f32 = 255.0;

        let (id, width, height) = (19767u32, 512usize, 512usize);
        let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
        let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
        let ref_data = {
            let mut file = std::fs::File::open(&ref_path).unwrap();
            file.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut buf = vec![0u8; entry.length as usize];
            file.read_exact(&mut buf).unwrap();
            buf
        };

        // Extract mip levels
        let mut mip_sizes = Vec::new();
        let (mut w, mut h) = (width, height);
        loop {
            mip_sizes.push(((w+3)/4)*((h+3)/4)*block_size);
            if w<=1 && h<=1 { break; }
            w=(w/2).max(1); h=(h/2).max(1);
        }
        let mut ref_mips: Vec<&[u8]> = Vec::new();
        let mut off = fctx_header_size;
        for i in (0..mip_sizes.len()).rev() {
            ref_mips.push(&ref_data[off..off+mip_sizes[i]]);
            off += mip_sizes[i];
        }
        ref_mips.reverse();

        // Decode mip0
        let (mip0_w, mip0_h) = (width, height);
        let pbx = (mip0_w/4).max(1);
        let pby = (mip0_h/4).max(1);
        let mut decoded = vec![[0u8;4]; mip0_w*mip0_h];
        for by in 0..pby { for bx in 0..pbx {
            let o = (by*pbx+bx)*block_size;
            if o+block_size > ref_mips[0].len() { continue; }
            let px = decode_dxt1_block(&ref_mips[0][o..o+block_size]);
            for py in 0..4 { for ppx in 0..4 {
                let (xx,yy) = (bx*4+ppx, by*4+py);
                if xx<mip0_w && yy<mip0_h { decoded[yy*mip0_w+xx] = px[py*4+ppx]; }
            }}
        }}

        let (mip1_w, mip1_h) = (width/2, height/2);
        let nbx = (mip1_w/4).max(1);
        let nby = (mip1_h/4).max(1);
        let num_blocks = nbx * nby;

        // === No-gamma mip1 ===
        let mut nogamma_f = vec![[0.0f32;4]; mip0_w*mip0_h];
        for (i,p) in decoded.iter().enumerate() {
            nogamma_f[i] = [p[0] as f32*recip255, p[1] as f32*recip255,
                            p[2] as f32*recip255, p[3] as f32*recip255];
        }
        let mut ng_filtered = vec![[0.0f32;4]; mip1_w*mip1_h];
        for y in 0..mip1_h { for x in 0..mip1_w {
            let (x0,y0) = ((x*2).min(mip0_w-1),(y*2).min(mip0_h-1));
            let (x1,y1) = ((x*2+1).min(mip0_w-1),(y*2+1).min(mip0_h-1));
            for c in 0..4 {
                ng_filtered[y*mip1_w+x][c] = x87_box_filter_f32(
                    nogamma_f[y0*mip0_w+x0][c], nogamma_f[y0*mip0_w+x1][c],
                    nogamma_f[y1*mip0_w+x0][c], nogamma_f[y1*mip0_w+x1][c]);
            }
        }}
        let mut ng_mip = Vec::with_capacity(num_blocks*block_size);
        for by in 0..nby { for bx in 0..nbx {
            let mut bp = [[0u8;4];16];
            for py in 0..4 { for px in 0..4 {
                let fv = &ng_filtered[((by*4+py).min(mip1_h-1))*mip1_w+(bx*4+px).min(mip1_w-1)];
                bp[py*4+px] = [x87_float_to_u8(fv[0]), x87_float_to_u8(fv[1]),
                               x87_float_to_u8(fv[2]), x87_float_to_u8(fv[3])];
            }}
            ng_mip.extend_from_slice(&encode_dxt1_block(&bp, false, true));
        }}

        // === Gamma mip1 ===
        let mut gamma_f = vec![[0.0f32;4]; mip0_w*mip0_h];
        for (i,p) in decoded.iter().enumerate() {
            let fv = [p[0] as f32*recip255, p[1] as f32*recip255,
                      p[2] as f32*recip255, p[3] as f32*recip255];
            gamma_f[i] = [x87_powf(fv[0], gamma_val), x87_powf(fv[1], gamma_val),
                          x87_powf(fv[2], gamma_val), fv[3]];
        }
        let mut gm_filtered = vec![[0.0f32;4]; mip1_w*mip1_h];
        for y in 0..mip1_h { for x in 0..mip1_w {
            let (x0,y0) = ((x*2).min(mip0_w-1),(y*2).min(mip0_h-1));
            let (x1,y1) = ((x*2+1).min(mip0_w-1),(y*2+1).min(mip0_h-1));
            for c in 0..4 {
                gm_filtered[y*mip1_w+x][c] = x87_box_filter_f32(
                    gamma_f[y0*mip0_w+x0][c], gamma_f[y0*mip0_w+x1][c],
                    gamma_f[y1*mip0_w+x0][c], gamma_f[y1*mip0_w+x1][c]);
            }
        }}
        let mut gm_mip = Vec::with_capacity(num_blocks*block_size);
        for by in 0..nby { for bx in 0..nbx {
            let mut bp = [[0u8;4];16];
            for py in 0..4 { for px in 0..4 {
                let fv = &gm_filtered[((by*4+py).min(mip1_h-1))*mip1_w+(bx*4+px).min(mip1_w-1)];
                for c in 0..3 {
                    let v = x87_pow_scale_floor(fv[c], inv_gamma, scale255);
                    bp[py*4+px][c] = v.clamp(0, 255) as u8;
                }
                bp[py*4+px][3] = x87_float_to_u8(fv[3]);
            }}
            gm_mip.extend_from_slice(&encode_dxt1_block(&bp, false, true));
        }}

        // === Compare ===
        let ref_mip1 = ref_mips[1];
        let mut both = 0usize;
        let mut ng_only = 0usize;
        let mut gm_only = 0usize;
        let mut neither = 0usize;
        let mut ref_solid = 0usize;

        for b in 0..num_blocks {
            let rb = &ref_mip1[b*8..(b+1)*8];
            let ng = &ng_mip[b*8..(b+1)*8];
            let gm = &gm_mip[b*8..(b+1)*8];
            let rc0 = u16::from_le_bytes([rb[0], rb[1]]);
            let rc1 = u16::from_le_bytes([rb[2], rb[3]]);
            if rc0 == rc1 { ref_solid += 1; }
            let ng_match = ng == rb;
            let gm_match = gm == rb;
            if ng_match && gm_match { both += 1; }
            else if ng_match { ng_only += 1; }
            else if gm_match { gm_only += 1; }
            else { neither += 1; }
        }

        eprintln!("\n=== Gamma vs No-Gamma Overlap (ID {} mip1, {} blocks) ===", id, num_blocks);
        eprintln!("  Reference solid blocks: {}", ref_solid);
        eprintln!("  Match BOTH:        {:>5} ({:.1}%)", both, both as f64/num_blocks as f64*100.0);
        eprintln!("  No-gamma ONLY:     {:>5} ({:.1}%)", ng_only, ng_only as f64/num_blocks as f64*100.0);
        eprintln!("  Gamma ONLY:        {:>5} ({:.1}%)", gm_only, gm_only as f64/num_blocks as f64*100.0);
        eprintln!("  NEITHER:           {:>5} ({:.1}%)", neither, neither as f64/num_blocks as f64*100.0);
        eprintln!("  No-gamma total:    {:>5} ({:.1}%)", both+ng_only, (both+ng_only) as f64/num_blocks as f64*100.0);
        eprintln!("  Gamma total:       {:>5} ({:.1}%)", both+gm_only, (both+gm_only) as f64/num_blocks as f64*100.0);

        // Show first 10 "gamma ONLY" blocks (blocks gamma gets right that no-gamma doesn't)
        let mut printed = 0;
        for b in 0..num_blocks {
            if printed >= 10 { break; }
            let rb = &ref_mip1[b*8..(b+1)*8];
            let ng = &ng_mip[b*8..(b+1)*8];
            let gm = &gm_mip[b*8..(b+1)*8];
            if gm == rb && ng != rb {
                let bx = b % nbx; let by = b / nbx;
                eprintln!("  [gamma-only] block ({},{}) ref={:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
                    bx, by, rb[0],rb[1],rb[2],rb[3],rb[4],rb[5],rb[6],rb[7]);
                printed += 1;
            }
        }

        // Show first 10 "no-gamma ONLY" blocks
        printed = 0;
        for b in 0..num_blocks {
            if printed >= 10 { break; }
            let rb = &ref_mip1[b*8..(b+1)*8];
            let ng = &ng_mip[b*8..(b+1)*8];
            let gm = &gm_mip[b*8..(b+1)*8];
            if ng == rb && gm != rb {
                let bx = b % nbx; let by = b / nbx;
                eprintln!("  [ng-only] block ({},{}) ref={:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
                    bx, by, rb[0],rb[1],rb[2],rb[3],rb[4],rb[5],rb[6],rb[7]);
                printed += 1;
            }
        }
    }

    /// Compare one-shot degamma (pow*255 at 80-bit) vs two-step (pow→f32→*255→floor)
    #[test]
    fn test_degamma_oneshot_vs_twostep() {
        let recip255: f32 = 1.0 / 255.0;
        let gamma: f32 = 2.2;
        let inv_gamma: f32 = 1.0 / 2.2;
        let scale: f32 = 255.0;
        let mut diffs = 0;
        // Test all possible linearized values (from all 256 pixel values)
        for v in 0u16..=255 {
            let f32_val = v as f32 * recip255;
            let lin = x87_powf(f32_val, gamma); // linearize
            // One-shot: pow(lin, 1/2.2) * 255 at 80-bit, floor
            let oneshot = x87_pow_scale_floor(lin, inv_gamma, scale).clamp(0, 255) as u8;
            // Two-step: pow(lin, 1/2.2) → f32, then f32 * 255 → floor
            let pow_result = x87_powf(lin, inv_gamma); // pow → f32
            let twostep = x87_float_to_u8(pow_result); // f32 * 255, floor
            if oneshot != twostep {
                diffs += 1;
                if diffs <= 20 {
                    eprintln!("  v={}: oneshot={} twostep={} lin={:e} pow={:e}",
                        v, oneshot, twostep, lin, pow_result);
                }
            }
        }
        eprintln!("  Degamma oneshot vs twostep: {}/256 differ", diffs);
    }

    /// Check if x87_powf and f64 pow agree for ALL 256 pixel values.
    #[test]
    fn test_pow_x87_vs_f64_exhaustive() {
        let recip255: f32 = 1.0 / 255.0;
        let gamma: f32 = 2.2;
        let mut diffs = 0;
        for v in 0u16..=255 {
            let f32_val = v as f32 * recip255;
            let x87_result = if f32_val <= 0.0 { 0.0f32 } else { x87_powf(f32_val, gamma) };
            let f64_result = if f32_val <= 0.0 { 0.0 } else {
                (f32_val as f64).powf(gamma as f64) as f32
            };
            if x87_result.to_bits() != f64_result.to_bits() {
                diffs += 1;
                eprintln!("  v={}: x87=0x{:08X} f64=0x{:08X} diff={}ULP",
                    v, x87_result.to_bits(), f64_result.to_bits(),
                    (x87_result.to_bits() as i64 - f64_result.to_bits() as i64).unsigned_abs());
            }
        }
        eprintln!("  pow(val, 2.2): {}/256 differ between x87 and f64", diffs);

        // Also test degamma: pow(val, 1/2.2)
        let inv_gamma: f32 = 1.0 / 2.2;
        let mut diffs2 = 0;
        for v in 0u16..=255 {
            let f32_val = v as f32 * recip255;
            let x87_result = if f32_val <= 0.0 { 0.0f32 } else { x87_powf(f32_val, inv_gamma) };
            let f64_result = if f32_val <= 0.0 { 0.0 } else {
                (f32_val as f64).powf(inv_gamma as f64) as f32
            };
            if x87_result.to_bits() != f64_result.to_bits() {
                diffs2 += 1;
                if diffs2 <= 10 {
                    eprintln!("  v={}: x87=0x{:08X} f64=0x{:08X}", v, x87_result.to_bits(), f64_result.to_bits());
                }
            }
        }
        eprintln!("  pow(val, 1/2.2): {}/256 differ between x87 and f64", diffs2);
        assert_eq!(diffs + diffs2, 0, "x87 and f64 pow differ for {} values", diffs + diffs2);
    }

    /// Compare gamma pixel values computed two ways for ALL mip1 pixels:
    /// Method A: x87 inline asm (our pipeline)
    /// Method B: f64 math (should match x87 53-bit)
    /// Any difference reveals where our pipeline diverges.
    #[test]
    #[ignore]
    fn bulk_gamma_pixel_comparison() {
        use crate::rdb::parse_le_index;
        use std::io::{Read, Seek, SeekFrom};
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let ref_dir = base.join("game-installs/normal-full-loggedin/The Secret World/RDB");
        let ref_idx = parse_le_index(&ref_dir.join("le.idx")).unwrap();
        let block_size = 8usize;
        let fctx_header_size = 24usize;

        let (id, width, height) = (19767u32, 512usize, 512usize);
        let entry = ref_idx.entries.iter().find(|e| e.id == id).unwrap();
        let ref_path = ref_dir.join(format!("{:02}.rdbdata", entry.file_num));
        let ref_data = {
            let mut f = std::fs::File::open(&ref_path).unwrap();
            f.seek(SeekFrom::Start(entry.offset as u64)).unwrap();
            let mut b = vec![0u8; entry.length as usize];
            f.read_exact(&mut b).unwrap();
            b
        };

        // Extract mip0
        let mut ms = Vec::new();
        let (mut w, mut h) = (width, height);
        loop { ms.push(((w+3)/4)*((h+3)/4)*block_size); if w<=1&&h<=1{break;} w=(w/2).max(1); h=(h/2).max(1); }
        let mut rm: Vec<&[u8]> = Vec::new();
        let mut off = fctx_header_size;
        for i in (0..ms.len()).rev() { rm.push(&ref_data[off..off+ms[i]]); off+=ms[i]; }
        rm.reverse();

        // Decode mip0
        let pbx = (width/4).max(1); let pby = (height/4).max(1);
        let mut mip0 = vec![[0u8;4]; width*height];
        for by in 0..pby { for bx in 0..pbx {
            let o = (by*pbx+bx)*block_size;
            if o+block_size > rm[0].len() { continue; }
            let px = decode_dxt1_block(&rm[0][o..o+block_size]);
            for py in 0..4 { for ppx in 0..4 {
                let (xx,yy) = (bx*4+ppx, by*4+py);
                if xx<width && yy<height { mip0[yy*width+xx] = px[py*4+ppx]; }
            }}
        }}

        let (mip1_w, mip1_h) = (width/2, height/2);
        let recip255: f32 = 1.0 / 255.0;
        let gamma: f32 = 2.2;
        let inv_gamma: f32 = 1.0 / 2.2;
        let scale: f32 = 255.0;

        let mut total = 0usize;
        let mut diffs_r = 0usize;
        let mut diffs_g = 0usize;
        let mut diffs_b = 0usize;
        let mut printed = 0usize;

        for y in 0..mip1_h {
            for x in 0..mip1_w {
                let sx = x*2; let sy = y*2;
                let x0 = sx.min(width-1); let y0 = sy.min(height-1);
                let x1 = (sx+1).min(width-1); let y1 = (sy+1).min(height-1);

                let p00 = mip0[y0*width+x0];
                let p10 = mip0[y0*width+x1];
                let p01 = mip0[y1*width+x0];
                let p11 = mip0[y1*width+x1];

                for c in 0..3 {
                    // Method A: x87 pipeline (our code)
                    let f0 = p00[c] as f32 * recip255;
                    let f1 = p10[c] as f32 * recip255;
                    let f2 = p01[c] as f32 * recip255;
                    let f3 = p11[c] as f32 * recip255;
                    let lin0 = x87_powf(f0, gamma);
                    let lin1 = x87_powf(f1, gamma);
                    let lin2 = x87_powf(f2, gamma);
                    let lin3 = x87_powf(f3, gamma);
                    let avg_lin = x87_box_filter_f32(lin0, lin1, lin2, lin3);
                    let method_a = x87_pow_scale_floor(avg_lin, inv_gamma, scale)
                        .clamp(0, 255) as u8;

                    // Method B: f64 math (reference computation)
                    let df0 = (p00[c] as f64) * (recip255 as f64);
                    let df1 = (p10[c] as f64) * (recip255 as f64);
                    let df2 = (p01[c] as f64) * (recip255 as f64);
                    let df3 = (p11[c] as f64) * (recip255 as f64);
                    let dlin0 = df0.powf(gamma as f64);
                    let dlin1 = df1.powf(gamma as f64);
                    let dlin2 = df2.powf(gamma as f64);
                    let dlin3 = df3.powf(gamma as f64);
                    let davg = (dlin0 + dlin1 + dlin2 + dlin3) * 0.25;
                    let method_b = (davg.powf(inv_gamma as f64) * 255.0).floor()
                        .max(0.0).min(255.0) as u8;

                    // Method C: no-gamma (integer truncation)
                    let method_c = ((p00[c] as u32 + p10[c] as u32 +
                                     p01[c] as u32 + p11[c] as u32) / 4) as u8;

                    total += 1;
                    if method_a != method_b {
                        match c { 0 => diffs_r += 1, 1 => diffs_g += 1, _ => diffs_b += 1, }
                        if printed < 20 {
                            printed += 1;
                            eprintln!("  DIFF ({},{}) ch={}: x87={} f64={} nogamma={} src=[{},{},{},{}]",
                                x, y, c, method_a, method_b, method_c,
                                p00[c], p10[c], p01[c], p11[c]);
                        }
                    }
                }
            }
        }

        let total_diffs = diffs_r + diffs_g + diffs_b;
        eprintln!("\n=== Bulk Gamma Pixel Comparison ({} channels tested) ===", total);
        eprintln!("  Diffs: {} ({:.2}%) [R={}, G={}, B={}]",
            total_diffs, total_diffs as f64 / total as f64 * 100.0,
            diffs_r, diffs_g, diffs_b);
        if total_diffs == 0 {
            eprintln!("  x87 pipeline and f64 reference AGREE for all pixels");
        }
    }

    /// Feed ACTUAL pixel values captured from the live binary (via Frida)
    /// into our range-fit encoder and compare output.
    #[test]
    #[ignore]
    fn test_captured_pixel_pairs() {
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let pairs_path = base.join("tools/captured_pixel_pairs.json");
        let data = std::fs::read_to_string(&pairs_path)
            .expect("Run frida_pixel_capture2.py first to generate captured_pixel_pairs.json");

        let pairs: Vec<serde_json::Value> = serde_json::from_str(&data).unwrap();
        eprintln!("Loaded {} pairs from Frida capture", pairs.len());

        let mut total = 0usize;
        let mut match_full = 0usize;
        let mut match_c0c1 = 0usize;
        let mut match_c0 = 0usize;
        let mut solid = 0usize;
        let mut printed = 0usize;

        for pair in &pairs {
            let ref_c0 = pair["c0"].as_u64().unwrap() as u16;
            let ref_c1 = pair["c1"].as_u64().unwrap() as u16;
            let ref_idx = pair["ix"].as_u64().unwrap() as u32;
            let px_arr = pair["px"].as_array().unwrap();

            if ref_c0 == ref_c1 { solid += 1; continue; }
            total += 1;

            // Decode BGRA u32 → RGBA u8 pixels
            let mut pixels = [[0u8; 4]; 16];
            for (i, v) in px_arr.iter().enumerate().take(16) {
                let bgra = v.as_u64().unwrap() as u32;
                pixels[i] = [
                    ((bgra >> 16) & 0xFF) as u8, // R
                    ((bgra >> 8) & 0xFF) as u8,  // G
                    (bgra & 0xFF) as u8,          // B
                    ((bgra >> 24) & 0xFF) as u8,  // A
                ];
            }

            // Run our encoder
            let our_block = encode_dxt1_block(&pixels, false, true);
            let our_c0 = u16::from_le_bytes([our_block[0], our_block[1]]);
            let our_c1 = u16::from_le_bytes([our_block[2], our_block[3]]);
            let our_idx = u32::from_le_bytes([our_block[4], our_block[5], our_block[6], our_block[7]]);

            let full_match = our_c0 == ref_c0 && our_c1 == ref_c1 && our_idx == ref_idx;
            if full_match { match_full += 1; }
            if our_c0 == ref_c0 && our_c1 == ref_c1 { match_c0c1 += 1; }
            if our_c0 == ref_c0 { match_c0 += 1; }

            if !full_match && printed < 10 {
                printed += 1;
                eprintln!("  DIFF blk={}: ref=({:04X},{:04X},{:08X}) ours=({:04X},{:04X},{:08X})",
                    pair["bk"].as_u64().unwrap(),
                    ref_c0, ref_c1, ref_idx, our_c0, our_c1, our_idx);
                eprintln!("    px[0]=({},{},{}) px[15]=({},{},{})",
                    pixels[0][0], pixels[0][1], pixels[0][2],
                    pixels[15][0], pixels[15][1], pixels[15][2]);
            }
        }

        eprintln!("\n=== Captured Pixel Pairs: Encoder Comparison ===");
        eprintln!("  Solid (skipped): {}", solid);
        eprintln!("  Non-solid tested: {}", total);
        eprintln!("  Full match (c0+c1+idx): {}/{} ({:.1}%)", match_full, total,
            match_full as f64 / total.max(1) as f64 * 100.0);
        eprintln!("  c0+c1 match: {}/{} ({:.1}%)", match_c0c1, total,
            match_c0c1 as f64 / total.max(1) as f64 * 100.0);
        eprintln!("  c0 only match: {}/{} ({:.1}%)", match_c0, total,
            match_c0 as f64 / total.max(1) as f64 * 100.0);
    }

    #[test]
    #[ignore] // Run with: cargo test captured_pairs_regression -- --ignored --nocapture
    fn captured_pairs_regression() {
        use std::path::PathBuf;

        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let pairs_path = base.join("tools/captured_pixel_pairs.json");
        let data = std::fs::read_to_string(&pairs_path)
            .expect("Failed to read captured_pixel_pairs.json");
        let pairs: serde_json::Value = serde_json::from_str(&data)
            .expect("Failed to parse JSON");
        let arr = pairs.as_array().unwrap();

        let mut total = 0usize;
        let mut matched = 0usize;
        let mut solid_total = 0usize;
        let mut solid_matched = 0usize;
        let mut nonsolid_total = 0usize;
        let mut nonsolid_matched = 0usize;
        let mut first_mismatches: Vec<String> = Vec::new();

        for entry in arr {
            let px_arr = entry["px"].as_array().unwrap();
            let ref_c0 = entry["c0"].as_u64().unwrap() as u16;
            let ref_c1 = entry["c1"].as_u64().unwrap() as u16;
            let ref_ix = entry["ix"].as_u64().unwrap() as u32;

            // Convert BGRA u32 → RGBA u8 array
            let mut pixels = [[0u8; 4]; 16];
            for i in 0..16 {
                let bgra = px_arr[i].as_u64().unwrap() as u32;
                let b = (bgra & 0xFF) as u8;
                let g = ((bgra >> 8) & 0xFF) as u8;
                let r = ((bgra >> 16) & 0xFF) as u8;
                let a = ((bgra >> 24) & 0xFF) as u8;
                pixels[i] = [r, g, b, a];
            }

            let result = encode_dxt1_range_fit(&pixels);
            let our_c0 = u16::from_le_bytes([result[0], result[1]]);
            let our_c1 = u16::from_le_bytes([result[2], result[3]]);
            let our_ix = u32::from_le_bytes([result[4], result[5], result[6], result[7]]);

            let mut ref_block = [0u8; 8];
            ref_block[0..2].copy_from_slice(&ref_c0.to_le_bytes());
            ref_block[2..4].copy_from_slice(&ref_c1.to_le_bytes());
            ref_block[4..8].copy_from_slice(&ref_ix.to_le_bytes());

            let is_solid = ref_c0 == ref_c1;
            total += 1;
            if is_solid { solid_total += 1; } else { nonsolid_total += 1; }

            if result == ref_block {
                matched += 1;
                if is_solid { solid_matched += 1; } else { nonsolid_matched += 1; }
            } else if first_mismatches.len() < 10 {
                let tx = entry["tx"].as_u64().unwrap();
                let bk = entry["bk"].as_u64().unwrap();
                first_mismatches.push(format!(
                    "  tx={} bk={}: ref c0=0x{:04X} c1=0x{:04X} ix=0x{:08X} | ours c0=0x{:04X} c1=0x{:04X} ix=0x{:08X}",
                    tx, bk, ref_c0, ref_c1, ref_ix, our_c0, our_c1, our_ix
                ));
            }
        }

        eprintln!("\n=== Captured Pairs Regression ({} blocks) ===", total);
        eprintln!("  Overall:    {}/{} ({:.1}%)", matched, total, matched as f64 / total as f64 * 100.0);
        eprintln!("  Solid:      {}/{} ({:.1}%)", solid_matched, solid_total,
            if solid_total > 0 { solid_matched as f64 / solid_total as f64 * 100.0 } else { 0.0 });
        eprintln!("  Non-solid:  {}/{} ({:.1}%)", nonsolid_matched, nonsolid_total,
            if nonsolid_total > 0 { nonsolid_matched as f64 / nonsolid_total as f64 * 100.0 } else { 0.0 });
        if !first_mismatches.is_empty() {
            eprintln!("\n  First {} mismatches:", first_mismatches.len());
            for m in &first_mismatches { eprintln!("{}", m); }
        }
    }

}
