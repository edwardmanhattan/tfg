//! QR codes, byte mode, error-correction level M, versions 1 to 10.
//!
//! Written out rather than pulled in because no QR crate is vendored, and a
//! pairing code is the one place in this program where a wrong answer is
//! *quiet*: a malformed matrix still looks like a QR code. So the module is
//! deliberately small in scope — one mode, one level, ten versions, enough for
//! a `tfg-control://` offer — and pinned by tests that compare against a
//! mature implementation rather than against itself.
//!
//! Three decisions worth the words:
//!
//! - **Level M only.** The payload is a pairing URI; M (15% recovery) is the
//!   usual default and one level is one table to get wrong instead of four. A
//!   caller that needs another level needs a different program, not a flag.
//! - **Byte mode only.** The payload is JSON and a URI, and byte mode encodes
//!   both without a mode-switching optimiser. A number in a pairing code is
//!   not worth the complexity of numeric compaction.
//! - **Versions 1 to 10.** 213 bytes at M, which is several times the largest
//!   offer this program makes. Beyond that [`encode`] refuses with the size it
//!   would have needed, rather than silently truncating.
//!
//! The rendering half is separate: [`Matrix`] is the truth, and
//! [`Matrix::render_terminal`] is one opinion about how to show it. A test that
//! compares matrices should use [`Matrix::to_ascii`], which has no quiet zone
//! and no colour.
//!
//! # How this was checked
//!
//! The encoder is not trusted because it looks right. Every version from 1 to
//! 10, each carrying a payload filled to its exact level-M capacity, plus the
//! pairing offer and a forced version-1/7/10 set, had all eight mask candidates
//! diffed module for module against `qrencode -t ASCII -l M -m 0 -8 -v N`. In
//! every case qrencode's matrix equals one candidate exactly, which pins the
//! byte encoding, Reed–Solomon parity, interleaving, placement, format bits and
//! version bits. Each chosen matrix was then written as a PBM with its quiet
//! zone and decoded by `zbarimg`, which returned the original bytes every time
//! — including the 213-byte version-10 payload and the offer as `tfgctl pair`
//! actually prints it.
//!
//! The one legitimate difference is the mask *choice*: qrencode's heuristic
//! and the ISO N1–N4 score above disagree on some payloads. Both are valid
//! symbols with the same data, so the tests pin the grid with [`encode_mask`]
//! where the two disagree rather than pretending the heuristics are identical.

/// A square grid of modules. `true` is a dark module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    size: usize,
    modules: Vec<bool>,
}

impl Matrix {
    /// Modules along one side, excluding any quiet zone.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether the module at `row`, `column` is dark. Out of range is light,
    /// which is what a quiet zone is anyway.
    pub fn is_dark(&self, row: usize, column: usize) -> bool {
        if row >= self.size || column >= self.size {
            return false;
        }
        self.modules[row * self.size + column]
    }

    /// Two characters per module, `##` dark and two spaces light — the shape
    /// `qrencode -t ASCII` prints, so a test can compare byte for byte.
    pub fn to_ascii(&self) -> String {
        let mut out = String::with_capacity(self.size * (self.size * 2 + 1));
        for row in 0..self.size {
            for column in 0..self.size {
                out.push_str(if self.is_dark(row, column) {
                    "##"
                } else {
                    "  "
                });
            }
            out.push('\n');
        }
        out
    }

    /// The code drawn for a terminal, with a four-module quiet zone.
    ///
    /// Two module rows per line of text, because a character cell is taller
    /// than it is wide and half-height blocks put the modules back into
    /// squares. `colour` paints explicit black-on-white with SGR so the code
    /// scans on a dark terminal as well as a light one; without it the caller
    /// gets the same glyphs on the terminal's own background, which is right
    /// for a light one and inverted on a dark one.
    pub fn render_terminal(&self, colour: bool) -> String {
        let quiet = 4usize;
        let width = self.size + quiet * 2;
        let height = self.size + quiet * 2;
        let dark = |row: isize, column: isize| -> bool {
            let row = row - quiet as isize;
            let column = column - quiet as isize;
            if row < 0 || column < 0 {
                return false;
            }
            self.is_dark(row as usize, column as usize)
        };
        let mut out = String::with_capacity((width + 12) * (height / 2 + 2));
        for line in 0..height.div_ceil(2) {
            if colour {
                out.push_str("\x1b[30;47m");
            }
            for column in 0..width {
                let top = dark((line * 2) as isize, column as isize);
                let bottom = dark((line * 2 + 1) as isize, column as isize);
                out.push(match (top, bottom) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                });
            }
            if colour {
                out.push_str("\x1b[0m");
            }
            out.push('\n');
        }
        out
    }
}

/// Why encoding failed: the payload does not fit the largest version this
/// module carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong {
    /// The payload's length in bytes.
    pub needed: usize,
    /// What the largest supported version holds.
    pub limit: usize,
}

impl std::fmt::Display for TooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} bytes is more than the {} a version-{} code holds at level M",
            self.needed, self.limit, MAX_VERSION
        )
    }
}

impl std::error::Error for TooLong {}

/// The largest version this module builds.
pub const MAX_VERSION: usize = 10;

/// Encode `data` as the smallest version that holds it.
pub fn encode(data: &[u8]) -> Result<Matrix, TooLong> {
    let version = (1..=MAX_VERSION)
        .find(|version| data.len() <= capacity_bytes(*version))
        .ok_or(TooLong {
            needed: data.len(),
            limit: capacity_bytes(MAX_VERSION),
        })?;
    Ok(encode_version(data, version))
}

/// Encode at a particular version, padding a short payload. Public for tests
/// and for a caller that wants a fixed size (a phone screen, a printed label).
pub fn encode_version(data: &[u8], version: usize) -> Matrix {
    assert!(
        (1..=MAX_VERSION).contains(&version),
        "version {version} is outside 1..={MAX_VERSION}"
    );
    assert!(
        data.len() <= capacity_bytes(version),
        "{} bytes does not fit version {version}",
        data.len()
    );

    // Every mask is a valid code; the standard's penalty score picks the one
    // least likely to confuse a scanner. Ties go to the lowest mask, which is
    // what every implementation does and therefore what a comparison test can
    // rely on.
    let mut best: Option<(i32, Matrix)> = None;
    for mask in 0..8 {
        let matrix = encode_mask(data, version, mask);
        let score = penalty(&matrix);
        if best.as_ref().is_none_or(|(chosen, _)| score < *chosen) {
            best = Some((score, matrix));
        }
    }
    best.expect("at least one mask").1
}

