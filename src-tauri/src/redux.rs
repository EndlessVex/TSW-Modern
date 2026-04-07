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

    // Convert to linear float buffer for sRGB-correct filtering
    // Gamma correction disabled — testing showed it does not improve matching
    // against the game's output. The original may not apply
    // gamma correction during mip filtering.
    let use_srgb = false;
    let mut src_linear = vec![[0.0f32; 4]; prev_w * prev_h];
    for i in 0..src_pixels.len() {
        let p = src_pixels[i];
        if use_srgb {
            src_linear[i] = [
                srgb_to_linear(p[0]),
                srgb_to_linear(p[1]),
                srgb_to_linear(p[2]),
                p[3] as f32 / 255.0,
            ];
        } else {
            src_linear[i] = [
                p[0] as f32 / 255.0,
                p[1] as f32 / 255.0,
                p[2] as f32 / 255.0,
                p[3] as f32 / 255.0,
            ];
        }
    }

    // Step 2: Box-filter in linear float space, then convert back
    let mut dst_pixels = vec![[0u8; 4]; new_w * new_h];
    for y in 0..new_h {
        for x in 0..new_w {
            let sx = x * 2;
            let sy = y * 2;
            let x0 = sx.min(prev_w - 1);
            let y0 = sy.min(prev_h - 1);
            let x1 = (sx + 1).min(prev_w - 1);
            let y1 = (sy + 1).min(prev_h - 1);

            let p00 = src_linear[y0 * prev_w + x0];
            let p10 = src_linear[y0 * prev_w + x1];
            let p01 = src_linear[y1 * prev_w + x0];
            let p11 = src_linear[y1 * prev_w + x1];

            for c in 0..3 {
                let avg = (p00[c] + p10[c] + p01[c] + p11[c]) / 4.0;
                dst_pixels[y * new_w + x][c] = if use_srgb {
                    linear_to_srgb(avg)
                } else {
                    (avg * 255.0 + 0.5).clamp(0.0, 255.0) as u8
                };
            }
            // Alpha: always linear averaging
            let alpha_avg = (p00[3] + p10[3] + p01[3] + p11[3]) / 4.0;
            dst_pixels[y * new_w + x][3] = (alpha_avg * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
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
                TextureCodec::Dxt1 => result.extend_from_slice(&encode_dxt1_block(&block_pixels, force_3color)),
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
fn build_color_set(pixels: &[[u8; 4]; 16]) -> (Vec<[f32; 3]>, Vec<f32>, Vec<usize>, usize) {
    let mut colors: Vec<[f32; 3]> = Vec::with_capacity(16);
    let mut color_bytes: Vec<[u8; 3]> = Vec::with_capacity(16);
    let mut weights: Vec<f32> = Vec::with_capacity(16);
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

    let n = colors.len();

    // Weighted centroid
    let total_weight: f32 = weights.iter().sum();
    let mut centroid = [0.0f32; 3];
    for (c, &w) in colors.iter().zip(weights.iter()) {
        for k in 0..3 { centroid[k] += c[k] * w; }
    }
    for k in 0..3 { centroid[k] /= total_weight; }

    // Weighted covariance matrix [rr, rg, rb, gg, gb, bb]
    let mut cov = [0.0f32; 6];
    for (c, &w) in colors.iter().zip(weights.iter()) {
        let dr = c[0] - centroid[0];
        let dg = c[1] - centroid[1];
        let db = c[2] - centroid[2];
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
            let mag = row[0] * row[0] + row[1] * row[1] + row[2] * row[2];
            if mag > best_mag { best_mag = mag; best_row = i; }
        }
        rows[best_row]
    };
    for _ in 0..8 {
        let next = [
            cov[0] * axis[0] + cov[1] * axis[1] + cov[2] * axis[2],
            cov[1] * axis[0] + cov[3] * axis[1] + cov[4] * axis[2],
            cov[2] * axis[0] + cov[4] * axis[1] + cov[5] * axis[2],
        ];
        let max_abs = next[0].abs().max(next[1].abs()).max(next[2].abs());
        if max_abs > 0.0 {
            axis = [next[0] / max_abs, next[1] / max_abs, next[2] / max_abs];
        } else {
            axis = [0.0; 3];
            break;
        }
    }

    // Project colors onto axis and insertion-sort ascending
    let mut order: Vec<usize> = (0..n).collect();
    let mut dots: Vec<f32> = colors.iter().map(|c| {
        c[0] * axis[0] + c[1] * axis[1] + c[2] * axis[2]
    }).collect();
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
fn cluster_fit_4color(pixels: &[[u8; 4]; 16]) -> ([u8; 8], f32) {
    let (colors, weights, _order, n) = build_color_set(pixels);

    // Precompute cumulative sums for partition search
    let mut cum_w = vec![0.0f32; n + 1];
    let mut cum_rgb = vec![[0.0f32; 3]; n + 1];
    for i in 0..n {
        cum_w[i + 1] = cum_w[i] + weights[i];
        for k in 0..3 {
            cum_rgb[i + 1][k] = cum_rgb[i][k] + colors[i][k] * weights[i];
        }
    }
    let total_rgb = cum_rgb[n];

    // Exhaustive 4-partition search
    let mut best_err = f32::MAX;
    let mut best_ep0 = [0.0f32; 3];
    let mut best_ep1 = [0.0f32; 3];

    for s in 0..=n {
        let w_a = cum_w[s];
        for t in s..=n {
            let w_b = cum_w[t] - cum_w[s];
            for u in t..=n {
                let w_c = cum_w[u] - cum_w[t];
                let w_d = cum_w[n] - cum_w[u];

                let alpha_aa = w_a + w_b * (4.0f32 / 9.0) + w_c * (1.0f32 / 9.0);
                let alpha_bb = w_d + w_c * (4.0f32 / 9.0) + w_b * (1.0f32 / 9.0);
                let alpha_ab = (w_b + w_c) * (2.0f32 / 9.0);
                let det = alpha_aa * alpha_bb - alpha_ab * alpha_ab;
                if det.abs() < 1e-6 { continue; }
                let inv_det: f32 = 1.0 / det;

                let sum_a = cum_rgb[s];
                let sum_b = [cum_rgb[t][0] - cum_rgb[s][0], cum_rgb[t][1] - cum_rgb[s][1], cum_rgb[t][2] - cum_rgb[s][2]];
                let sum_c = [cum_rgb[u][0] - cum_rgb[t][0], cum_rgb[u][1] - cum_rgb[t][1], cum_rgb[u][2] - cum_rgb[t][2]];
                let sum_d = [total_rgb[0] - cum_rgb[u][0], total_rgb[1] - cum_rgb[u][1], total_rgb[2] - cum_rgb[u][2]];

                let mut ep_a = [0.0f32; 3];
                let mut ep_b = [0.0f32; 3];
                for k in 0..3 {
                    let beta_a = sum_a[k] + sum_b[k] * (2.0f32 / 3.0) + sum_c[k] * (1.0f32 / 3.0);
                    let beta_b = sum_d[k] + sum_c[k] * (2.0f32 / 3.0) + sum_b[k] * (1.0f32 / 3.0);
                    let a_k = (beta_a * alpha_bb - beta_b * alpha_ab) * inv_det;
                    let b_k = (beta_b * alpha_aa - beta_a * alpha_ab) * inv_det;
                    ep_a[k] = a_k.clamp(0.0, 1.0);
                    ep_b[k] = b_k.clamp(0.0, 1.0);
                }

                // Grid-snap: quantize to 5/6/5 then dequantize back
                let grid = [31.0f32, 63.0, 31.0];
                let inv_grid = [1.0f32 / 31.0, 1.0 / 63.0, 1.0 / 31.0];
                for k in 0..3 {
                    ep_a[k] = (ep_a[k] * grid[k] + 0.5).floor() * inv_grid[k];
                    ep_b[k] = (ep_b[k] * grid[k] + 0.5).floor() * inv_grid[k];
                }

                // Per-channel error metric weights (squared).
                // The original stores these at object offsets 0x124/0x128/0x12c.
                let metric = [1.0f32, 1.0, 1.0];

                let mut err = 0.0f32;
                for k in 0..3 {
                    let beta_a = sum_a[k] + sum_b[k] * (2.0f32 / 3.0) + sum_c[k] * (1.0f32 / 3.0);
                    let beta_b = sum_d[k] + sum_c[k] * (2.0f32 / 3.0) + sum_b[k] * (1.0f32 / 3.0);
                    err += (alpha_aa * ep_a[k] * ep_a[k]
                        + alpha_bb * ep_b[k] * ep_b[k]
                        + 2.0 * alpha_ab * ep_a[k] * ep_b[k]
                        - 2.0 * (beta_a * ep_a[k] + beta_b * ep_b[k])) * metric[k];
                }
                if err < best_err {
                    best_err = err;
                    best_ep0 = ep_a;
                    best_ep1 = ep_b;
                }
            }
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

    // Assign closest palette indices using the ORIGINAL endpoint order
    // (before any swap for 4-color mode enforcement).
    let (dr0, dg0, db0) = decode_rgb565(c0);
    let (dr1, dg1, db1) = decode_rgb565(c1);
    let palette: [(i16, i16, i16); 4] = [
        (dr0 as i16, dg0 as i16, db0 as i16),
        (dr1 as i16, dg1 as i16, db1 as i16),
        ((2 * dr0 as i16 + dr1 as i16 + 1) / 3, (2 * dg0 as i16 + dg1 as i16 + 1) / 3, (2 * db0 as i16 + db1 as i16 + 1) / 3),
        ((dr0 as i16 + 2 * dr1 as i16 + 1) / 3, (dg0 as i16 + 2 * dg1 as i16 + 1) / 3, (db0 as i16 + 2 * db1 as i16 + 1) / 3),
    ];

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
        return (out, err);
    }

    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    let err = compute_4color_error(pixels, c0, c1);
    (out, err)
}

/// 3-color ClusterFit encoder.
/// Returns (encoded_block, weighted_error).
#[allow(dead_code)]
fn cluster_fit_3color(pixels: &[[u8; 4]; 16]) -> ([u8; 8], f32) {
    let (sorted_colors, sorted_weights, _order, n) = build_color_set(pixels);

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
fn encode_dxt1_block(pixels: &[[u8; 4]; 16], _force_3color: bool) -> [u8; 8] {
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

    // Solid-color fast path: check if all pixels share the same RGB.
    let first_rgb = (pixels[0][0], pixels[0][1], pixels[0][2]);
    let all_solid = pixels.iter().all(|p| (p[0], p[1], p[2]) == first_rgb);
    if all_solid {
        return encode_dxt1_solid(first_rgb.0, first_rgb.1, first_rgb.2);
    }

    // Use 4-color mode only for opaque blocks.
    // The original encoder does NOT try 3-color for opaque
    // blocks despite the reference encoder doing so. Trying 3-color causes
    // massive mode-change regressions (1.1% -> 25.8% of blocks).
    let (block_4, _) = cluster_fit_4color(pixels);
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
    let color_block = encode_dxt1_block(pixels, false);

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
        let re_encoded = encode_dxt1_block(&pixels, false);
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
        let encoded = encode_dxt1_block(&pixels, false);
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

        let re_encoded = encode_dxt1_block(&pixels, false);
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

        let encoded = encode_dxt1_block(&pixels, false);
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
        let encoded = encode_dxt1_block(&pixels, false);
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
        let (block, _err) = cluster_fit_4color(&gradient);
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
        let (block2, _) = cluster_fit_4color(&green_grad);
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
        let (block3, _) = cluster_fit_4color(&noise);
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
                        if matched { nonsolid_match += 1; }
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
        let (our_encoded, _) = cluster_fit_4color(&filtered);
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
            let our_encoded = encode_dxt1_block(&pixels, false);

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

}
