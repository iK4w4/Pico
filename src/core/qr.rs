//! Pure-Rust QR encoder for Phone Teleport (RESEARCH_v25.md §a).
//!
//! Profile: QR Code Model 2, a single **byte-mode** segment,
//! error-correction level **M**, automatic smallest-version
//! selection over versions **1–12**. Input longer than the v12-M
//! byte capacity (287 bytes) is an error, never a truncated
//! symbol — a QR that scans wrong is worse than none.
//!
//! The encoder is written from the specification's construction:
//! GF(256) Reed–Solomon (primitive polynomial 0x11D, generator 2),
//! the standard block-split/interleave arithmetic, format and
//! version information computed through their BCH codes (0x537 /
//! 0x1F25 generators, format mask 0x5412), and mask choice by the
//! specification's four penalty rules (N1=3, N2=3, N3=40, N4=10).
//! Its outputs are pinned module-for-module by golden vectors
//! generated with an independent reference implementation — see
//! `tests/qr_vectors.inc` and RESEARCH_v25.md §a. No crate: the
//! dependency tree gains nothing for this feature.

/// Largest version this encoder emits (design target 1–12).
pub const MAX_VERSION: usize = 12;

/// Byte capacity of the largest supported symbol (v12, level M).
pub const MAX_PAYLOAD_BYTES: usize = 287;

/// ECC codewords per block, level M, indexed by version (index 0
/// is a placeholder — versions start at 1). Cross-checked against
/// the reference tables in RESEARCH_v25.md §a.
const ECC_PER_BLOCK_M: [usize; 13] = [0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22];

/// Number of ECC blocks, level M, indexed by version.
const NUM_BLOCKS_M: [usize; 13] = [0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrError {
    /// The payload does not fit any version ≤ 12 at level M.
    TooLong { bytes: usize },
    /// Empty payloads are not encodable (and never useful).
    Empty,
}

/// A finished QR symbol: `modules[y][x]` (row-major), `true` =
/// dark. The quiet zone is NOT included; renderers add it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrMatrix {
    pub version: usize,
    pub mask: u8,
    pub size: usize,
    pub modules: Vec<Vec<bool>>,
}

impl QrMatrix {
    /// Rows as `"1"`/`"0"` strings (dark = `1`), top row first —
    /// the exact shape the teleport snapshot carries and the
    /// golden-vector tests compare against.
    pub fn rows(&self) -> Vec<String> {
        self.modules
            .iter()
            .map(|row| row.iter().map(|&m| if m { '1' } else { '0' }).collect())
            .collect()
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        self.modules
            .get(y)
            .and_then(|row| row.get(x))
            .copied()
            .unwrap_or(false)
    }
}

/// Total codewords (data + ECC) in a symbol of `version`: the
/// raw module count formula, floored to whole codewords.
fn raw_codewords(version: usize) -> usize {
    let mut result = (16 * version + 128) * version + 64;
    if version >= 2 {
        let num_align = version / 7 + 2;
        result -= (25 * num_align - 10) * num_align - 55;
        if version >= 7 {
            result -= 36;
        }
    }
    result / 8
}

/// Data codewords available at level M for `version`.
fn data_codewords(version: usize) -> usize {
    raw_codewords(version) - ECC_PER_BLOCK_M[version] * NUM_BLOCKS_M[version]
}

/// Character-count indicator width for byte mode: 8 bits through
/// version 9, 16 bits from version 10.
fn cci_bits(version: usize) -> usize {
    if version <= 9 {
        8
    } else {
        16
    }
}

/// Remainder bits appended after the interleaved codewords.
fn remainder_bits(version: usize) -> usize {
    match version {
        2..=6 => 7,
        _ => 0,
    }
}

/// Alignment-pattern center coordinates for `version` (the
/// standard step computation; v1 has none).
fn alignment_positions(version: usize, size: usize) -> Vec<usize> {
    if version == 1 {
        return Vec::new();
    }
    let num_align = version / 7 + 2;
    let step = (version * 8 + num_align * 3 + 5) / (num_align * 4 - 4) * 2;
    let mut result: Vec<usize> = (0..num_align - 1).map(|i| size - 7 - i * step).collect();
    result.push(6);
    result.reverse();
    result
}

