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
    let mut all_mips: Vec<Vec<u8>> = vec![mip0_data[..mip_sizes[0]].to_vec()];
    let mut current_w = stream1.width;
    let mut current_h = stream1.height;

    for mip_idx in 1..mip_count {
        let new_w = (current_w / 2).max(4);
        let new_h = (current_h / 2).max(4);

        let mip_data = generate_mip(
            &all_mips[mip_idx - 1], current_w, current_h, new_w, new_h, codec,
            has_binary_alpha,
        );

        if mip_data.len() != mip_sizes[mip_idx] {
            return Err(format!(
                "Mip {} size mismatch: generated {} bytes, expected {}",
                mip_idx,
                mip_data.len(),
                mip_sizes[mip_idx]
            ));
        }

        all_mips.push(mip_data);
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
            let nw = (cw / 2).max(4);
            let nh = (ch / 2).max(4);
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
        let nw = (cw / 2).max(4);
        let nh = (ch / 2).max(4);
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
        let nw = (cw / 2).max(4);
        let nh = (ch / 2).max(4);
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

    // Step 2: Box-filter — float averaging with truncation (no +0.5 rounding).
    // Testing showed truncating division matches the original better than
    // rounding. The original likely uses float averaging with truncating
    // conversion back to u8.
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
///
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
    // Bit-replication expansion matching the original's alg_nv library.
    // Empirically verified: shift+trunc gives 100% solid block match.
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
    // The original uses alg_nv (NVIDIA texture tools) which omits the
    // Microsoft +1 in the interpolation formula. This shifts interpolated
    // colors by 1 unit for many values, cascading through mip generation.
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
    if dedup {
        let mut color_bytes: Vec<[u8; 3]> = Vec::with_capacity(16);
        for (_i, p) in pixels.iter().enumerate() {
            let rgb_bytes = [p[0], p[1], p[2]];
            let rgb = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
            let w = (p[3] as f32 + 1.0) / 256.0;
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
            colors.push([p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]);
            weights.push((p[3] as f32 + 1.0) / 256.0);
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

    // Weighted covariance matrix [rr, rg, rb, gg, gb, bb]
    // Each element is accumulated via x87: w*dr*dr stays at 80-bit, stored to f32
    let mut cov = [0.0f32; 6];
    for (c, &w) in colors.iter().zip(weights.iter()) {
        let dr = c[0] - centroid[0];
        let dg = c[1] - centroid[1];
        let db = c[2] - centroid[2];
        // x87 fma: w*dr*dr at 80-bit, then add to accumulator
        cov[0] += x87_fma_f32(dr, dr, 0.0) * w;
        cov[1] += x87_fma_f32(dr, dg, 0.0) * w;
        cov[2] += x87_fma_f32(dr, db, 0.0) * w;
        cov[3] += x87_fma_f32(dg, dg, 0.0) * w;
        cov[4] += x87_fma_f32(dg, db, 0.0) * w;
        cov[5] += x87_fma_f32(db, db, 0.0) * w;
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

                // Determinant at x87 80-bit — critical for avoiding catastrophic cancellation
                let det = x87_cross_f32(alpha_aa, alpha_bb, alpha_ab, alpha_ab);

                if det.abs() >= f32::MIN_POSITIVE {
                    let inv_det = x87_rcp_f32(det);

                    let mut ep_a = [0.0f32; 3];
                    let mut ep_b = [0.0f32; 3];
                    for k in 0..3 {
                        // beta_a = mid*2/3 + outer + inner*1/3
                        // Critical: mid*2/3 + outer must stay at 80-bit (not truncated to
                        // f32) before adding inner*1/3. Use x87_add_fma_f32 which keeps
                        // the intermediate sum on the FPU stack.
                        let beta_a = x87_add_fma_f32(beta_a_mid[k], outer_rgb[k], inner_rgb[k], c_1_3);
                        let beta_b = x87_sub_f32(total_rgb[k], beta_a);

                        // Endpoint solve: (beta_a*alpha_bb - beta_b*alpha_ab) * inv_det
                        let a_k = x87_cross_mul_f32(beta_a, alpha_bb, beta_b, alpha_ab, inv_det);
                        let b_k = x87_cross_mul_f32(beta_b, alpha_aa, beta_a, alpha_ab, inv_det);
                        ep_a[k] = a_k.clamp(0.0, 1.0);
                        ep_b[k] = b_k.clamp(0.0, 1.0);
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

                    // Error computation
                    let mut err = 0.0f32;
                    for k in 0..3 {
                        let beta_a = x87_add_fma_f32(beta_a_mid[k], outer_rgb[k], inner_rgb[k], c_1_3);
                        let beta_b = x87_sub_f32(total_rgb[k], beta_a);

                        // aa*ea^2 + bb*eb^2 + 2*ab*ea*eb - 2*(ba*ea + bb_val*eb)
                        let t1 = alpha_aa * ep_a[k] * ep_a[k];
                        let t2 = alpha_bb * ep_b[k] * ep_b[k];
                        let t3 = c_2 * alpha_ab * ep_a[k] * ep_b[k];
                        let t4 = c_2 * (beta_a * ep_a[k] + beta_b * ep_b[k]);
                        let ch_err = t1 + t2 + t3 - t4;
                        err += ch_err;
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

    if !for_mip {
        // Solid-color fast path (non-mip encoding only).
        // For mip generation, the original sends ALL blocks through ClusterFit
        // (even solid ones) because the monochrome check rarely triggers after
        // box filtering. ClusterFit naturally produces c0=c1 for uniform blocks.
        let first_rgb = (pixels[0][0], pixels[0][1], pixels[0][2]);
        let all_solid = pixels.iter().all(|p| (p[0], p[1], p[2]) == first_rgb);
        if all_solid {
            return encode_dxt1_solid(first_rgb.0, first_rgb.1, first_rgb.2);
        }
    }

    let dedup = !for_mip;
    let (block_4, _) = cluster_fit_4color(pixels, dedup);
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

        let fctx_header_size = 40usize;
        let block_size = 8usize; // DXT1

        let mut total_blocks = 0usize;
        let mut matching_blocks = 0usize;

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
                if w <= 4 && h <= 4 { break; }
                w = (w / 2).max(4);
                h = (h / 2).max(4);
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

            // Regenerate mips 1..N using our encoder
            let mut current_data = mip0.to_vec();
            let mut current_w = width;
            let mut current_h = height;

            let mut tex_total = 0usize;
            let mut tex_match = 0usize;

            for mip_idx in 1..mip_count {
                let new_w = (current_w / 2).max(4);
                let new_h = (current_h / 2).max(4);

                let our_mip = generate_mip(
                    &current_data, current_w, current_h, new_w, new_h,
                    TextureCodec::Dxt1, false,
                );

                let ref_mip = ref_mips[mip_idx];
                assert_eq!(our_mip.len(), ref_mip.len(),
                    "ID {} mip{}: size mismatch {} vs {}", id, mip_idx, our_mip.len(), ref_mip.len());

                // Compare block by block
                let num_blocks = our_mip.len() / block_size;
                let mut mip_match = 0usize;
                for b in 0..num_blocks {
                    let our_block = &our_mip[b * block_size..(b + 1) * block_size];
                    let ref_block = &ref_mip[b * block_size..(b + 1) * block_size];
                    if our_block == ref_block {
                        mip_match += 1;
                    }
                }

                eprintln!("  ID {} mip{} ({}x{}): {}/{} blocks match ({:.1}%)",
                    id, mip_idx, new_w, new_h, mip_match, num_blocks,
                    (mip_match as f64 / num_blocks as f64) * 100.0);

                tex_total += num_blocks;
                tex_match += mip_match;

                // Use OUR generated mip as input for next level
                // (matching what the pipeline does -- each mip is generated from the previous)
                current_data = our_mip;
                current_w = new_w;
                current_h = new_h;
            }

            eprintln!("  ID {} TOTAL: {}/{} blocks ({:.1}%)\n",
                id, tex_match, tex_total, (tex_match as f64 / tex_total as f64) * 100.0);
            total_blocks += tex_total;
            matching_blocks += tex_match;
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
                if w <= 4 && h <= 4 { break; }
                w = (w / 2).max(4);
                h = (h / 2).max(4);
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

            // Regenerate mips and compare, tracking solid vs non-solid
            let mut current_data = ref_mips[0].to_vec();
            let mut current_w = width;
            let mut current_h = height;

            for mip_idx in 1..mip_count {
                let new_w = (current_w / 2).max(4);
                let new_h = (current_h / 2).max(4);

                let our_mip = generate_mip(
                    &current_data, current_w, current_h, new_w, new_h,
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

                current_data = our_mip;
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

        eprintln!("  OVERALL: {}/{} blocks match ({:.2}%)",
            matching_blocks, total_blocks,
            (matching_blocks as f64 / total_blocks as f64) * 100.0);
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
            w = (w / 2).max(4);
            h = (h / 2).max(4);
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
            w = (w/2).max(4); h = (h/2).max(4);
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

}