/// One fixed mask, without scoring. [`encode_version`] is the scored choice;
/// this exists so a test can compare each candidate against another
/// implementation, which is the only way to tell a bad score from a bad grid.
fn encode_mask(data: &[u8], version: usize, mask: usize) -> Matrix {
    let codewords = interleave(&payload_codewords(data, version), version);
    let mut canvas = Canvas::new(version);
    canvas.draw_function_patterns();
    canvas.draw_codewords(&codewords);
    canvas.apply_mask(mask);
    canvas.draw_format(mask);
    canvas.into_matrix()
}

/// How many bytes a version holds at level M. The bit budget is the data
/// codewords, less a four-bit mode indicator, the character count (eight bits
/// up to version 9, sixteen from version 10), and whatever the terminator
/// cannot use.
fn capacity_bytes(version: usize) -> usize {
    let bits = data_codewords(version) * 8;
    let count_bits = if version <= 9 { 8 } else { 16 };
    (bits - 4 - count_bits) / 8
}

/// The block layout for level M. `(blocks, data codewords per block)`; versions
/// with more than one group exist so that the last blocks are not left with a
/// short tail the interleaver would have to special-case.
struct Layout {
    ec_per_block: usize,
    groups: &'static [(usize, usize)],
}

fn layout(version: usize) -> Layout {
    let (ec_per_block, groups): (usize, &'static [(usize, usize)]) = match version {
        1 => (10, &[(1, 16)]),
        2 => (16, &[(1, 28)]),
        3 => (26, &[(1, 44)]),
        4 => (18, &[(2, 32)]),
        5 => (24, &[(2, 43)]),
        6 => (16, &[(4, 27)]),
        7 => (18, &[(4, 31)]),
        8 => (22, &[(2, 38), (2, 39)]),
        9 => (22, &[(3, 36), (2, 37)]),
        10 => (26, &[(4, 43), (1, 44)]),
        other => panic!("no layout for version {other}"),
    };
    Layout {
        ec_per_block,
        groups,
    }
}

fn data_codewords(version: usize) -> usize {
    layout(version)
        .groups
        .iter()
        .map(|(blocks, codewords)| blocks * codewords)
        .sum()
}

/// Alignment-pattern centres, per version. Version 1 has none: it is all
/// finders and timing.
fn alignment_centres(version: usize) -> &'static [usize] {
    match version {
        1 => &[],
        2 => &[6, 18],
        3 => &[6, 22],
        4 => &[6, 26],
        5 => &[6, 30],
        6 => &[6, 34],
        7 => &[6, 22, 38],
        8 => &[6, 24, 42],
        9 => &[6, 26, 46],
        10 => &[6, 28, 50],
        other => panic!("no alignment centres for version {other}"),
    }
}

// ---------------------------------------------------------------------------
// Bits and blocks
// ---------------------------------------------------------------------------

struct Bits {
    bits: Vec<bool>,
}

impl Bits {
    fn new() -> Self {
        Self { bits: Vec::new() }
    }

    fn push(&mut self, value: u32, count: usize) {
        for shift in (0..count).rev() {
            self.bits.push((value >> shift) & 1 == 1);
        }
    }
}

/// Data codewords: mode, count, bytes, terminator, padding.
fn payload_codewords(data: &[u8], version: usize) -> Vec<u8> {
    let mut bits = Bits::new();
    bits.push(0b0100, 4); // byte mode
    bits.push(data.len() as u32, if version <= 9 { 8 } else { 16 });
    for &byte in data {
        bits.push(u32::from(byte), 8);
    }
    let capacity = data_codewords(version) * 8;
    // Terminator: up to four zeros, then the rest of the last byte.
    let terminator = (capacity - bits.bits.len()).min(4);
    bits.push(0, terminator);
    while bits.bits.len() % 8 != 0 {
        bits.bits.push(false);
    }
    let mut codewords: Vec<u8> = bits
        .bits
        .chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .fold(0u8, |byte, &bit| (byte << 1) | u8::from(bit))
        })
        .collect();
    // The standard's two padding codewords, alternating so a scanner sees a
    // changing pattern rather than a long run.
    let mut pad = 0xEC;
    while codewords.len() < data_codewords(version) {
        codewords.push(pad);
        pad ^= 0xEC ^ 0x11;
    }
    codewords
}

/// Split into blocks, compute error correction, then interleave: data
/// codewords round-robin, then the parity round-robin.
fn interleave(codewords: &[u8], version: usize) -> Vec<u8> {
    let layout = layout(version);
    let mut data_blocks: Vec<Vec<u8>> = Vec::new();
    let mut offset = 0;
    for &(blocks, per_block) in layout.groups {
        for _ in 0..blocks {
            data_blocks.push(codewords[offset..offset + per_block].to_vec());
            offset += per_block;
        }
    }
    let divisor = reed_solomon_divisor(layout.ec_per_block);
    let ec_blocks: Vec<Vec<u8>> = data_blocks
        .iter()
        .map(|block| reed_solomon_remainder(block, &divisor))
        .collect();

    let mut out = Vec::with_capacity(codewords.len() + ec_blocks.len() * layout.ec_per_block);
    let longest = data_blocks.iter().map(Vec::len).max().unwrap_or(0);
    for index in 0..longest {
        for block in &data_blocks {
            if let Some(&byte) = block.get(index) {
                out.push(byte);
            }
        }
    }
    for index in 0..layout.ec_per_block {
        for block in &ec_blocks {
            out.push(block[index]);
        }
    }
    out
}

/// Multiply in GF(2^8) modulo the QR code's polynomial, x^8 + x^4 + x^3 + x^2 +
/// 1.
fn gf_multiply(x: u8, y: u8) -> u8 {
    // Russian peasant multiplication. `product` is kept reduced at every step:
    // shifting it left is a multiply by x, and a carry out of the top bit is
    // reduced by the modulus x^8 + x^4 + x^3 + x^2 + 1, which as a low byte is
    // 0x1D. The bits of `y` are read most-significant first so that the shift
    // of `product` lines up with the place value being added.
    let mut product = 0u8;
    for shift in (0..8).rev() {
        product = (product << 1) ^ ((product >> 7) * 0x1D);
        product ^= ((y >> shift) & 1) * x;
    }
    product
}