// ---------------------------------------------------------------------------
// GF(256) arithmetic and Reed–Solomon
// ---------------------------------------------------------------------------

/// Multiply in GF(256) modulo 0x11D (shift-and-reduce; no tables
/// are materialized for this cold path). Bits of `y` are consumed
/// most-significant first; `z` stays a field element throughout
/// because the reduction clears bit 8 whenever it appears.
fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u16 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ if z & 0x80 != 0 { 0x11D } else { 0 };
        if (y >> i) & 1 != 0 {
            z ^= x as u16;
        }
    }
    debug_assert!(z >> 8 == 0);
    z as u8
}

/// The RS generator divisor of `degree`: coefficients of
/// ∏(x − α^i) for i in 0..degree, highest degree first, leading
/// 1 omitted — the form synthetic division consumes.
fn rs_divisor(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree];
    if degree == 0 {
        return result;
    }
    result[degree - 1] = 1; // Start with the polynomial x^0 = 1.
    let mut root: u8 = 1; // α^0
    for _ in 0..degree {
        for j in 0..degree {
            result[j] = gf_mul(result[j], root);
            if j + 1 < degree {
                result[j] ^= result[j + 1];
            }
        }
        root = gf_mul(root, 2); // Next power of α.
    }
    result
}

/// The ECC remainder of `data` under `divisor` (synthetic
/// division of data·x^degree).
fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; divisor.len()];
    for &b in data {
        let factor = b ^ result.remove(0);
        result.push(0);
        for (i, &coef) in divisor.iter().enumerate() {
            result[i] ^= gf_mul(coef, factor);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Bit packing and block interleaving
// ---------------------------------------------------------------------------

/// MSB-first bit accumulator.
#[derive(Default)]
struct BitBuf {
    bits: Vec<bool>,
}

impl BitBuf {
    fn push(&mut self, value: u32, len: usize) {
        for i in (0..len).rev() {
            self.bits.push((value >> i) & 1 != 0);
        }
    }
    fn push_bytes(&mut self, data: &[u8]) {
        for &b in data {
            self.push(b as u32, 8);
        }
    }
    fn len(&self) -> usize {
        self.bits.len()
    }
    fn into_bytes(self) -> Vec<u8> {
        debug_assert_eq!(self.bits.len() % 8, 0);
        self.bits
            .chunks(8)
            .map(|c| c.iter().fold(0u8, |acc, &b| (acc << 1) | b as u8))
            .collect()
    }
}

/// Assemble the data codewords for `data` at `version`: mode +
/// count + payload + terminator + byte alignment + pad bytes.
fn build_data_codewords(data: &[u8], version: usize) -> Vec<u8> {
    let capacity_bits = data_codewords(version) * 8;
    let mut buf = BitBuf::default();
    buf.push(0b0100, 4); // Byte mode.
    buf.push(data.len() as u32, cci_bits(version));
    buf.push_bytes(data);
    // Terminator: up to 4 zero bits, limited by capacity.
    let terminator = (capacity_bits - buf.len()).min(4);
    buf.push(0, terminator);
    // Pad to a byte boundary.
    while buf.len() % 8 != 0 {
        buf.push(0, 1);
    }
    // Alternate the two pad bytes to fill the capacity.
    let pads = [0xECu8, 0x11u8];
    let mut k = 0;
    while buf.len() < capacity_bits {
        buf.push(pads[k % 2] as u32, 8);
        k += 1;
    }
    buf.into_bytes()
}

/// Split data codewords into blocks, compute each block's ECC,
/// and interleave (data column-wise, then ECC column-wise) into
/// the final codeword stream.
fn add_ecc_and_interleave(data: &[u8], version: usize) -> Vec<u8> {
    let ecc_len = ECC_PER_BLOCK_M[version];
    let num_blocks = NUM_BLOCKS_M[version];
    let raw = raw_codewords(version);
    let num_short = num_blocks - raw % num_blocks;
    let short_block_len = raw / num_blocks; // Total cw per short block.
    let divisor = rs_divisor(ecc_len);

    // Split: the first `num_short` blocks are short (one fewer
    // data codeword); the rest are long.
    let mut blocks: Vec<Vec<u8>> = Vec::with_capacity(num_blocks);
    let mut offset = 0;
    for i in 0..num_blocks {
        let dat_len = short_block_len - ecc_len + usize::from(i >= num_short);
        let dat = &data[offset..offset + dat_len];
        offset += dat_len;
        let ecc = rs_remainder(dat, &divisor);
        let mut block = dat.to_vec();
        if i < num_short {
            block.push(0); // Phantom byte aligning short blocks for interleaving.
        }
        block.extend_from_slice(&ecc);
        blocks.push(block);
    }
    debug_assert_eq!(offset, data.len());

    let mut result = Vec::with_capacity(raw);
    for i in 0..blocks[0].len() {
        for (j, block) in blocks.iter().enumerate() {
            // Skip the phantom byte position in short blocks.
            if i != short_block_len - ecc_len || j >= num_short {
                result.push(block[i]);
            }
        }
    }
    debug_assert_eq!(result.len(), raw);
    result
}

// ---------------------------------------------------------------------------
// Matrix assembly
// ---------------------------------------------------------------------------

struct Canvas {
    size: usize,
    modules: Vec<Vec<bool>>,
    is_function: Vec<Vec<bool>>,
}

impl Canvas {
    fn new(version: usize) -> Canvas {
        let size = version * 4 + 17;
        Canvas {
            size,
            modules: vec![vec![false; size]; size],
            is_function: vec![vec![false; size]; size],
        }
    }

    fn set_function(&mut self, x: i32, y: i32, dark: bool) {
        if x >= 0 && y >= 0 && (x as usize) < self.size && (y as usize) < self.size {
            self.modules[y as usize][x as usize] = dark;
            self.is_function[y as usize][x as usize] = true;
        }
    }

    fn draw_finder(&mut self, cx: i32, cy: i32) {
        for dy in -4..=4i32 {
            for dx in -4..=4i32 {
                let dist = dx.abs().max(dy.abs());
                self.set_function(cx + dx, cy + dy, dist != 2 && dist != 4);
            }
        }
    }

    fn draw_alignment(&mut self, cx: usize, cy: usize) {
        for dy in -2..=2i32 {
            for dx in -2..=2i32 {
                let dist = dx.abs().max(dy.abs());
                self.set_function(cx as i32 + dx, cy as i32 + dy, dist != 1);
            }
        }
    }

    /// 15 format bits for (level M, `mask`), computed through
    /// the (15,5) BCH code with generator 0x537 and XOR mask
    /// 0x5412. Level M's format indicator bits are 0b00.
    fn format_bits(mask: u8) -> u32 {
        let data: u32 = (mask as u32) & 0x7; // (0b00 << 3) | mask
        let mut rem = data;
        for _ in 0..10 {
            rem = (rem << 1) ^ if rem >> 9 != 0 { 0x537 } else { 0 };
        }
        ((data << 10) | rem) ^ 0x5412
    }

    fn draw_format_bits(&mut self, mask: u8) {
        let bits = Self::format_bits(mask);
        let bit = |i: u32| (bits >> i) & 1 != 0;
        let size = self.size as i32;
        // First copy (around the top-left finder).
        for i in 0..6u32 {
            self.set_function(8, i as i32, bit(i));
        }
        self.set_function(8, 7, bit(6));
        self.set_function(8, 8, bit(7));
        self.set_function(7, 8, bit(8));
        for i in 9..15u32 {
            self.set_function(14 - i as i32, 8, bit(i));
        }
        // Second copy (split along the other two finders).
        for i in 0..8u32 {
            self.set_function(size - 1 - i as i32, 8, bit(i));
        }
        for i in 8..15u32 {
            self.set_function(8, size - 15 + i as i32, bit(i));
        }
        self.set_function(8, size - 8, true); // The always-dark module.
    }

    fn draw_version_info(&mut self, version: usize) {
        if version < 7 {
            return;
        }
        let mut rem: u32 = version as u32;
        for _ in 0..12 {
            rem = (rem << 1) ^ if rem >> 11 != 0 { 0x1F25 } else { 0 };
        }
        let bits = (version as u32) << 12 | rem;
        let size = self.size as i32;
        for i in 0..18u32 {
            let b = (bits >> i) & 1 != 0;
            let a = size - 11 + (i % 3) as i32;
            let c = (i / 3) as i32;
            self.set_function(a, c, b);
            self.set_function(c, a, b);
        }
    }

    fn draw_function_patterns(&mut self, version: usize) {
        let size = self.size as i32;
        // Timing patterns (finders overwrite the overlaps).
        for i in 0..size {
            self.set_function(6, i, i % 2 == 0);
            self.set_function(i, 6, i % 2 == 0);
        }
        self.draw_finder(3, 3);
        self.draw_finder(size - 4, 3);
        self.draw_finder(3, size - 4);
        // Alignment patterns, skipping the three finder corners.
        let positions = alignment_positions(version, self.size);
        let n = positions.len();
        for (i, &px) in positions.iter().enumerate() {
            for (j, &py) in positions.iter().enumerate() {
                if (i, j) != (0, 0) && (i, j) != (0, n - 1) && (i, j) != (n - 1, 0) {
                    self.draw_alignment(px, py);
                }
            }
        }
        self.draw_format_bits(0); // Dummy mask; the winner redraws.
        self.draw_version_info(version);
    }

    /// Place the interleaved codeword bits (plus remainder zeros)
    /// in the standard upward/downward zigzag, skipping column 6.
    fn place_data(&mut self, codewords: &[u8], version: usize) {
        let total_bits = codewords.len() * 8 + remainder_bits(version);
        let size = self.size as i32;
        let mut i = 0usize; // Bit index into the stream (MSB first).
        let mut right = size - 1;
        while right >= 1 {
            if right == 6 {
                right = 5; // Skip the vertical timing column.
            }
            for vert in 0..size {
                for j in 0..2 {
                    let x = right - j;
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { size - 1 - vert } else { vert };
                    if !self.is_function[y as usize][x as usize] {
                        let mut dark = false;
                        if i < total_bits {
                            if i / 8 < codewords.len() {
                                dark = (codewords[i / 8] >> (7 - i % 8)) & 1 != 0;
                            }
                            i += 1;
                        }
                        self.modules[y as usize][x as usize] = dark;
                    }
                }
            }
            right -= 2;
        }
    }

    fn apply_mask(&mut self, mask: u8) {
        for y in 0..self.size {
            for x in 0..self.size {
                if self.is_function[y][x] {
                    continue;
                }
                let invert = match mask {
                    0 => (x + y) % 2 == 0,
                    1 => y % 2 == 0,
                    2 => x % 3 == 0,
                    3 => (x + y) % 3 == 0,
                    4 => (x / 3 + y / 2) % 2 == 0,
                    5 => (x * y) % 2 + (x * y) % 3 == 0,
                    6 => ((x * y) % 2 + (x * y) % 3) % 2 == 0,
                    _ => ((x + y) % 2 + (x * y) % 3) % 2 == 0,
                };
                if invert {
                    self.modules[y][x] = !self.modules[y][x];
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Penalty evaluation (mask selection)
// ---------------------------------------------------------------------------

/// Rule 3's finder-like pattern counter over one line's run
/// history (most recent run first, 7 entries): counts patterns
/// with a 1:1:3:1:1 core and a light run of ≥4n on either side
/// (the caller's virtual quiet-zone padding supplies the border).
fn finder_penalty_count(history: &[i32; 7]) -> usize {
    let n = history[1];
    let core =
        n > 0 && history[2] == n && history[4] == n && history[5] == n && history[3] == n * 3;
    if !core {
        return 0;
    }
    (if history[0] >= n * 4 && history[6] >= n {
        1
    } else {
        0
    }) + (if history[6] >= n * 4 && history[0] >= n {
        1
    } else {
        0
    })
}

/// Add a run to the history, applying the virtual quiet zone:
/// a run that begins the line (buffer still empty) is extended
/// by one symbol width of light border.
fn finder_penalty_add(run: i32, size: i32, history: &mut [i32; 7]) {
    let mut run = run;
    if history[0] == 0 {
        run += size;
    }
    history.copy_within(0..6, 1);
    history[0] = run;
}

/// End-of-line rule 3 accounting: terminate a trailing dark run,
/// then add the trailing virtual light border.
fn finder_penalty_terminate(
    run_color_dark: bool,
    mut run: i32,
    size: i32,
    history: &mut [i32; 7],
) -> usize {
    if run_color_dark {
        finder_penalty_add(run, size, history);
        run = 0;
    }
    let run = run + size;
    finder_penalty_add(run, size, history);
    finder_penalty_count(history)
}

/// Rules 1 and 3 scored along one row or column.
fn penalty_line(line: &[bool], size: i32) -> usize {
    let mut result = 0usize;
    let mut run_color = false;
    let mut run = 0i32;
    let mut history = [0i32; 7];
    for &cell in line {
        if cell == run_color {
            run += 1;
            if run == 5 {
                result += 3; // N1
            } else if run > 5 {
                result += 1;
            }
        } else {
            finder_penalty_add(run, size, &mut history);
            if !run_color {
                result += finder_penalty_count(&history) * 40; // N3
            }
            run_color = cell;
            run = 1;
        }
    }
    result + finder_penalty_terminate(run_color, run, size, &mut history) * 40
}

/// The specification's total penalty score for one candidate
/// symbol (lower is better).
fn penalty_score(modules: &[Vec<bool>], size: usize) -> usize {
    let mut result = 0usize;
    let sz = size as i32;

    // Rules 1 and 3, per row then per column.
    for row in modules {
        result += penalty_line(row, sz);
    }
    let mut columns: Vec<Vec<bool>> = vec![Vec::with_capacity(size); size];
    for row in modules {
        for (x, &cell) in row.iter().enumerate() {
            columns[x].push(cell);
        }
    }
    for col in &columns {
        result += penalty_line(col, sz);
    }

    // Rule 2: 2×2 blocks of one color.
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let c = modules[y][x];
            if c == modules[y][x + 1] && c == modules[y + 1][x] && c == modules[y + 1][x + 1] {
                result += 3; // N2
            }
        }
    }

    // Rule 4: deviation of the dark ratio from 50% in 5% steps.
    let dark = modules.iter().flatten().filter(|&&m| m).count() as i32;
    let total = sz * sz;
    let k = ((dark * 20 - total * 10).abs() + total - 1) / total - 1;
    result + (k.max(0) as usize) * 10 // N4
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Encode `text` (as its UTF-8 bytes) into a QR symbol at level
/// M, choosing the smallest fitting version in 1..=MAX_VERSION
/// and the penalty-optimal mask.
pub fn encode(data: &[u8]) -> Result<QrMatrix, QrError> {
    if data.is_empty() {
        return Err(QrError::Empty);
    }
    let version = (1..=MAX_VERSION)
        .find(|&v| 4 + cci_bits(v) + data.len() * 8 <= data_codewords(v) * 8)
        .ok_or(QrError::TooLong { bytes: data.len() })?;

    let data_cw = build_data_codewords(data, version);
    let all_cw = add_ecc_and_interleave(&data_cw, version);

    let mut best: Option<(usize, u8, Vec<Vec<bool>>)> = None;
    for mask in 0..8u8 {
        let mut canvas = Canvas::new(version);
        canvas.draw_function_patterns(version);
        canvas.place_data(&all_cw, version);
        canvas.apply_mask(mask);
        canvas.draw_format_bits(mask);
        let score = penalty_score(&canvas.modules, canvas.size);
        if best.as_ref().map(|(s, _, _)| score < *s).unwrap_or(true) {
            best = Some((score, mask, canvas.modules));
        }
    }
    let (_, mask, modules) = match best {
        Some(b) => b,
        None => return Err(QrError::Empty), // Unreachable: 8 candidates always exist.
    };
    Ok(QrMatrix {
        version,
        mask,
        size: version * 4 + 17,
        modules,
    })
}

/// Convenience: encode a string's UTF-8 bytes.
pub fn encode_str(text: &str) -> Result<QrMatrix, QrError> {
    encode(text.as_bytes())
}
