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

/// DXT1 solid-block lookup tables extracted from ClientPatcher.exe via Ghidra.
/// VA 0x873F08 (5-bit R/B channels) and VA 0x874108 (6-bit G channel).
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

    // ── 5. Generate all mips, then write SMALLEST-FIRST ────────────
    // The FCTX format stores mips smallest-first (1x1 at start, full mip0 at end).
    let mut all_mips: Vec<Vec<u8>> = vec![mip0_data[..mip_sizes[0]].to_vec()];
    let mut current_w = stream1.width;
    let mut current_h = stream1.height;

    for mip_idx in 1..mip_count {
        let new_w = (current_w / 2).max(4);
        let new_h = (current_h / 2).max(4);

        let mip_data = generate_mip(
            &all_mips[mip_idx - 1], current_w, current_h, new_w, new_h, codec,
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
            let mip = generate_mip(&ati2_mips[mip_idx - 1], cw, ch, nw, nh, TextureCodec::Ati2);
            ati2_mips.push(mip);
            cw = nw;
            ch = nh;
        }

        // Assemble per-mip grouped output (Ghidra-verified layout, same as format_enum=6):
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

    // -- Assemble output in per-mip grouped layout --
    // Verified via Ghidra decompilation of the game's FCTX MIXD reader (FUN_00c63360):
    // the game reads per-mip, NOT as global planes. Each mip has its prefix section
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

/// Encode a solid-color DXT1 block using Ghidra-verified lookup tables.
/// Ghidra: FUN_006807F0. Uses precomputed optimal RGB565 endpoint pairs.
/// Selector bits are always 0xAAAAAAAA (all texels select palette entry 2).
/// If c0 < c1, swap endpoints and use selector 0x55555555.
fn encode_dxt1_solid(r: u8, g: u8, b: u8) -> [u8; 8] {
    let [r0, r1] = DXT1_SOLID_5BIT[r as usize];
    let [g0, g1] = DXT1_SOLID_6BIT[g as usize];
    let [b0, b1] = DXT1_SOLID_5BIT[b as usize];

    let c0 = ((r0 as u16) << 11) | ((g0 as u16) << 5) | (b0 as u16);
    let c1 = ((r1 as u16) << 11) | ((g1 as u16) << 5) | (b1 as u16);

    let mut block = [0u8; 8];
    if c0 < c1 {
        block[0..2].copy_from_slice(&c1.to_le_bytes());
        block[2..4].copy_from_slice(&c0.to_le_bytes());
        block[4..8].copy_from_slice(&0x55555555u32.to_le_bytes());
    } else {
        block[0..2].copy_from_slice(&c0.to_le_bytes());
        block[2..4].copy_from_slice(&c1.to_le_bytes());
        block[4..8].copy_from_slice(&0xAAAAAAAAu32.to_le_bytes());
    }
    block
}

/// Encode 16 RGBA pixels → DXT1 block (8 bytes).
///
/// Uses bounding-box endpoint selection for speed. Quality is sufficient for
/// mipmap generation where the source is already DXT-compressed.
fn encode_dxt1_block(pixels: &[[u8; 4]; 16]) -> [u8; 8] {
    // Solid-color fast path: if all 16 pixels have the same RGB, use lookup tables.
    // Ghidra: FUN_0067B5A0 checks solid, FUN_006807F0 encodes.
    let first_rgb = (pixels[0][0], pixels[0][1], pixels[0][2]);
    let all_solid = pixels.iter().all(|p| (p[0], p[1], p[2]) == first_rgb);
    if all_solid {
        return encode_dxt1_solid(first_rgb.0, first_rgb.1, first_rgb.2);
    }

    // ── WeightedClusterFit (Ghidra: FUN_0067F490 + FUN_0067E280) ──

    // Step 1: Build color set — float RGB, alpha-weighted, deduplicated
    let mut colors: Vec<[f32; 3]> = Vec::with_capacity(16);
    let mut weights: Vec<f32> = Vec::with_capacity(16);
    let mut remap = [0usize; 16];

    for (i, p) in pixels.iter().enumerate() {
        let rgb = [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0];
        let w = (p[3] as f32 + 1.0) / 256.0;
        let mut found = false;
        for (j, existing) in colors.iter().enumerate() {
            if (existing[0] - rgb[0]).abs() < 1e-6
                && (existing[1] - rgb[1]).abs() < 1e-6
                && (existing[2] - rgb[2]).abs() < 1e-6
            {
                weights[j] += w;
                remap[i] = j;
                found = true;
                break;
            }
        }
        if !found {
            remap[i] = colors.len();
            colors.push(rgb);
            weights.push(w);
        }
    }

    let n = colors.len();

    // Step 2: Weighted centroid
    let total_weight: f32 = weights.iter().sum();
    let mut centroid = [0.0f32; 3];
    for (c, &w) in colors.iter().zip(weights.iter()) {
        for k in 0..3 { centroid[k] += c[k] * w; }
    }
    for k in 0..3 { centroid[k] /= total_weight; }

    // Step 3: Weighted covariance matrix [rr, rg, rb, gg, gb, bb]
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

    // Step 4: Power iteration — 8 iterations, seed = max-magnitude row
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

    // Step 5: Project colors onto axis and insertion-sort ascending
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

    // Step 6: Precompute cumulative sums for partition search
    let mut cum_w = vec![0.0f32; n + 1];
    let mut cum_rgb = vec![[0.0f32; 3]; n + 1];
    for i in 0..n {
        cum_w[i + 1] = cum_w[i] + sorted_weights[i];
        for k in 0..3 {
            cum_rgb[i + 1][k] = cum_rgb[i][k] + sorted_colors[i][k] * sorted_weights[i];
        }
    }
    let total_rgb = cum_rgb[n];

    // Step 7: Exhaustive 4-partition search
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

                let alpha_aa = w_a + w_b * (4.0 / 9.0) + w_c * (1.0 / 9.0);
                let alpha_bb = w_d + w_c * (4.0 / 9.0) + w_b * (1.0 / 9.0);
                let alpha_ab = (w_b + w_c) * (2.0 / 9.0);
                let det = alpha_aa * alpha_bb - alpha_ab * alpha_ab;
                if det.abs() < 1e-10 { continue; }
                let inv_det = 1.0 / det;

                let sum_a = cum_rgb[s];
                let sum_b = [cum_rgb[t][0] - cum_rgb[s][0], cum_rgb[t][1] - cum_rgb[s][1], cum_rgb[t][2] - cum_rgb[s][2]];
                let sum_c = [cum_rgb[u][0] - cum_rgb[t][0], cum_rgb[u][1] - cum_rgb[t][1], cum_rgb[u][2] - cum_rgb[t][2]];
                let sum_d = [total_rgb[0] - cum_rgb[u][0], total_rgb[1] - cum_rgb[u][1], total_rgb[2] - cum_rgb[u][2]];

                let mut err = 0.0f32;
                let mut ep_a = [0.0f32; 3];
                let mut ep_b = [0.0f32; 3];
                for k in 0..3 {
                    let beta_a = sum_a[k] + sum_b[k] * (2.0 / 3.0) + sum_c[k] * (1.0 / 3.0);
                    let beta_b = sum_d[k] + sum_c[k] * (2.0 / 3.0) + sum_b[k] * (1.0 / 3.0);
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
    }

    // Step 8: Quantize to RGB565
    let r0 = (best_ep0[0] * 31.0 + 0.5).clamp(0.0, 31.0) as u8;
    let g0 = (best_ep0[1] * 63.0 + 0.5).clamp(0.0, 63.0) as u8;
    let b0 = (best_ep0[2] * 31.0 + 0.5).clamp(0.0, 31.0) as u8;
    let r1 = (best_ep1[0] * 31.0 + 0.5).clamp(0.0, 31.0) as u8;
    let g1 = (best_ep1[1] * 63.0 + 0.5).clamp(0.0, 63.0) as u8;
    let b1 = (best_ep1[2] * 31.0 + 0.5).clamp(0.0, 31.0) as u8;

    let mut c0 = ((r0 as u16) << 11) | ((g0 as u16) << 5) | (b0 as u16);
    let mut c1 = ((r1 as u16) << 11) | ((g1 as u16) << 5) | (b1 as u16);

    if c0 < c1 { std::mem::swap(&mut c0, &mut c1); }
    if c0 == c1 {
        let mut out = [0u8; 8];
        out[0..2].copy_from_slice(&c0.to_le_bytes());
        out[2..4].copy_from_slice(&c1.to_le_bytes());
        return out;
    }

    // Step 9: Assign closest palette indices
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
            ((6 * a0w + 1 * a1w) / 7) as u8,
            ((5 * a0w + 2 * a1w) / 7) as u8,
            ((4 * a0w + 3 * a1w) / 7) as u8,
            ((3 * a0w + 4 * a1w) / 7) as u8,
            ((2 * a0w + 5 * a1w) / 7) as u8,
            ((1 * a0w + 6 * a1w) / 7) as u8,
        ]
    } else if a0 == a1 {
        [a0; 8]
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

    // Find closest alpha index for each pixel (strict < for game-compatible tie-breaking)
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

    // Use truncating division (no +3 rounding bias) to match the game engine's
    // BC4 interpolation. Verified: this produces byte-identical output for ~53%
    // of generated mip blocks vs the ClientPatcher, up from ~9% with standard rounding.
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
/// Ghidra-verified: FUN_006801F0 — squared-error distance, per-pixel min across palette.
fn bc4_sse(values: &[u8; 16], a0: u8, a1: u8) -> u32 {
    let a0w = a0 as u16;
    let a1w = a1 as u16;
    // Build palette with truncating division (Ghidra-verified FUN_0067B0E0)
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

    // Ghidra-verified exhaustive endpoint search (FUN_00680B60):
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
                // Ghidra pruning: skip pairs that can't beat current best
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

    // Build palette with truncating division (Ghidra-verified FUN_0067B0E0)
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
    // Ghidra-verified: FUN_006802B0
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
/// Ghidra: FUN_0067C680. Used for ATI2/BC5 channels (normal maps).
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

    // Initial endpoints: inset by range/34 (Ghidra: divisor 0x22)
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
/// Ghidra: FUN_0067BBB0.
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
/// Ghidra: FUN_0067C460.
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
        let re_encoded = encode_dxt1_block(&pixels);
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
        let encoded = encode_dxt1_block(&pixels);
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

}
