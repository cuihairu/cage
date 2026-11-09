//! Convention engine tests (X1, design §51): grammar, vocabulary, anchors,
//! the config gate, and the A1 helpers. The script runner and response gate
//! run against a real subprocess in the cage-source-excel adapter tests.

use super::*;
use std::collections::HashSet;

fn grid(rows: &[&[&str]]) -> Grid {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    if c.is_empty() {
                        CellValue::Null
                    } else {
                        CellValue::Str((*c).to_string())
                    }
                })
                .collect()
        })
        .collect()
}

fn meta_of(rows: &[&[&str]], header_row: usize, territory: &[usize]) -> TableMeta {
    decode(
        &grid(rows),
        header_row,
        territory,
        0,
        0,
        DEFAULT_MARKER,
        &HashSet::new(),
        "config/Skill.xlsx",
        "Skill",
    )
    .expect("decode must succeed")
}

mod derive;
mod grammar;
mod script;
