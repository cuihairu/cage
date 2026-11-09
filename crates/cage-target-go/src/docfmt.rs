//! gofmt-shaped output model: cells, lines and tabwriter sections, plus the break-term estimation helpers (split out of lib.rs, LAY-009).

use super::Member;

// ---------------------------------------------------------------------------
// gofmt-shaped output model: cells, lines and tabwriter sections
// ---------------------------------------------------------------------------

/// Cell terminator — the separator go/printer feeds to text/tabwriter after
/// each cell. The final cell of every line carries the line break itself.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Term {
    /// '\t' — hard tab; the column can never be discarded (indent columns).
    Htab,
    /// '\v' — soft tab; a column of only-empty vtab cells is discarded.
    Vtab,
    /// '\n' — line break.
    Nl,
    /// '\f' — formfeed: line break that also ends the alignment section.
    Ff,
}

/// One tab-separated cell (text plus its terminator).
pub(crate) struct Cell {
    text: String,
    term: Term,
}

impl Cell {
    pub(crate) fn new(text: impl Into<String>, term: Term) -> Self {
        Self {
            text: text.into(),
            term,
        }
    }

    /// An empty hard-tab cell — one indentation level.
    pub(crate) fn indent() -> Self {
        Self::new(String::new(), Term::Htab)
    }

    /// Cell width in runes (tabwriter counts runes, not bytes).
    fn width(&self) -> usize {
        self.text.chars().count()
    }
}

/// A gofmt-shaped document: lines of tab-separated cells, split into the
/// sections the tabwriter would flush (at formfeeds and at lines that carry
/// no alignment cells at all).
#[derive(Default)]
pub(crate) struct Doc {
    sections: Vec<Vec<Vec<Cell>>>,
    current: Vec<Vec<Cell>>,
}