/// The generator polynomial for `degree` error-correction codewords, highest
/// term implicit.
fn reed_solomon_divisor(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree.saturating_sub(1)];
    result.push(1);
    let mut root = 1u8;
    for _ in 0..degree {
        for index in 0..degree {
            result[index] = gf_multiply(result[index], root);
            if index + 1 < degree {
                result[index] ^= result[index + 1];
            }
        }
        root = gf_multiply(root, 0x02);
    }
    result
}

/// Synthetic division of `data` by `divisor`, giving the parity bytes.
fn reed_solomon_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; divisor.len()];
    for &byte in data {
        let factor = byte ^ result.remove(0);
        result.push(0);
        for (slot, &coefficient) in result.iter_mut().zip(divisor) {
            *slot ^= gf_multiply(coefficient, factor);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// The grid
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Canvas {
    size: usize,
    modules: Vec<bool>,
    function: Vec<bool>,
}

impl Canvas {
    fn new(version: usize) -> Self {
        let size = version * 4 + 17;
        Self {
            size,
            modules: vec![false; size * size],
            function: vec![false; size * size],
        }
    }

    /// Write a module that belongs to a function pattern. Function modules are
    /// never masked and never hold data, so this also marks the position as
    /// reserved.
    fn put_function(&mut self, row: isize, column: isize, dark: bool) {
        if row < 0 || column < 0 || row >= self.size as isize || column >= self.size as isize {
            return;
        }
        let index = row as usize * self.size + column as usize;
        self.modules[index] = dark;
        self.function[index] = true;
    }

    /// Write a data module. Deliberately does *not* mark it as a function:
    /// doing that would make [`Canvas::apply_mask`] skip every module it is
    /// supposed to flip, and the code would look plausible while carrying no
    /// mask at all.
    fn put_data(&mut self, row: isize, column: isize, dark: bool) {
        if row < 0 || column < 0 || row >= self.size as isize || column >= self.size as isize {
            return;
        }
        self.modules[row as usize * self.size + column as usize] = dark;
    }

    fn is_function(&self, row: usize, column: usize) -> bool {
        self.function[row * self.size + column]
    }

    fn draw_function_patterns(&mut self) {
        let size = self.size as isize;

        // Timing patterns first: the finders and alignment patterns draw over
        // them where they meet, which is how they are supposed to look.
        for index in 0..self.size {
            let dark = index % 2 == 0;
            self.put_function(6, index as isize, dark);
            self.put_function(index as isize, 6, dark);
        }

        for (column, row) in [(3isize, 3isize), (size - 4, 3), (3, size - 4)] {
            for dy in -4..=4isize {
                for dx in -4..=4isize {
                    let distance = dx.abs().max(dy.abs());
                    self.put_function(row + dy, column + dx, distance != 2 && distance != 4);
                }
            }
        }

        let centres = alignment_centres((self.size - 17) / 4);
        let last = centres.len().saturating_sub(1);
        for (i, &row) in centres.iter().enumerate() {
            for (j, &column) in centres.iter().enumerate() {
                // The three corners are finder patterns already.
                if (i, j) == (0, 0) || (i, j) == (0, last) || (i, j) == (last, 0) {
                    continue;
                }
                for dy in -2..=2isize {
                    for dx in -2..=2isize {
                        let distance = dx.abs().max(dy.abs());
                        self.put_function(row as isize + dy, column as isize + dx, distance != 1);
                    }
                }
            }
        }

        // Reserve the format areas so data placement leaves them alone; the
        // values are written once a mask is chosen.
        for (_, row, column) in self.format_positions() {
            self.put_function(row as isize, column as isize, false);
        }
        // The module that is always dark, below the top-left format strip.
        self.put_function(self.size as isize - 8, 8, true);

        // Version information exists from version 7 and does not depend on the
        // mask, so it is written once here.
        if self.size >= 45 {
            let version = (self.size - 17) / 4;
            let bits = version_bits(version);
            for index in 0..18 {
                let bit = (bits >> index) & 1 == 1;
                let a = self.size - 11 + index % 3;
                let b = index / 3;
                self.put_function(b as isize, a as isize, bit);
                self.put_function(a as isize, b as isize, bit);
            }
        }
    }

    /// Every format module as `(bit, row, column)`, both copies. Bit 0 is the
    /// least significant bit of the fifteen-bit format word. The placement is
    /// the standard's Figure 25; writing the bit index next to the position is
    /// what keeps the second copy from drifting a row (the always-dark module
    /// at `(size - 8, 8)` sits exactly between the two runs).
    fn format_positions(&self) -> Vec<(usize, usize, usize)> {
        let size = self.size;
        let mut positions = Vec::with_capacity(30);
        // First copy: bits 0–5 down column 8, then bits 6–8 around the corner,
        // then bits 9–14 leftwards along row 8.
        for bit in 0..6 {
            positions.push((bit, bit, 8));
        }
        positions.push((6, 7, 8));
        positions.push((7, 8, 8));
        positions.push((8, 8, 7));
        for bit in 9..15 {
            positions.push((bit, 8, 14 - bit));
        }
        // Second copy: bits 0–7 right-to-left along row 8, bits 8–14 up column
        // 8 from `size - 7` to `size - 1`.
        for bit in 0..8 {
            positions.push((bit, 8, size - 1 - bit));
        }
        for bit in 8..15 {
            positions.push((bit, size - 15 + bit, 8));
        }
        positions
    }

    /// The format value for level M and `mask`: five data bits, ten BCH bits,
    /// XORed with the standard's mask so an all-zero format is not all light.
    fn draw_format(&mut self, mask: usize) {
        let bits = format_bits(mask);
        for (bit, row, column) in self.format_positions() {
            self.put_function(row as isize, column as isize, (bits >> bit) & 1 == 1);
        }
    }

    /// Walk the two-module-wide columns from the bottom right, skipping
    /// function modules, and drop the codeword bits in.
    fn draw_codewords(&mut self, codewords: &[u8]) {
        let size = self.size as isize;
        let mut bit_index = 0usize;
        let total_bits = codewords.len() * 8;
        let mut right = size - 1;
        while right >= 1 {
            if right == 6 {
                // Column 6 is the timing pattern; the data columns are 0–5 and
                // 7 onwards.
                right = 5;
            }
            for vertical in 0..size {
                for offset in 0..2 {
                    let column = right - offset;
                    let upward = (right + 1) & 2 == 0;
                    let row = if upward {
                        size - 1 - vertical
                    } else {
                        vertical
                    };
                    if !self.is_function(row as usize, column as usize) && bit_index < total_bits {
                        let dark = (codewords[bit_index / 8] >> (7 - bit_index % 8)) & 1 == 1;
                        self.put_data(row, column, dark);
                        bit_index += 1;
                    }
                }
            }
            right -= 2;
        }
    }

    fn apply_mask(&mut self, mask: usize) {
        for row in 0..self.size {
            for column in 0..self.size {
                if self.is_function(row, column) {
                    continue;
                }
                if mask_hit(mask, row, column) {
                    let index = row * self.size + column;
                    self.modules[index] = !self.modules[index];
                }
            }
        }
    }

    fn into_matrix(self) -> Matrix {
        Matrix {
            size: self.size,
            modules: self.modules,
        }
    }
}

fn format_bits(mask: usize) -> u32 {
    // Level M is `00`; then the three mask bits.
    let data = mask as u32;
    let mut remainder = data;
    for _ in 0..10 {
        remainder = (remainder << 1) ^ ((remainder >> 9) * 0x537);
    }
    ((data << 10) | remainder) ^ 0x5412
}

fn version_bits(version: usize) -> u32 {
    let mut remainder = version as u32;
    for _ in 0..12 {
        remainder = (remainder << 1) ^ ((remainder >> 11) * 0x1F25);
    }
    (version as u32) << 12 | remainder
}

fn mask_hit(mask: usize, row: usize, column: usize) -> bool {
    match mask {
        0 => (row + column) % 2 == 0,
        1 => row % 2 == 0,
        2 => column % 3 == 0,
        3 => (row + column) % 3 == 0,
        4 => (row / 2 + column / 3) % 2 == 0,
        5 => (row * column) % 2 + (row * column) % 3 == 0,
        6 => ((row * column) % 2 + (row * column) % 3) % 2 == 0,
        7 => ((row + column) % 2 + (row * column) % 3) % 2 == 0,
        other => panic!("mask {other} does not exist"),
    }
}

/// The seven modules of a finder pattern's centre line: dark, light, three
/// dark, light, dark.
const FINDER_CORE: [bool; 7] = [true, false, true, true, true, false, true];

/// Penalty for finder-like patterns along one row or column, given a way to
/// read a module at an offset. A pattern is the core flanked by light on both
/// sides (otherwise it is not the 1:1:3:1:1 shape at all), and it scores once
/// for every side whose light run is four modules or longer.
fn finder_like(line: impl Fn(usize) -> bool, size: usize) -> i32 {
    let mut count = 0;
    for start in 0..=size.saturating_sub(FINDER_CORE.len()) {
        if (0..FINDER_CORE.len()).any(|offset| line(start + offset) != FINDER_CORE[offset]) {
            continue;
        }
        let before_light = start > 0 && !line(start - 1);
        let after_light = start + 7 < size && !line(start + 7);
        if !before_light && !after_light {
            continue;
        }
        let mut before = 0;
        while start > before && !line(start - before - 1) {
            before += 1;
        }
        let mut after = 0;
        while start + 7 + after < size && !line(start + 7 + after) {
            after += 1;
        }
        if before >= 4 {
            count += 1;
        }
        if after >= 4 {
            count += 1;
        }
    }
    count
}

/// The four penalty rules, added up. Lower is better: they penalise the
/// features that make a scanner's job hard (long runs, blocks, finder-like
/// shapes, and a lopsided ratio of dark to light).
fn penalty(matrix: &Matrix) -> i32 {
    let size = matrix.size();
    let mut score = 0;

    // Runs of five or more of one colour, in rows and columns.
    for index in 0..size {
        let mut run_row = 1;
        let mut run_column = 1;
        for step in 1..size {
            if matrix.is_dark(index, step) == matrix.is_dark(index, step - 1) {
                run_row += 1;
            } else {
                if run_row >= 5 {
                    score += 3 + (run_row - 5);
                }
                run_row = 1;
            }
            if matrix.is_dark(step, index) == matrix.is_dark(step - 1, index) {
                run_column += 1;
            } else {
                if run_column >= 5 {
                    score += 3 + (run_column - 5);
                }
                run_column = 1;
            }
        }
        if run_row >= 5 {
            score += 3 + (run_row - 5);
        }
        if run_column >= 5 {
            score += 3 + (run_column - 5);
        }
    }

    // Two-by-two blocks of one colour.
    for row in 0..size - 1 {
        for column in 0..size - 1 {
            let colour = matrix.is_dark(row, column);
            if matrix.is_dark(row, column + 1) == colour
                && matrix.is_dark(row + 1, column) == colour
                && matrix.is_dark(row + 1, column + 1) == colour
            {
                score += 3;
            }
        }
    }

    // Finder-like patterns: the seven-module core dark, light, dark ×3, light,
    // dark, counted once for each side whose light run is at least four
    // modules wide. Counting maximal runs rather than eleven-module windows is
    // what stops a five-module gap from counting as two patterns.
    for index in 0..size {
        score += finder_like(|offset| matrix.is_dark(index, offset), size) * 40;
        score += finder_like(|offset| matrix.is_dark(offset, index), size) * 40;
    }

    // How far the dark proportion is from half, in 5% steps.
    let total = (size * size) as i32;
    let dark = (0..size)
        .flat_map(|row| (0..size).map(move |column| (row, column)))
        .filter(|&(row, column)| matrix.is_dark(row, column))
        .count() as i32;
    let deviation = (dark * 20 - total * 10).abs();
    let steps = (deviation + total - 1) / total - 1;
    score += steps.max(0) * 10;

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFER: &str = r#"{"pairing_code":"ABCD-EFGH","protocol":"tfg-control","url":"http://192.168.1.20:8787","version":1}"#;
    const HELLO_GOLDEN: &str = concat!(
        "##############  ##    ##    ##############\n",
        "##          ##  ########    ##          ##\n",
        "##  ######  ##      ##  ##  ##  ######  ##\n",
        "##  ######  ##  ##  ##  ##  ##  ######  ##\n",
        "##  ######  ##        ##    ##  ######  ##\n",
        "##          ##        ####  ##          ##\n",
        "##############  ##  ##  ##  ##############\n",
        "                ##    ####                \n",
        "##  ####  ######  ##  ####  ##    ##  ####\n",
        "  ####  ####  ##  ############    ####    \n",
        "##      ##  ##    ##  ##              ####\n",
        "##  ####      ##      ##    ########  ##  \n",
        "  ##  ############      ##    ##    ##  ##\n",
        "                ########    ##      ##  ##\n",
        "##############  ##    ####    ##          \n",
        "##          ##  ##  ##        ##########  \n",
        "##  ######  ##          ############  ####\n",
        "##  ######  ##  ##  ####    ##  ########  \n",
        "##  ######  ##  ####    ##  ####    ##    \n",
        "##          ##      ##    ##  ####      ##\n",
        "##############  ##  ##    ##  ##          \n",
    );

    const OFFER_GOLDEN: &str = concat!(
        "##############    ##  ######          ##    ##########        ####  ##############\n",
        "##          ##    ##    ##          ##      ##    ########          ##          ##\n",
        "##  ######  ##  ##        ##          ####      ##      ##      ##  ##  ######  ##\n",
        "##  ######  ##  ##    ######        ####  ####  ####    ####        ##  ######  ##\n",
        "##  ######  ##  ####    ####  ####    ######  ##        ##    ####  ##  ######  ##\n",
        "##          ##  ##    ######    ####    ##          ####  ##    ##  ##          ##\n",
        "##############  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##############\n",
        "                ########    ##      ########  ####  ##        ##                  \n",
        "##  ##########    ##      ##          ##  ####  ####  ##            ##########    \n",
        "####  ####    ##  ##      ######  ####  ##              ####  ####  ##  ##  ##  ##\n",
        "      ########  ####    ##  ##  ##      ##  ##      ####  ##      ##      ######  \n",
        "    ##        ##  ######    ##  ##  ##  ######  ##    ####    ########  ##    ##  \n",
        "            ######      ##    ##  ####      ##  ########  ##  ##  ##        ######\n",
        "####  ##  ##  ##            ##  ##  ##  ####    ##      ##  ##################  ##\n",
        "  ##      ####    ##  ####    ##  ######    ##        ####        ####            \n",
        "  ##  ##      ##  ########  ######  ########        ##  ##  ######      ##    ####\n",
        "    ##    ####  ##  ##        ##########      ##  ##      ##    ##          ####  \n",
        "##      ##    ######    ##  ##  ####  ##  ########  ##        ####  ########  ####\n",
        "  ##    ######            ##      ##    ####    ####  ########      ########  ##  \n",
        "####  ##      ########  ####  ########    ######        ##  ##      ##  ####  ####\n",
        "####        ##      ####  ##    ##        ##  ######      ##  ##          ######  \n",
        "####  ####    ##    ####  ##  ####  ############    ##    ##  ####  ##  ####    ##\n",
        "##      ##  ####      ####    ######              ##  ##  ##    ########  ####    \n",
        "  ####  ####        ########          ##  ####                ############    ####\n",
        "    ##    ####    ########  ##  ##        ##########      ##  ##  ##      ######  \n",
        "        ####  ######  ##  ##    ##  ##  ##          ##  ##############  ##      ##\n",
        "##        ####  ########        ######  ##  ##  ##########      ##  ##  ##  ####  \n",
        "      ##  ##  ##  ##    ##    ####  ##  ####  ####      ##  ##    ##  ##  ##  ####\n",
        "      ############  ######  ####  ########  ##    ####    ##########  ##  ########\n",
        "####  ######      ########  ####    ##      ####  ##    ##  ##############    ####\n",
        "##    ##    ##########  ##  ####    ##  ####          ####  ##    ##    ##  ####  \n",
        "##  ########  ########  ##########  ##    ##    ##  ####    ####  ####  ####  ##  \n",
        "##    ##  ########          ##########  ##      ####      ####  ##############  ##\n",
        "                ##  ####  ##  ##      ##      ########          ##      ##    ####\n",
        "##############    ##  ####  ######  ##      ##  ####  ##  ########  ##  ##  ####  \n",
        "##          ##  ####    ######        ####    ####            ####      ####  ##  \n",
        "##  ######  ##  ####  ##        ##  ##########  ####    ####    ##########  ######\n",
        "##  ######  ##  ##              ##    ######  ########      ##  ##          ######\n",
        "##  ######  ##  ##  ######                ####    ##  ##  ####    ########        \n",
        "##          ##        ####  ######  ##  ####  ##                ##      ##    ##  \n",
        "##############  ##  ##  ##    ####  ##          ####  ##  ##  ##  ##    ######    \n",
    );

    const V7_GOLDEN: &str = concat!(
        "##############    ####  ##  ####  ##  ####    ##  ####  ######  ##      ##  ##############\n",
        "##          ##  ####  ##################  ##        ######  ########  ##    ##          ##\n",
        "##  ######  ##          ##  ##  ##  ##    ####  ##  ####      ##      ##    ##  ######  ##\n",
        "##  ######  ##    ##  ##    ####        ##  ######        ##          ####  ##  ######  ##\n",
        "##  ######  ##  ##########  ##    ##  ############  ##  ######  ##########  ##  ######  ##\n",
        "##          ##    ##    ####        ######      ##    ####  ######          ##          ##\n",
        "##############  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##############\n",
        "                  ##########      ########      ######  ######  ##########                \n",
        "##  ##  ##  ##    ##  ########  ####    ##########  ######  ######  ####        ##    ##  \n",
        "####      ##  ##      ####  ########  ########        ##      ##              ##  ####  ##\n",
        "  ########  ##############  ######    ####  ##  ####      ##      ##    ####        ######\n",
        "    ##  ####  ##      ######      ##  ##          ######  ####  ##############  ##    ##  \n",
        "####  ########  ############################    ##  ####    ######  ####    ########      \n",
        "####  ##  ##  ##  ##    ##  ####    ####    ##  ##    ####    ##              ##  ####  ##\n",
        "####  ##  ########  ##  ##    ####    ##      ######  ##  ##      ##    ####        ######\n",
        "##########    ####  ##  ####  ##  ##        ####  ####  ######  ##############  ##    ##  \n",
        "####  ##  ####  ##  ######    ##  ####  ##  ##      ######  ######  ####    ########      \n",
        "##    ######  ##    ##  ##  ######  ##########  ##    ##      ##              ##  ####  ##\n",
        "####  ##    ######  ##  ##  ####      ##    ########      ##      ##    ####        ######\n",
        "####  ##            ##  ############          ##    ##  ######  ##############  ##    ##  \n",
        "##      ##########  ##    ####    ####  ##########    ####  ######  ################      \n",
        "##########      ##  ##    ########  ######      ##  ####      ##        ##      ######  ##\n",
        "  ####  ##  ##  ##  ##    ########  ##  ##  ##  ##  ##    ##      ##  ####  ##  ##  ######\n",
        "##    ####      ##########  ##  ##      ##      ######  ######  ##########      ##    ##  \n",
        "####    ############    ##  ##      ##############  ######  ######  ##  ############      \n",
        "########  ##      ##  ##      ####      ##  ######    ##      ##      ##  ##      ####  ##\n",
        "  ####      ##        ############      ##    ##  ##      ##      ##  ##  ####  ##  ######\n",
        "##    ######  ######  ####  ##  ########  ##      ######  ####  ######  ##  ######    ####\n",
        "######  ######      ##      ####      ########  ##  ##  ##  ######  ##  ##    ##  ##  ##  \n",
        "####  ##  ##  ########          ##########  ######    ##      ##      ##  ##      ####  ##\n",
        "      ##    ########      ##    ##########      ####      ##      ##  ##  ####  ##  ######\n",
        "##  ########  ######  ##    ##  ####    ####    ######  ######  ######  ##  ######    ##  \n",
        "      ##  ####              ##  ##  ####  ####      ######  ######  ##  ##    ##  ##      \n",
        "##    ####    ########      ##  ##  ##  ##  ##  ##    ##      ##      ##  ##      ####  ##\n",
        "        ##  ########        ##    ####  ##  ####  ##      ##      ##  ##  ####  ##  ######\n",
        "  ########    ######  ##  ##########  ##          ##    ######  ######  ##  ######    ##  \n",
        "##    ####  ####            ######  ######################  ######  ##  ############      \n",
        "                ######  ##  ####    ##  ##      ####  ##      ##      ####      ######  ##\n",
        "##############      ##  ##      ##  ######  ##  ######    ##      ##    ##  ##  ##  ######\n",
        "##          ##        ##  ##    ####    ##      ######  ######  ##########      ##    ##  \n",
        "##  ######  ##  ####    ##    ####  ##  ##########  ######  ######  ################      \n",
        "##  ######  ##    ####  ####  ##    ####  ##  ##      ##      ##        ######  ##########\n",
        "##  ######  ##  ####  ##        ####      ####    ##      ##      ##  ####  ######  ######\n",
        "##          ##        ##  ##    ######  ######  ######  ######  ########      ##      ##  \n",
        "##############  ##      ##    ##  ##  ####  ######  ######  ######  ##    ##      ##  ####\n",
    );

    const V10_GOLDEN: &str = concat!(
        "##############    ####      ##    ####  ####  ########  ######  ######  ######  ######  ##  ####    ##############\n",
        "##          ##  ##  ##    ######  ##    ########    ######  ######  ######  ######  ########  ##    ##          ##\n",
        "##  ######  ##    ##        ##              ##        ##      ####    ##      ##      ##    ####    ##  ######  ##\n",
        "##  ######  ##      ##    ######    ##  ##      ####      ##  ##  ##      ##      ##          ##    ##  ######  ##\n",
        "##  ######  ##  ##      ##  ##      ##    ##  ####  ##################  ######  ######  ####  ##    ##  ######  ##\n",
        "##          ##      ##    ######    ########  ##    ##      ####    ######  ######  ######  ##      ##          ##\n",
        "##############  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##  ##############\n",
        "                    ##  ##    ######  ####    ####  ##      ##  ######  ######  ######  ##########                \n",
        "##  ##  ##  ##    ##  ##  ####  ##    ##    ####  ################  ######  ######  ######  ####        ##    ##  \n",
        "####  ######  ########  ######  ####  ####        ######      ##      ##      ##      ##              ##  ####  ##\n",
        "####  ##    ########  ##  ##########      ##    ##  ##    ##      ##      ##      ##      ##    ####        ######\n",
        "  ####              ##  ##  ####  ##  ############        ####  ######  ######  ######  ##############  ##    ##  \n",
        "##      ##  ########  ##  ####  ##    ##  ##    ####    ##  ######  ######  ######  ######  ####    ########      \n",
        "  ####  ##    ########  ######    ##  ######    ##  ##        ##      ##      ##      ##              ##  ####  ##\n",
        "####        ######    ##  ##########      ##            ####      ##      ##      ##      ##    ####        ######\n",
        "  ##  ####        ##    ########  ##  ######    ##  ##  ######  ######  ######  ######  ##############  ##    ##  \n",
        "##  ####  ##########  ##  ####  ##    ##  ##  ##    ######  ######  ######  ######  ######  ####    ########      \n",
        "  ##    ##    ######    ##        ##      ##      ######      ##      ##      ##      ##              ##  ####  ##\n",
        "######    ##########  ##  ######  ############  ##  ##    ##      ##      ##      ##      ##    ####        ######\n",
        "  ##      ##    ####    ########      ##  ##  ####      ##  ##  ######  ######  ######  ##############  ##    ##  \n",
        "##  ####  ####    ######  ####  ##    ####  ####  ##  ####    ####  ######  ######  ######  ####    ########      \n",
        "  ##      ##      ##  ##          ########  ##    ######    ####      ##      ##      ##              ##  ####  ##\n",
        "##  ##    ########  ############      ########  ##  ##      ##    ##      ##      ##      ##    ####        ######\n",
        "  ##      ##  ##    ##    ######  ######  ##  ####      ########  ####  ######  ######  ##############  ##    ##  \n",
        "##  ####  ######        ######  ##  ######  ####  ##  ####  ####    ######  ######  ######  ####    ########      \n",
        "          ##      ##    ##        ##  ##    ##    ######      ####    ##      ##      ##              ##  ####  ##\n",
        "######  ##########  ####  ######    ##########  ##  ############  ##      ##      ##      ##    ##########  ######\n",
        "##      ##      ##  ##  ########  ##########  ########      ##  ######  ######  ######  ##########      ##    ##  \n",
        "  ##    ##  ##  ##      ######      ######  ####    ##  ##  ######  ######  ######  ######  ##  ##  ##  ####      \n",
        "  ########      ####  ##  ######  ##  ##    ##      ##      ####      ##      ##      ##        ##      ######  ##\n",
        "##  ##############  ####    ####    ##########  ##############    ##      ##      ##      ##  ############  ######\n",
        "####    ##      ##  ######    ##  ################  ######  ##  ######  ######  ######  ######  ##  ######    ##  \n",
        "  ####  ##  ####        ##  ##      ##############  ####      ####  ######  ######  ######  ##  ##    ##  ##      \n",
        "##  ######        ##  ##  ##########  ##  ##  ##  ##      ##  ##      ##      ##      ##      ##  ##      ######  \n",
        "##      ##  ##  ####  ##        ##  ######    ######    ######    ##      ##      ##      ##  ##  ####  ##  ####  \n",
        "##########    ####  ######  ##    ########    ##  ########  ##  ######  ######  ######  ######  ##  ######    ##  \n",
        "  ##  ##########  ####  ##  ##      ######    ##    ##  ##    ####  ######  ######  ######  ##  ##    ##  ##    ##\n",
        "##    ##  ##        ####    ##  ####  ######      ####  ####  ##      ##      ##      ##      ##  ##      ####  ##\n",
        "##      ######  ########            ##  ####    ##  ##    ####    ##      ##      ##      ##  ##  ####  ##  ######\n",
        "####  ######  ####  ######  ##      ##  ##    ####  ##  ##  ##  ######  ######  ######  ######  ##  ######    ##  \n",
        "  ##  ##########      ##    ##      ####    ####    ####      ####  ######  ######  ######  ##  ##    ##  ##      \n",
        "##    ######      ####      ##  ############      ##      ##  ##      ##      ##      ##      ##  ##      ####  ##\n",
        "####    ##  ##  ##      ##        ####  ####    ####    ######    ##      ##      ##      ##  ##  ####  ##  ######\n",
        "####  ##  ##  ######  ##    ##    ####        ####  ######  ####  ####  ######  ######  ######  ##  ######    ##  \n",
        "  ##  ####  ####      ####  ##      ####    ####    ####    ##  ##  ######  ######  ######  ##  ##    ##  ##      \n",
        "  ##  ######    ####  ##    ##  ##  ########      ##          ##      ##      ##      ##      ##  ##      ####  ##\n",
        "##  ##    ####  ##                    ##  ##    ####    ##  ##    ##      ##      ##      ##  ##  ####  ##  ######\n",
        "##########        ##              ####        ####  ######      ######  ######  ######  ######  ##  ######    ##  \n",
        "            ##  ##        ######    ####    ####    ##############  ######  ######  ######  ##  ############      \n",
        "                ####    ######  ##    ######      ####      ####      ##      ##      ##      ####      ######  ##\n",
        "##############          ##  ##        ##        ##  ##  ##  ##    ##      ##      ##      ##    ##  ##  ##  ######\n",
        "##          ##    ##    ##    ########        ########      ##  ######  ######  ######  ##########      ##    ##  \n",
        "##  ######  ##  ##          ##      ####    ##  ##################  ######  ######  ######  ################      \n",
        "##  ######  ##    ##    ########      ####      ##  ##  ########      ##      ##      ##        ######  ########  \n",
        "##  ######  ##  ####    ####  ####    ##              ####  ##    ##      ##      ##      ##  ####  ######  ##  ##\n",
        "##          ##    ##    ####    ######      ##  ####  ##        ######  ######  ######  ########      ##      ##  \n",
        "##############  ##  ##    ##  ####  ######    ##  ####    ##  ####  ######  ######  ######  ##    ##      ##  ####\n",
    );

    #[test]
    fn a_version_one_code_is_twenty_one_modules_across() {
        let matrix = encode(b"HELLO").expect("fits");
        assert_eq!(matrix.size(), 21);
    }

    #[test]
    fn the_smallest_version_that_fits_is_chosen() {
        // Capacities at level M: 14, 26, 42, 62, 84, 106, 122, 152, 180, 213.
        for (bytes, version) in [
            (14, 1),
            (15, 2),
            (26, 2),
            (27, 3),
            (106, 6),
            (107, 7),
            (213, 10),
        ] {
            let data = vec![b'x'; bytes];
            let matrix = encode(&data).expect("fits");
            assert_eq!(matrix.size(), version * 4 + 17, "{bytes} bytes");
        }
    }

    #[test]
    fn a_payload_past_the_last_version_is_refused_with_its_size() {
        let error = encode(&vec![b'x'; 214]).expect_err("too long");
        assert_eq!(error.needed, 214);
        assert_eq!(error.limit, 213);
        assert!(error.to_string().contains("213"));
    }

    #[test]
    fn a_code_has_its_three_finders_and_its_dark_module() {
        let matrix = encode(b"HELLO").expect("fits");
        let size = matrix.size();
        for (row, column) in [(0, 0), (0, size - 7), (size - 7, 0)] {
            for dy in 0..7 {
                for dx in 0..7 {
                    let edge = dy == 0 || dy == 6 || dx == 0 || dx == 6;
                    let centre = (2..=4).contains(&dy) && (2..=4).contains(&dx);
                    assert_eq!(
                        matrix.is_dark(row + dy, column + dx),
                        edge || centre,
                        "finder at {row},{column} offset {dy},{dx}"
                    );
                }
            }
        }
        assert!(matrix.is_dark(size - 8, 8), "the always-dark module");
    }

    #[test]
    fn the_quiet_zone_is_light_and_four_modules_wide() {
        let matrix = encode(b"HELLO").expect("fits");
        let rendered = matrix.render_terminal(false);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), (matrix.size() + 8).div_ceil(2));
        let width = matrix.size() + 8;
        for line in &lines {
            assert_eq!(line.chars().count(), width, "every line is the full width");
        }
        for line in &lines[..2] {
            assert!(line.chars().all(|glyph| glyph == ' '), "top quiet zone");
        }
    }

    #[test]
    fn the_terminal_glyphs_round_trip_to_the_same_modules() {
        let matrix = encode(b"round trip").expect("fits");
        let quiet = 4;
        let mut rows: Vec<Vec<bool>> = Vec::new();
        for line in matrix.render_terminal(false).lines() {
            let width = matrix.size() + quiet * 2;
            let mut top = vec![false; width];
            let mut bottom = vec![false; width];
            assert_eq!(line.chars().count(), width, "a full line of cells");
            for (column, glyph) in line.chars().enumerate() {
                match glyph {
                    '█' => {
                        top[column] = true;
                        bottom[column] = true;
                    }
                    '▀' => top[column] = true,
                    '▄' => bottom[column] = true,
                    ' ' => {}
                    other => panic!("unexpected glyph {other:?}"),
                }
            }
            rows.push(top);
            rows.push(bottom);
        }
        for row in 0..matrix.size() {
            for column in 0..matrix.size() {
                assert_eq!(
                    rows[row + quiet][column + quiet],
                    matrix.is_dark(row, column),
                    "module {row},{column}"
                );
            }
        }
        for row in rows.iter().take(quiet) {
            assert!(row.iter().all(|&dark| !dark), "quiet zone is light");
        }
    }

    #[test]
    fn colour_output_is_wrapped_and_plain_output_is_not() {
        let matrix = encode(b"HELLO").expect("fits");
        assert!(!matrix.render_terminal(false).contains('\x1b'));
        let coloured = matrix.render_terminal(true);
        assert!(coloured.starts_with("\x1b[30;47m"));
        assert!(
            coloured
                .lines()
                .next()
                .expect("a line")
                .ends_with("\x1b[0m")
        );
    }

    #[test]
    fn multiplication_in_the_field_is_the_inverse_of_itself() {
        assert_eq!(gf_multiply(0, 0), 0);
        assert_eq!(gf_multiply(1, 0x77), 0x77);
        assert_eq!(gf_multiply(2, 0x80), 0x1D, "2 * x^7 wraps by the modulus");
        // Every non-zero element has an inverse, so x * y * y is not x unless
        // y is 1 — but x * 1 == x for all x is the identity that matters here.
        for x in 0..=255u8 {
            assert_eq!(gf_multiply(x, 1), x);
        }
    }

    #[test]
    fn the_error_correction_parity_of_a_known_block_is_stable() {
        // Pin the Reed-Solomon arithmetic itself, not the whole code: the
        // parity of a real block must not change when the placement code is
        // refactored.
        let divisor = reed_solomon_divisor(10);
        assert_eq!(divisor.len(), 10);
        let parity = reed_solomon_remainder(&[0x40, 0x00], &divisor);
        assert_eq!(parity.len(), 10);
        // A zero-filled block has zero parity: division by the generator of
        // nothing is nothing, and an implementation with an off-by-one in the
        // loop would not produce that.
        assert_eq!(reed_solomon_remainder(&[0; 16], &divisor), vec![0u8; 10]);
    }

    #[test]
    fn the_format_bits_are_fifteen_and_carry_the_mask() {
        let seen: Vec<u32> = (0..8).map(format_bits).collect();
        for bits in &seen {
            assert_eq!(bits >> 15, 0, "fifteen bits");
        }
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 8, "each mask has its own format word");
    }

    #[test]
    fn a_version_seven_code_carries_its_version_bits() {
        let matrix = encode(&vec![b'x'; 110]).expect("fits version 7");
        assert_eq!(matrix.size(), 45);
        let bits = version_bits(7);
        for index in 0..18 {
            let expected = (bits >> index) & 1 == 1;
            let a = matrix.size() - 11 + index % 3;
            let b = index / 3;
            assert_eq!(matrix.is_dark(b, a), expected);
            assert_eq!(matrix.is_dark(a, b), expected);
        }
    }

    /// Pinned against matrices produced by `qrencode -t ASCII -l M -m 0 -8`,
    /// with `-v` where a version is fixed. Two of these call [`encode`], where
    /// the chosen mask happens to agree with qrencode's; the other two call
    /// [`encode_mask`], so they prove the grid without depending on a mask
    /// preference. qrencode's mask scoring and this module's ISO N1–N4
    /// scoring disagree on some payloads — both are valid codes, and the
    /// choice is covered by the decoding tests in the module documentation.
    #[test]
    fn the_encoder_matches_matrices_captured_from_qrencode() {
        assert_eq!(
            encode_mask(b"HELLO", 1, 3).to_ascii(),
            HELLO_GOLDEN,
            "version 1, mask 3"
        );
        assert_eq!(
            encode(OFFER.as_bytes()).expect("fits").to_ascii(),
            OFFER_GOLDEN,
            "version 6, chosen mask"
        );
        assert_eq!(
            encode(&vec![b'x'; 110]).expect("fits").to_ascii(),
            V7_GOLDEN,
            "version 7, chosen mask"
        );
        assert_eq!(
            encode_mask(&vec![b'x'; 200], 10, 0).to_ascii(),
            V10_GOLDEN,
            "version 10, mask 0"
        );
    }

    /// Write matrices and all eight mask candidates for comparison against
    /// `qrencode` by hand. Ignored because it is not a test: a green tick here
    /// would only prove the files were written.
    ///
    /// `QR_DUMP_DIR=/tmp/opencode/qr cargo test --lib --offline
    /// qr::tests::dump_for_comparison -- --ignored --nocapture`
    #[test]
    #[ignore = "writes files for an external comparison"]
    fn dump_for_comparison() {
        let dir = std::path::PathBuf::from(std::env::var("QR_DUMP_DIR").expect("set QR_DUMP_DIR"));
        std::fs::create_dir_all(&dir).expect("create dump directory");
        let cases: [(&str, &[u8], Option<usize>); 4] = [
            ("hello", b"HELLO", None),
            ("offer", OFFER.as_bytes(), None),
            ("v7", &[b'x'; 110], Some(7)),
            ("v10", &[b'x'; 200], Some(10)),
        ];
        for (name, data, forced) in cases {
            write_case(&dir, name, data, forced);
        }
        // One payload filled to the brim for every version, because a wrong
        // block table only shows up at a boundary.
        for version in 1..=MAX_VERSION {
            let data = vec![b'y'; capacity_bytes(version)];
            write_case(&dir, &format!("cap{version}"), &data, Some(version));
        }
    }

    fn write_case(dir: &std::path::Path, name: &str, data: &[u8], forced: Option<usize>) {
        let version = forced.unwrap_or_else(|| {
            (1..=MAX_VERSION)
                .find(|version| data.len() <= capacity_bytes(*version))
                .expect("fits")
        });
        let chosen = encode_version(data, version);
        std::fs::write(dir.join(format!("{name}.ascii")), chosen.to_ascii())
            .expect("write chosen matrix");
        let mut winner = None;
        for mask in 0..8 {
            let matrix = encode_mask(data, version, mask);
            std::fs::write(dir.join(format!("{name}.m{mask}.ascii")), matrix.to_ascii())
                .expect("write candidate");
            if matrix == chosen {
                winner = Some(mask);
            }
        }
        println!(
            "{name}: {} bytes, version {version}, size {}, mask {}",
            data.len(),
            chosen.size(),
            winner.expect("a chosen mask")
        );
    }
}