impl Doc {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Append a finished line. Lines that carry no alignment cells (a single
    /// cell), or that end in a formfeed, terminate the current section —
    /// exactly the conditions under which text/tabwriter flushes.
    pub(crate) fn line(&mut self, cells: Vec<Cell>) {
        let ends_section = cells.len() == 1 || cells[cells.len() - 1].term == Term::Ff;
        self.current.push(cells);
        if ends_section {
            self.sections.push(std::mem::take(&mut self.current));
        }
    }

    /// A single-cell line: no tabs, so the tabwriter flushes immediately.
    pub(crate) fn raw(&mut self, text: impl Into<String>) {
        self.line(vec![Cell::new(text, Term::Nl)]);
    }
    /// Render every section through the tabwriter port; each line ends in
    /// exactly one newline, so the file is newline-terminated.
    pub(crate) fn finish(mut self) -> String {
        if !self.current.is_empty() {
            self.sections.push(std::mem::take(&mut self.current));
        }
        let mut out = String::new();
        for section in &self.sections {
            format_section(section, 0, section.len(), &mut Vec::new(), &mut out);
        }
        out
    }
}

/// Port of text/tabwriter's `format` with gofmt's configuration
/// (minwidth 0, tabwidth 8, padding 1, padchar ' ', `DiscardEmptyColumns` |
/// `TabIndent`): contiguous runs of lines sharing a cell in a column form a
/// block; every column cell is padded (with spaces) to the block's widest
/// cell plus one, all-empty soft-tab columns are discarded, and the leading
/// empty indent cells are padded with tabs.
pub(crate) fn format_section(
    lines: &[Vec<Cell>],
    line0: usize,
    line1: usize,
    widths: &mut Vec<usize>,
    out: &mut String,
) {
    let mut line0 = line0;
    let mut this = line0;
    while this < line1 {
        let column = widths.len();
        if column + 1 >= lines[this].len() {
            this += 1;
            continue;
        }
        // print unprinted lines until beginning of block
        write_section_lines(lines, line0, this, widths, out);
        line0 = this;
        // column block: the contiguous run of lines with a cell at `column`
        let mut width = 0; // minwidth
        let mut discardable = true;
        while this < line1 {
            let line = &lines[this];
            if column + 1 >= line.len() {
                break;
            }
            let c = &line[column];
            width = width.max(c.width() + 1); // + padding
            if c.width() > 0 || c.term == Term::Htab {
                discardable = false;
            }
            this += 1;
        }
        if discardable {
            width = 0;
        }
        widths.push(width);
        format_section(lines, line0, this, widths, out);
        widths.pop();
        line0 = this;
        this += 1; // the line that broke the block lacks this cell too
    }
    write_section_lines(lines, line0, line1, widths, out);
}

/// Port of text/tabwriter's `writeLines` for one block of lines.
pub(crate) fn write_section_lines(
    lines: &[Vec<Cell>],
    line0: usize,
    line1: usize,
    widths: &[usize],
    out: &mut String,
) {
    for line in &lines[line0..line1] {
        let mut use_tabs = true; // TabIndent: tabs for leading empty cells
        for (j, c) in line.iter().enumerate() {
            if c.text.is_empty() {
                if j < widths.len() {
                    write_padding(c.width(), widths[j], use_tabs, out);
                }
            } else {
                use_tabs = false;
                out.push_str(&c.text);
                if j < widths.len() {
                    write_padding(c.width(), widths[j], false, out);
                }
            }
        }
        out.push('\n');
    }
}

/// Port of text/tabwriter's `writePadding`: tab padding rounds the cell up to
/// a multiple of the tab width; space padding pads exactly to the cell width.
pub(crate) fn write_padding(textw: usize, cellw: usize, use_tabs: bool, out: &mut String) {
    if use_tabs {
        let cellw = cellw.div_ceil(8) * 8;
        let n = cellw - textw;
        for _ in 0..n.div_ceil(8) {
            out.push('\t');
        }
    } else {
        for _ in 0..(cellw - textw) {
            out.push(' ');
        }
    }
}

/// Port of go/printer exprList's section-break heuristic for multi-line
/// composite literals: the line break after an entry becomes a formfeed
/// (splitting the alignment) when the key sizes involved differ enough
/// (smallSize 40, threshold r = 2.5, log2ish/exp2ish geometric mean).
/// The last entry's break is always the terminating formfeed.
pub(crate) fn literal_break_terms(members: &[&Member]) -> Vec<Term> {
    let n = members.len();
    let mut terms = vec![Term::Nl; n];
    // nodeSize of the whole `Key: Value` pair, in bytes; 0 = too big to fit.
    let pair_size = |m: &Member| -> i64 {
        let expr = m.default_expr.as_deref().unwrap_or_default();
        (m.ident.len() + 2 + expr.len()) as i64
    };
    let key_size = |m: &Member| -> i64 {
        if pair_size(m) > 1_000_000 {
            0
        } else {
            m.ident.len() as i64
        }
    };
    let mut log2sum = 0.0f64;
    let mut count = 0usize;
    let mut prev_size = 0i64;
    for i in 0..n {
        let size = key_size(members[i]);
        if i > 0 {
            let mut use_ff = true;
            if prev_size > 0 && size > 0 {
                if count == 0 || (prev_size <= 40 && size <= 40) {
                    use_ff = false;
                } else {
                    let geomean = exp2ish(log2sum / count as f64);
                    let ratio = size as f64 / geomean;
                    use_ff = 2.5 * ratio <= 1.0 || 2.5 <= ratio;
                }
            }
            terms[i - 1] = if use_ff { Term::Ff } else { Term::Nl };
            // a new section restarts the geometric-mean accumulation
            if use_ff {
                log2sum = 0.0;
                count = 0;
            }
        }
        if size > 0 {
            log2sum += log2ish(size as f64);
            count += 1;
        }
        prev_size = size;
    }
    if n > 0 {
        terms[n - 1] = Term::Ff; // terminating comma needs a line break
    }
    terms
}

/// Crude log₂ ported from go/printer (math.go) — identical on all platforms.
pub(crate) fn log2ish(x: f64) -> f64 {
    let (f, e) = frexp(x);
    f64::from(e) + 2.0 * (f - 1.0)
}

/// Crude 2^x ported from go/printer (math.go).
pub(crate) fn exp2ish(x: f64) -> f64 {
    let n = x.floor();
    let f = x - n;
    (1.0 + f) * (2.0f64).powi(n as i32)
}

/// frexp: x = f * 2^e with 0.5 <= f < 1 (loop form — exact for powers of 2,
/// and our inputs are small positive sizes).
pub(crate) fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let mut f = x;
    let mut e = 0i32;
    while f.abs() >= 1.0 {
        f *= 0.5;
        e += 1;
    }
    while f.abs() < 0.5 {
        f *= 2.0;
        e -= 1;
    }
    (f, e)
}

// ---------------------------------------------------------------------------
// identifiers, types, literals
// ---------------------------------------------------------------------------
