use super::*;
use cage_core::convention::{CellValue, Preset};
use cage_core::error::codes::source_convention::{E2201, E2203, E2207};
#[cfg(unix)]
use cage_core::error::codes::source_convention::{E2202, E2204, E2205, E2206};
use cage_core::error::codes::{internal, parse};
use cage_core::schema::FieldType;
use cage_core::value::Value;
use calamine::{Data, Range};
use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_col_index_to_letter() {
    assert_eq!(ExcelSourceAdapter::col_index_to_letter(0), "A");
    assert_eq!(ExcelSourceAdapter::col_index_to_letter(1), "B");
    assert_eq!(ExcelSourceAdapter::col_index_to_letter(25), "Z");
    assert_eq!(ExcelSourceAdapter::col_index_to_letter(26), "AA");
    assert_eq!(ExcelSourceAdapter::col_index_to_letter(27), "AB");
    assert_eq!(ExcelSourceAdapter::col_index_to_letter(51), "AZ");
    assert_eq!(ExcelSourceAdapter::col_index_to_letter(52), "BA");
}

#[test]
fn test_sanitize_table_name() {
    assert_eq!(ExcelSourceAdapter::sanitize_table_name("Item"), "Item");
    assert_eq!(
        ExcelSourceAdapter::sanitize_table_name("Monster Data"),
        "Monster_Data"
    );
    assert_eq!(
        ExcelSourceAdapter::sanitize_table_name("Sheet-1"),
        "Sheet_1"
    );
    assert_eq!(ExcelSourceAdapter::sanitize_table_name("123"), "123");
}

#[test]
fn test_cell_to_cage_value() {
    use calamine::{Data, ExcelDateTime, ExcelDateTimeType};

    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::Empty),
        Value::Null
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::String("hello".to_string())),
        Value::String("hello".to_string())
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::Int(42)),
        Value::Int(42)
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::Float(3.14)),
        Value::Float(3.14)
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::Float(100.0)),
        Value::Int(100)
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::Bool(true)),
        Value::Bool(true)
    );
    // Excel serial date -> ISO-8601 (2025/10/13 12:59:02.400 = 45943.541),
    // not the `ExcelDateTime` struct dump.
    let dt = ExcelDateTime::new(45943.541, ExcelDateTimeType::DateTime, false);
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::DateTime(dt)),
        Value::String("2025-10-13T12:59:02.400".to_string())
    );
    // Whole-second datetime drops the fractional part.
    let whole = ExcelDateTime::new(45943.5, ExcelDateTimeType::DateTime, false);
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::DateTime(whole)),
        Value::String("2025-10-13T12:00:00".to_string())
    );
    // Duration format cell renders as an ISO-8601 duration.
    let dur = ExcelDateTime::new(1.5, ExcelDateTimeType::TimeDelta, false);
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::DateTime(dur)),
        Value::String("PT36H0M0.000S".to_string())
    );
}

#[test]
fn test_infer_string_type() {
    assert_eq!(ExcelSourceAdapter::infer_string_type("42"), Value::Int(42));
    assert_eq!(
        ExcelSourceAdapter::infer_string_type("3.14"),
        Value::Float(3.14)
    );
    assert_eq!(
        ExcelSourceAdapter::infer_string_type("true"),
        Value::Bool(true)
    );
    assert_eq!(
        ExcelSourceAdapter::infer_string_type("false"),
        Value::Bool(false)
    );
    assert_eq!(
        ExcelSourceAdapter::infer_string_type("hello"),
        Value::String("hello".to_string())
    );
}

#[test]
fn test_source_adapter_trait() {
    let adapter = ExcelSourceAdapter::default();
    assert_eq!(adapter.supported_extensions(), &["xlsx", "xlsm", "ods"]);
    assert_eq!(adapter.format_name(), "Excel");
}

/// T2.4 end-to-end: vertical merge fill + sheet-grid addressing.
/// Fixture (`tests/fixtures/gen_merged_cells.py`) has its used range start
/// at B2 and merges D3:D4 with no value stored in D4.
#[test]
fn test_merged_cells_fill_and_sheet_addressing() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merged_cells.xlsx");
    let adapter = ExcelSourceAdapter::default();
    let doc = adapter.parse_file(&path).expect("fixture must parse");

    let table = doc.tables.get("Items").expect("Items table");
    assert_eq!(table.rows.len(), 3, "all three data rows present");

    // Primary key heuristic picks the id column
    assert_eq!(table.rows[0].primary_key, vec![Value::Int(1001)]);
    assert_eq!(table.rows[1].primary_key, vec![Value::Int(1002)]);

    // Vertical merge: D4 carries no value in the file, takes D3's anchor
    assert_eq!(
        table.rows[0].fields["type"].value,
        Value::String("weapon".to_string())
    );
    assert_eq!(
        table.rows[1].fields["type"].value,
        Value::String("weapon".to_string()),
        "covered cell D4 must be filled from anchor D3"
    );
    assert_eq!(
        table.rows[2].fields["type"].value,
        Value::String("material".to_string())
    );

    // Sheet-grid addressing (G27-style): used range starts at B2, so the
    // anchor row of the merge is sheet row 3, the covered row is sheet row 4
    // and the type column is D — not range-relative offsets.
    let anchor_loc = &table.rows[0].fields["type"].location;
    assert_eq!(anchor_loc.row, Some(3));
    assert_eq!(anchor_loc.column.as_deref(), Some("D"));
    let covered_loc = &table.rows[1].fields["type"].location;
    assert_eq!(covered_loc.row, Some(4));
    assert_eq!(covered_loc.column.as_deref(), Some("D"));

    // Empty covered cells elsewhere stay Null; plain cells unaffected
    assert_eq!(table.rows[1].fields["note"].value, Value::Null);
    assert_eq!(
        table.rows[2].fields["note"].value,
        Value::String("rare".to_string())
    );
    assert_eq!(table.rows[1].fields["price"].value, Value::Int(250));
}

/// Horizontal merge: covered cell takes the anchor to its left; anchors,
/// uncovered cells and anchors outside the used range stay raw.
#[test]
fn test_resolve_cell_horizontal_and_out_of_range() {
    // Sheet rows 3..5, columns B..F — start deliberately not A1
    let mut range: Range<Data> = Range::new((2, 1), (4, 5));
    range.set_value((3, 4), Data::String("wide".to_string())); // E4 anchor

    let mut anchors = HashMap::new();
    anchors.insert((3, 5), (3, 4)); // F4 covered by merge E4:F4

    // F4 (rel 1,4) is covered → E4's value
    let covered = ExcelSourceAdapter::resolve_cell(&range, &anchors, 1, 4, &Data::Empty);
    assert_eq!(covered, &Data::String("wide".to_string()));

    // E4 itself is the anchor → raw value kept
    let anchor_raw = Data::String("wide".to_string());
    let anchor = ExcelSourceAdapter::resolve_cell(&range, &anchors, 1, 3, &anchor_raw);
    assert_eq!(anchor, &anchor_raw);

    // B3 not covered → raw value kept
    let raw = ExcelSourceAdapter::resolve_cell(&range, &anchors, 0, 0, &Data::Int(7));
    assert_eq!(raw, &Data::Int(7));

    // Merge anchor sits before range.start() → no bogus substitution
    let mut outside = HashMap::new();
    outside.insert((3, 5), (1, 0)); // anchor at sheet col A (before B)
    let kept = ExcelSourceAdapter::resolve_cell(&range, &outside, 1, 4, &Data::Int(9));
    assert_eq!(kept, &Data::Int(9));
}

// ---- Hand-built minimal .xlsx fixtures ---------------------------------
//
// calamine opens a workbook from four zip members: `_rels/.rels`,
// `xl/workbook.xml`, `xl/_rels/workbook.xml.rels` and the sheet XML
// (sharedStrings/styles/[Content_Types] are optional). The builders below
// assemble a STORED zip — no compression, hand-rolled CRC-32 — so the
// sheet-read error paths can be exercised with inline bytes.

/// CRC-32 (IEEE, reflected) of `data`; zip readers verify it even for
/// STORED entries.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Assemble a STORED (uncompressed) zip archive from name/content pairs.
fn build_stored_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for &(name, contents) in entries {
        let data = contents.as_bytes();
        let crc = crc32(data).to_le_bytes();
        let size = (data.len() as u32).to_le_bytes();
        let name_len = (name.len() as u16).to_le_bytes();
        let offset = (out.len() as u32).to_le_bytes();

        // Local file header
        out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&[0; 2]); // flags
        out.extend_from_slice(&[0; 2]); // method: stored
        out.extend_from_slice(&[0; 2]); // mod time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // mod date (1980-01-01)
        out.extend_from_slice(&crc);
        out.extend_from_slice(&size); // compressed size
        out.extend_from_slice(&size); // uncompressed size
        out.extend_from_slice(&name_len);
        out.extend_from_slice(&[0; 2]); // extra len
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);

        // Central directory record
        central.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]);
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&[0; 2]); // flags
        central.extend_from_slice(&[0; 2]); // method
        central.extend_from_slice(&[0; 2]); // mod time
        central.extend_from_slice(&0x21u16.to_le_bytes()); // mod date
        central.extend_from_slice(&crc);
        central.extend_from_slice(&size);
        central.extend_from_slice(&size);
        central.extend_from_slice(&name_len);
        central.extend_from_slice(&[0; 2]); // extra len
        central.extend_from_slice(&[0; 2]); // comment len
        central.extend_from_slice(&[0; 2]); // disk number
        central.extend_from_slice(&[0; 2]); // internal attrs
        central.extend_from_slice(&[0; 4]); // external attrs
        central.extend_from_slice(&offset);
        central.extend_from_slice(name.as_bytes());
    }

    let cd_offset = (out.len() as u32).to_le_bytes();
    let cd_size = (central.len() as u32).to_le_bytes();
    out.extend_from_slice(&central);
    // End of central directory
    out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
    out.extend_from_slice(&[0; 2]); // disk number
    out.extend_from_slice(&[0; 2]); // cd start disk
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size);
    out.extend_from_slice(&cd_offset);
    out.extend_from_slice(&[0; 2]); // comment len
    out
}

/// One-sheet workbook ("Data"); `sheet_xml: None` omits the sheet part
/// from the zip (sheet referenced but not shipped).
fn minimal_xlsx(sheet_xml: Option<&str>) -> Vec<u8> {
    let package_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let workbook_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;

    let mut entries = vec![
        ("_rels/.rels", package_rels),
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", workbook_rels),
    ];
    if let Some(xml) = sheet_xml {
        entries.push(("xl/worksheets/sheet1.xml", xml));
    }
    build_stored_zip(&entries)
}

/// Like [`minimal_xlsx`], but the caller supplies the workbook.xml (to
/// exercise sheet metadata such as visibility states) plus one XML part
/// per `rIdN` it declares.
fn minimal_xlsx_with_workbook(workbook_xml: &str, sheets: &[(&str, &str)]) -> Vec<u8> {
    use std::fmt::Write as _;
    let package_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let mut rel = String::from(
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
    );
    let mut entries = vec![
        ("_rels/.rels".to_string(), package_rels.to_string()),
        ("xl/workbook.xml".to_string(), workbook_xml.to_string()),
    ];
    for (n, (part, xml)) in sheets.iter().enumerate() {
        if n > 0 {
            let _ = write!(
                rel,
                r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/{part}"/>"#,
                n + 1
            );
        }
        entries.push((format!("xl/worksheets/{part}"), (*xml).to_string()));
    }
    let workbook_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rel}</Relationships>"#
    );
    entries.insert(2, ("xl/_rels/workbook.xml.rels".to_string(), workbook_rels));
    let refs: Vec<(&str, &str)> = entries
        .iter()
        .map(|(name, xml)| (name.as_str(), xml.as_str()))
        .collect();
    build_stored_zip(&refs)
}

/// A file in the system temp dir, removed on drop (the crate has no
/// tempfile dev-dependency).
struct TempWorkbook(PathBuf);

impl TempWorkbook {
    fn new(bytes: &[u8], file_name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "cage_excel_{}_{}_{}",
            std::process::id(),
            nanos,
            file_name
        ));
        std::fs::write(&path, bytes).expect("temp workbook write");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempWorkbook {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A non-Excel extension (or none at all) is rejected with E0001 before
/// any file is opened.
#[test]
fn test_unsupported_extension_rejected_before_open() {
    let adapter = ExcelSourceAdapter::default();

    for name in ["book.xls", "book"] {
        let diags = adapter.parse_file(Path::new(name)).unwrap_err();
        assert_eq!(diags.errors().len(), 1);
        assert_eq!(diags.errors()[0].code, parse::E0001);
        assert!(diags.errors()[0]
            .message
            .contains("Unsupported Excel format"));
        assert!(diags.errors()[0].source.contains(name));
    }
}

/// Bytes that are not a zip at all fail workbook opening with E9902.
#[test]
fn test_open_failure_reports_io_error() {
    let wb = TempWorkbook::new(b"definitely not a zip", "corrupt.xlsx");

    let adapter = ExcelSourceAdapter::default();
    let diags = adapter.parse_file(wb.path()).unwrap_err();

    assert_eq!(diags.errors().len(), 1);
    assert_eq!(diags.errors()[0].code, internal::E9902);
    assert!(diags.errors()[0]
        .message
        .contains("Failed to open workbook"));
    assert!(diags.errors()[0].source.contains("corrupt.xlsx"));
}

/// A header row that names the same column twice is an authoring error,
/// not a silent overwrite: the flat path stores fields by name, so the
/// later column would replace the earlier one and its values would be
/// lost without any report (audit finding 2026-10-29).
#[test]
fn test_duplicate_header_names_are_an_error_not_a_silent_overwrite() {
    let sheet = sheet_xml(
        &[&["id", "name", "name"][..], &["1", "a", "b"][..]],
        1,
        "A1:C2",
    );
    let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "dup.xlsx");
    let adapter = ExcelSourceAdapter::default();

    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert!(diags.has_errors(), "duplicate header must fail the parse");
    let errors = diags.errors();
    let err = errors
        .iter()
        .find(|d| d.code == parse::E0001)
        .expect("an E0001 diagnostic names the duplicate");
    assert!(
        err.message.contains("Duplicate column name 'name'"),
        "message names the duplicate: {}",
        err.message
    );
    assert!(
        err.message.contains("column C"),
        "message anchors the later column: {}",
        err.message
    );
    assert!(err.source.contains("dup.xlsx"), "source: {}", err.source);
}

/// The structural path (`##var` / marker rows) stores fields by group
/// name too, so a duplicate top-level header is the same silent
/// overwrite the flat path already reports (audit finding 2026-10-29).
#[test]
fn test_duplicate_top_level_headers_are_an_error_in_structural_path() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "name", "weapon", "", "name", ""][..],
            &["##+", "", "", "atk", "def", "", ""][..],
            &["##type", "Int32", "String", "Int32", "Int32", "", ""][..],
            &["", "1", "sword", "10", "5", "other", ""][..],
        ],
        &["D1:E1"],
        "structural-dup.xlsx",
    );

    let diags = luban_adapter().parse_file(wb.path()).unwrap_err();
    assert!(diags.has_errors(), "duplicate header must fail the parse");
    let errors = diags.errors();
    let err = errors
        .iter()
        .find(|d| d.code == parse::E0001)
        .expect("an E0001 diagnostic names the duplicate");
    assert!(
        err.message.contains("Duplicate column name 'name'"),
        "message names the duplicate: {}",
        err.message
    );
    assert!(
        err.message.contains("column F"),
        "message anchors the later column: {}",
        err.message
    );
    assert!(
        err.source.contains("structural-dup.xlsx"),
        "source: {}",
        err.source
    );
}

/// Two blank top-level `##var` headers in the structural path get the
/// same `col<n>` fallback the flat path already uses; they must not
/// collide as an empty-string duplicate (audit finding 2026-10-30).
#[test]
fn test_structural_blank_headers_get_unique_col_fallbacks() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "", "weapon", ""][..],
            &["##+", "", "", "atk", ""][..],
            &["##type", "Int32", "String", "Int32", "String"][..],
            &["", "1", "spare", "10", "extra"][..],
        ],
        &[],
        "structural-blank-headers.xlsx",
    );

    let doc = luban_adapter()
        .parse_file(wb.path())
        .expect("blank headers must not collide as an empty name");
    let table = doc.tables.get("Data").expect("Data table");
    let names: Vec<&str> = table.rows[0].fields.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["id", "col2", "weapon", "col4"],
        "blank headers get col<n> on their absolute column"
    );
    assert_eq!(
        table.rows[0].fields["col2"].value,
        Value::String("spare".to_string())
    );
    assert_eq!(
        table.rows[0].fields["col4"].value,
        Value::String("extra".to_string())
    );
    assert_eq!(
        table.rows[0].fields["weapon"].value,
        Value::Object(IndexMap::from([("atk".to_string(), Value::Int(10))]))
    );
}

/// A visible sheet with an empty used range is an authoring error, not a
/// silent skip: by the §3.1 ruling a visible sheet is a table, and hiding
/// a sheet is the documented way to say "not a table". The silent skip
/// lost the sheet without any report (audit finding 2026-10-29).
#[test]
fn test_empty_used_range_sheet_is_an_error_not_a_silent_skip() {
    let sheet = sheet_xml(&[], 1, "A1:A1");
    let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "empty.xlsx");
    let adapter = ExcelSourceAdapter::default();

    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert!(
        diags.has_errors(),
        "an empty visible sheet must fail the parse"
    );
    let errors = diags.errors();
    let err = errors
        .iter()
        .find(|d| d.code == parse::E0001)
        .expect("an E0001 diagnostic names the empty sheet");
    assert!(
        err.message.contains("empty used range"),
        "message names the empty range: {}",
        err.message
    );
    assert!(
        err.message.contains("Data"),
        "message names the sheet: {}",
        err.message
    );
}

/// Gaps in the used range: a missing header cell (B1) is named `col1` by
/// its absolute grid column, and an entirely absent middle row is skipped
/// by `skip_empty_rows` — leaving a sheet-row gap between data rows.
#[test]
fn test_handbuilt_xlsx_empty_header_cell_and_blank_row() {
    let sheet = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        r#"<dimension ref="A1:C4"/>"#,
        "<sheetData>",
        r#"<row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c><c r="C1" t="inlineStr"><is><t>name</t></is></c></row>"#,
        r#"<row r="2"><c r="A2"><v>1001</v></c><c r="C2" t="inlineStr"><is><t>Sword</t></is></c></row>"#,
        r#"<row r="4"><c r="A4"><v>1002</v></c><c r="C4" t="inlineStr"><is><t>Shield</t></is></c></row>"#,
        "</sheetData></worksheet>"
    );
    let wb = TempWorkbook::new(&minimal_xlsx(Some(sheet)), "gaps.xlsx");

    let adapter = ExcelSourceAdapter::default();
    let doc = adapter.parse_file(wb.path()).expect("workbook parses");

    let table = doc.tables.get("Data").expect("Data table");
    assert_eq!(table.rows.len(), 2, "blank sheet row 3 is skipped");

    // Header row: A1 "id", missing B1 → generated col1, C1 "name"
    let names: Vec<&str> = table.rows[0].fields.keys().map(String::as_str).collect();
    assert_eq!(names, ["id", "col1", "name"]);
    assert_eq!(table.rows[0].fields["id"].value, Value::Int(1001));
    assert_eq!(table.rows[0].fields["col1"].value, Value::Null);
    assert_eq!(
        table.rows[0].fields["name"].value,
        Value::String("Sword".to_string())
    );

    // col1 addresses sheet column B; the id column drives the primary key
    assert_eq!(
        table.rows[0].fields["col1"].location.column.as_deref(),
        Some("B")
    );
    assert_eq!(table.rows[0].fields["col1"].location.row, Some(2));
    assert_eq!(table.rows[0].primary_key, vec![Value::Int(1001)]);

    // The skipped blank row leaves a sheet-row gap before the next datum
    assert_eq!(table.rows[1].fields["id"].value, Value::Int(1002));
    assert_eq!(table.rows[1].location.row, Some(4));
}

/// A malformed `<mergeCell ref>` warns but does not abort the sheet: the
/// merge-fill map stays empty and the data is still tabulated.
#[test]
fn test_handbuilt_xlsx_bad_merge_ref_still_parses() {
    let sheet = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        r#"<dimension ref="A1:B2"/>"#,
        "<sheetData>",
        r#"<row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c><c r="B1" t="inlineStr"><is><t>name</t></is></c></row>"#,
        r#"<row r="2"><c r="A2"><v>1</v></c><c r="B2" t="inlineStr"><is><t>Sword</t></is></c></row>"#,
        "</sheetData>",
        r#"<mergeCells count="1"><mergeCell ref="not-a-ref"></mergeCell></mergeCells>"#,
        "</worksheet>"
    );
    let wb = TempWorkbook::new(&minimal_xlsx(Some(sheet)), "badmerge.xlsx");

    let adapter = ExcelSourceAdapter::default();
    let doc = adapter
        .parse_file(wb.path())
        .expect("merge-table failure is a warning");

    let table = doc.tables.get("Data").unwrap();
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.rows[0].fields["id"].value, Value::Int(1));
}

/// A visible sheet with no cells yields an empty range. Since §3.1 a
/// visible sheet is a table, so this is an authoring error — it used to be
/// skipped with no diagnostic at all (audit finding 2026-10-29).
#[test]
fn test_handbuilt_xlsx_empty_sheet_is_an_error() {
    let sheet = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        "<sheetData></sheetData></worksheet>"
    );
    let wb = TempWorkbook::new(&minimal_xlsx(Some(sheet)), "emptysheet.xlsx");

    let adapter = ExcelSourceAdapter::default();
    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert!(diags.has_errors(), "an empty visible sheet is an error");
    assert!(
        diags.errors().iter().any(|d| d.code == parse::E0001),
        "the error is E0001"
    );
}

/// A workbook whose sheet part is missing from the zip: reading the sheet
/// fails, the sheet is skipped and no table is produced.
#[test]
fn test_handbuilt_xlsx_missing_sheet_part_skips_sheet() {
    let wb = TempWorkbook::new(&minimal_xlsx(None), "nosheet.xlsx");

    let adapter = ExcelSourceAdapter::default();
    let doc = adapter
        .parse_file(wb.path())
        .expect("missing sheet part is a warning");

    assert!(doc.tables.is_empty());
}

/// Sheet metadata: hidden and veryHidden sheets are authoring scratch —
/// drafts never ship, so only the visible sheet reaches the canonical
/// model (2026-10-12 ruling, design §3.1).
#[test]
fn test_handbuilt_xlsx_hidden_sheets_are_skipped() {
    let sheet = |cells: &str| {
        format!(
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
                r#"<dimension ref="A1:C2"/>"#,
                "<sheetData>",
                r#"<row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c><c r="C1" t="inlineStr"><is><t>name</t></is></c></row>"#,
                r#"<row r="2">"#,
                "{}",
                "</row></sheetData></worksheet>"
            ),
            cells
        )
    };
    let visible =
        sheet(r#"<c r="A2"><v>1001</v></c><c r="C2" t="inlineStr"><is><t>Sword</t></is></c>"#);
    let hidden =
        sheet(r#"<c r="A2"><v>9001</v></c><c r="C2" t="inlineStr"><is><t>DraftRow</t></is></c>"#);
    let very_hidden =
        sheet(r#"<c r="A2"><v>9002</v></c><c r="C2" t="inlineStr"><is><t>SecretRow</t></is></c>"#);
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Draft" sheetId="2" state="hidden" r:id="rId2"/><sheet name="Secrets" sheetId="3" state="veryHidden" r:id="rId3"/></sheets></workbook>"#;
    let bytes = minimal_xlsx_with_workbook(
        workbook,
        &[
            ("sheet1.xml", visible.as_str()),
            ("sheet2.xml", hidden.as_str()),
            ("sheet3.xml", very_hidden.as_str()),
        ],
    );
    let wb = TempWorkbook::new(&bytes, "staging.xlsx");

    let adapter = ExcelSourceAdapter::default();
    let doc = adapter.parse_file(wb.path()).expect("workbook parses");

    assert_eq!(doc.tables.len(), 1, "only the visible sheet is a table");
    let table = doc.tables.get("Data").expect("Data table");
    assert_eq!(table.rows.len(), 1);
    assert_eq!(
        table.rows[0].fields["id"].value,
        Value::Int(1001),
        "the shipped row is the visible sheet's, not a hidden sheet's"
    );
    assert!(doc.tables.get("Draft").is_none());
    assert!(doc.tables.get("Secrets").is_none());
}

/// An ODS workbook takes the non-Xlsx arm: sheet data is still read, but
/// the merged-cell fill (an Xlsx-only calamine API) is skipped.
#[test]
fn test_handbuilt_ods_workbook_parses_without_merge_fill() {
    // Rows must contain nothing but cell elements: any stray text node
    // inside a row is a parse error for the ODS reader.
    let content = concat!(
        r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0""#,
        r#" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0""#,
        r#" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
        "<office:body><office:spreadsheet>",
        r#"<table:table table:name="Data">"#,
        r#"<table:table-row><table:table-cell office:value-type="string"><text:p>id</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>name</text:p></table:table-cell></table:table-row>"#,
        r#"<table:table-row><table:table-cell office:value="1001"></table:table-cell><table:table-cell office:value-type="string"><text:p>Sword</text:p></table:table-cell></table:table-row>"#,
        "</table:table></office:spreadsheet></office:body></office:document-content>"
    );
    let ods = build_stored_zip(&[
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
        ("META-INF/manifest.xml", "<manifest:manifest/>"),
        ("content.xml", content),
    ]);
    let wb = TempWorkbook::new(&ods, "sheet.ods");

    let adapter = ExcelSourceAdapter::default();
    let doc = adapter.parse_file(wb.path()).expect("ods workbook parses");

    let table = doc.tables.get("Data").expect("Data table");
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.rows[0].fields["id"].value, Value::Int(1001));
    assert_eq!(
        table.rows[0].fields["name"].value,
        Value::String("Sword".to_string())
    );
}

/// `has_header: false` names columns from the absolute grid position and
/// treats every used row (including the former header row) as data; with
/// no `id` header each row's primary key falls back to its first field.
#[test]
fn test_has_header_false_generates_column_names() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merged_cells.xlsx");
    let adapter = ExcelSourceAdapter {
        has_header: false,
        ..Default::default()
    };
    let doc = adapter.parse_file(&path).expect("fixture must parse");

    let table = doc.tables.get("Items").unwrap();
    assert_eq!(table.rows.len(), 4, "header row is now a data row");

    // Used range starts at B → generated names are col1..col5
    let names: Vec<&str> = table.rows[0].fields.keys().map(String::as_str).collect();
    assert_eq!(names, ["col1", "col2", "col3", "col4", "col5"]);

    // No id header → primary key falls back to the first field
    assert_eq!(
        table.rows[0].primary_key,
        vec![Value::String("id".to_string())]
    );
    assert_eq!(table.rows[1].primary_key, vec![Value::Int(1001)]);

    // Locations keep sheet-grid columns (B..F) and rows (2..)
    assert_eq!(
        table.rows[0].fields["col1"].location.column.as_deref(),
        Some("B")
    );
    assert_eq!(
        table.rows[0].fields["col5"].location.column.as_deref(),
        Some("F")
    );
    assert_eq!(table.rows[0].location.row, Some(2));
}

/// Exotic cell conversions: whitespace-only strings, trimmed numeric
/// strings, non-finite floats, error cells, date and duration passthroughs.
#[test]
fn test_cell_to_cage_value_exotic() {
    use calamine::{CellErrorType, ExcelDateTime, ExcelDateTimeType};

    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::String("  ".to_string())),
        Value::Null
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::String(" 42 ".to_string())),
        Value::Int(42)
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::Float(f64::NAN)),
        Value::String("NaN".to_string())
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::Float(f64::INFINITY)),
        Value::String("inf".to_string())
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::Error(CellErrorType::Div0)),
        Value::String("#ERROR: Div0".to_string())
    );
    // Serial 44927 (1900 epoch) is 2023-01-01; the ISO-8601 rendering
    // replaces the old `ExcelDateTime` Debug dump (the data-plane defect
    // fixed 2026-10-28).
    let dt = ExcelDateTime::new(44_927.0, ExcelDateTimeType::DateTime, false);
    let converted = ExcelSourceAdapter::cell_to_cage_value(&Data::DateTime(dt));
    assert_eq!(
        converted,
        Value::String("2023-01-01T00:00:00".to_string()),
        "serial date renders as ISO-8601: {converted:?}"
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::DateTimeIso(
            "2024-01-02T03:04:05Z".to_string()
        )),
        Value::String("2024-01-02T03:04:05Z".to_string())
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_cage_value(&Data::DurationIso("PT1H".to_string())),
        Value::String("PT1H".to_string())
    );
}

/// `cell_to_string` renders every cell kind for header extraction.
#[test]
fn test_cell_to_string_matrix() {
    use calamine::{CellErrorType, ExcelDateTime, ExcelDateTimeType};

    assert_eq!(ExcelSourceAdapter::cell_to_string(&Data::Empty), "");
    assert_eq!(ExcelSourceAdapter::cell_to_string(&Data::Float(3.5)), "3.5");
    assert_eq!(ExcelSourceAdapter::cell_to_string(&Data::Int(7)), "7");
    assert_eq!(
        ExcelSourceAdapter::cell_to_string(&Data::Bool(true)),
        "true"
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_string(&Data::Error(CellErrorType::NA)),
        "#ERROR: NA"
    );
    let dt = ExcelDateTime::new(0.5, ExcelDateTimeType::TimeDelta, false);
    let rendered = ExcelSourceAdapter::cell_to_string(&Data::DateTime(dt));
    assert!(rendered.contains("0.5"), "got: {rendered}");
}

/// Unsigned integers past `i64::MAX` keep their width in string inference.
#[test]
fn test_infer_string_type_unsigned() {
    assert_eq!(
        ExcelSourceAdapter::infer_string_type("18446744073709551615"),
        Value::UInt(u64::MAX)
    );
    assert_eq!(
        ExcelSourceAdapter::infer_string_type("-9223372036854775808"),
        Value::Int(i64::MIN)
    );
}

/// An empty used range has no anchor cell: `resolve_cell` must return the
/// cell it was handed instead of indexing into the range.
#[test]
fn test_resolve_cell_empty_range_keeps_raw_cell() {
    let range = Range::<Data>::empty();
    let raw = Data::Int(5);

    let resolved = ExcelSourceAdapter::resolve_cell(&range, &HashMap::new(), 0, 0, &raw);

    assert_eq!(resolved, &raw);
}

/// The `SourceAdapter` trait forwards `parse_file` to the inherent method.
#[test]
fn test_source_adapter_trait_parse_delegation() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merged_cells.xlsx");

    let adapter = ExcelSourceAdapter::default();
    let doc = SourceAdapter::parse_file(&adapter, &path).expect("fixture must parse");
    assert!(doc.tables.contains_key("Items"));
}

// ---- Meta-row convention (X1, design §51) ------------------------------
//
// The presets read declarations from where the author put them. The
// workbooks are built with the helpers above so the fixtures are inline
// bytes — no generated file to check in, no OS-specific reader.

/// A worksheet part from a 2-D array of cell texts; an empty string omits
/// the cell, so a genuinely absent row or cell is representable.
fn sheet_xml(rows: &[&[&str]], first_row: u32, dimension: &str) -> String {
    sheet_xml_with_merges(rows, first_row, dimension, &[])
}

/// Like [`sheet_xml`], with `<mergeCell ref>` entries (for multi-level
/// headers: a parent column merged across its sub-column span).
fn sheet_xml_with_merges(
    rows: &[&[&str]],
    first_row: u32,
    dimension: &str,
    merges: &[&str],
) -> String {
    use std::fmt::Write as _;
    let mut xml = String::from(concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
    ));
    let _ = write!(xml, "<dimension ref=\"{dimension}\"/>");
    let _ = write!(xml, "<sheetData>");
    for (i, row) in rows.iter().enumerate() {
        let r = first_row + i as u32;
        let _ = write!(xml, "<row r=\"{r}\">");
        for (j, text) in row.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let col = ExcelSourceAdapter::col_index_to_letter(j);
            let _ = write!(
                xml,
                "<c r=\"{col}{r}\" t=\"inlineStr\"><is><t>{text}</t></is></c>",
                text = xml_escape(text),
            );
        }
        let _ = write!(xml, "</row>");
    }
    let _ = write!(xml, "</sheetData>");
    if !merges.is_empty() {
        let _ = write!(xml, "<mergeCells count=\"{}\">", merges.len());
        for cell in merges {
            let _ = write!(xml, "<mergeCell ref=\"{cell}\"></mergeCell>");
        }
        let _ = write!(xml, "</mergeCells>");
    }
    let _ = write!(xml, "</worksheet>");
    xml
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A workbook with a Luban-style meta block: three declaration rows, the
/// data header at row index 3, one data row.
fn skill_workbook() -> TempWorkbook {
    let sheet = sheet_xml(
        &[
            &["##type Int32", "String", "Enum<Rarity>"][..],
            &["##description", "武器名", "稀有度"][..],
            &["##enum", "", "common,rare,epic"][..],
            &["id", "name", "rarity"][..],
            &["1", "sword", "common"][..],
        ],
        1,
        "A1:C5",
    );
    TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "skill.xlsx")
}

/// The convention gate: with no `[source.excel]` declaration
/// declaration rows are ordinary data, exactly as before.
#[test]
fn test_convention_off_keeps_meta_rows_as_data() {
    let wb = skill_workbook();
    let adapter = ExcelSourceAdapter::default();

    let doc = adapter.parse_file(wb.path()).expect("workbook parses");
    let (table_name, table) = doc.tables.first().expect("one table");
    assert_eq!(table_name, "Data");
    assert_eq!(
        table.rows.len(),
        4,
        "the header row and every row below are data"
    );
    // The `##type Int32` row is the header row and the `##description`
    // row is ordinary data — nothing was decoded, nothing was dropped.
    assert!(
        table.rows[0]
            .fields
            .values()
            .any(|v| v.value == Value::String("##description".to_string())),
        "declaration text is ordinary data with no convention declared"
    );
    assert!(
        table.rows[1]
            .fields
            .values()
            .any(|v| v.value == Value::String("common,rare,epic".to_string())),
        "the enum row is data too"
    );
}

/// Preset B: every row above the header is a declaration row.
#[test]
fn test_marker_rows_preset_decodes_types_descriptions_and_enums() {
    let wb = skill_workbook();
    let adapter = ExcelSourceAdapter {
        header_row: 3,
        convention: Some(ExcelConvention {
            preset: Preset::MarkerRows,
            ..Default::default()
        }),
        ..Default::default()
    };

    let (doc, metas) = adapter
        .parse_file_with_meta(wb.path())
        .expect("convention decodes");
    let table = doc
        .tables
        .get("Data")
        .expect("the data table is still there");
    assert_eq!(table.rows.len(), 1, "only the row below the header is data");
    assert_eq!(
        table.rows[0].fields.get("name").unwrap().value,
        Value::String("sword".to_string())
    );

    let meta = metas.get("Data").expect("metadata for the table");
    assert_eq!(meta.header_row, 3);
    let id = meta.columns.get("id").expect("id declared");
    assert_eq!(id.cell, "A1");
    assert_eq!(id.field_type, Some(FieldType::Int32));
    assert_eq!(id.description, None);
    let name = meta.columns.get("name").expect("name declared");
    assert_eq!(name.cell, "B1");
    assert_eq!(name.field_type, Some(FieldType::String));
    assert_eq!(name.description.as_deref(), Some("武器名"));
    let rarity = meta.columns.get("rarity").expect("rarity declared");
    assert_eq!(rarity.cell, "C1");
    assert_eq!(
        rarity.field_type,
        Some(FieldType::Enum("Rarity".to_string()))
    );
    assert_eq!(
        rarity.enum_values.as_deref(),
        Some(["common", "rare", "epic"].map(String::from).as_slice())
    );
}

// ---- Luban sub-column fixtures (design §51 Q6, 回流-5) -----------------
//
// Five cases in the Luban row shape the sourcing directive points at:
// `##var` is the data header row, `##+` declares sub-columns, and the
// `##type` (and other keyword) rows below declare them. A parent column
// is merged across its sub-column span, which is what makes multi-level
// headers and nested beans visible to the data plane.

/// A workbook from Luban-style rows; the dimension derives from the widest
/// row and `merges` are `<mergeCell ref>` entries.
fn luban_workbook(rows: &[&[&str]], merges: &[&str], file_name: &str) -> TempWorkbook {
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(1);
    let last_col = ExcelSourceAdapter::col_index_to_letter(width - 1);
    let dimension = format!("A1:{last_col}{}", rows.len());
    let sheet = sheet_xml_with_merges(rows, 1, &dimension, merges);
    TempWorkbook::new(&minimal_xlsx(Some(&sheet)), file_name)
}

/// A marker-rows adapter for a Luban sheet whose `##var` row is row 0.
fn luban_adapter() -> ExcelSourceAdapter {
    ExcelSourceAdapter {
        header_row: 0,
        convention: Some(ExcelConvention {
            preset: Preset::MarkerRows,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Case 1 — a single nested bean (skill weapon): `##var` / `##+` /
/// `##type` rows assemble each data row's weapon column into an object.
#[test]
fn test_luban_nested_bean_assembles_sub_columns_into_object() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "weapon", ""][..],
            &["##+", "", "atk", "def"][..],
            &["##type", "Int32", "Int32", "Int32"][..],
            &["", "1", "10", "5"][..],
            &["", "2", "20", "8"][..],
        ],
        &["C1:D1"],
        "skill.xlsx",
    );
    let (doc, metas) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("nested bean decodes");
    let table = doc.tables.get("Data").expect("one data table");
    assert_eq!(table.rows.len(), 2, "declaration rows are not data");

    let weapon = &table.rows[0].fields["weapon"].value;
    assert_eq!(
        weapon,
        &Value::Object(IndexMap::from([
            ("atk".to_string(), Value::Int(10)),
            ("def".to_string(), Value::Int(5)),
        ])),
        "the weapon column is a nested bean"
    );
    assert_eq!(
        table.rows[1].fields["weapon"].value,
        Value::Object(IndexMap::from([
            ("atk".to_string(), Value::Int(20)),
            ("def".to_string(), Value::Int(8)),
        ]))
    );
    assert_eq!(
        table.rows[0].fields["weapon"].location.column.as_deref(),
        Some("C")
    );
    assert_eq!(table.rows[0].primary_key, vec![Value::Int(1)]);

    let meta = metas.get("Data").expect("table metadata");
    let weapon_meta = meta.columns.get("weapon").expect("weapon declared");
    assert_eq!(
        weapon_meta.sub_columns.keys().collect::<Vec<_>>(),
        ["atk", "def"],
        "sub-columns in declaration order"
    );
    assert_eq!(
        weapon_meta.sub_columns["atk"].field_type,
        Some(FieldType::Int32)
    );
    assert_eq!(
        weapon_meta.sub_columns["def"].field_type,
        Some(FieldType::Int32)
    );
}

/// Case 2 — a nested bean alongside flat scalar columns: the flat fields
/// are untouched and keep their cells; only the nested column changes.
#[test]
fn test_luban_nested_bean_with_flat_siblings() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "name", "weapon", ""][..],
            &["##+", "", "", "atk", "def"][..],
            &["##type", "Int32", "String", "Int32", "Int32"][..],
            &["", "1", "sword", "10", "5"][..],
            &["", "2", "shield", "3", "2"][..],
        ],
        &["D1:E1"],
        "weapon.xlsx",
    );
    let (doc, metas) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("mixed flat and nested decodes");
    let table = doc.tables.get("Data").expect("one data table");

    assert_eq!(table.rows[0].fields["id"].value, Value::Int(1));
    assert_eq!(
        table.rows[0].fields["name"].value,
        Value::String("sword".to_string())
    );
    assert_eq!(
        table.rows[0].fields["weapon"].value,
        Value::Object(IndexMap::from([
            ("atk".to_string(), Value::Int(10)),
            ("def".to_string(), Value::Int(5)),
        ]))
    );
    let fields: Vec<&str> = table.rows[0].fields.keys().map(String::as_str).collect();
    assert_eq!(fields, ["id", "name", "weapon"], "header order preserved");

    let meta = metas.get("Data").expect("table metadata");
    assert!(
        meta.columns["name"].sub_columns.is_empty(),
        "flat column has no sub-columns"
    );
    assert_eq!(meta.columns["name"].field_type, Some(FieldType::String));
    assert_eq!(
        meta.columns["weapon"].sub_columns["atk"].field_type,
        Some(FieldType::Int32)
    );
}

/// Case 3 — two nested beans in one table (dungeon stage and reward): both
/// groups assemble independently under their own merged parent headers.
#[test]
fn test_luban_two_nested_beans_in_one_table() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "stage", "", "reward", ""][..],
            &["##+", "", "wave", "waveId", "gold", "items"][..],
            &["##type", "Int32", "Int32", "Int32", "Int32", "String"][..],
            &["", "1", "1", "100", "50", "sword"][..],
        ],
        &["C1:D1", "E1:F1"],
        "dungeon.xlsx",
    );
    let (doc, _) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("two nested beans decode");
    let table = doc.tables.get("Data").expect("one data table");

    assert_eq!(
        table.rows[0].fields["stage"].value,
        Value::Object(IndexMap::from([
            ("wave".to_string(), Value::Int(1)),
            ("waveId".to_string(), Value::Int(100)),
        ]))
    );
    assert_eq!(
        table.rows[0].fields["reward"].value,
        Value::Object(IndexMap::from([
            ("gold".to_string(), Value::Int(50)),
            ("items".to_string(), Value::String("sword".to_string())),
        ]))
    );
    assert_eq!(
        table.rows[0].fields["reward"].location.column.as_deref(),
        Some("E")
    );
}

/// Case 4 — nesting depth (hero → stats → hp): a sub-column that itself
/// carries sub-columns is an object, read from the deeper `##+` row.
#[test]
fn test_luban_grandchild_nesting_is_an_object() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "hero", ""][..],
            &["##+", "", "base", "stats"][..],
            &["##+", "", "", "hp"][..],
            &["##type", "Int32", "Int32", "Int32"][..],
            &["", "1", "10", "5"][..],
        ],
        &["C1:D1"],
        "hero.xlsx",
    );
    let (doc, metas) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("grandchild nesting decodes");
    let table = doc.tables.get("Data").expect("one data table");
    assert_eq!(
        table.rows[0].fields["hero"].value,
        Value::Object(IndexMap::from([
            ("base".to_string(), Value::Int(10)),
            (
                "stats".to_string(),
                Value::Object(IndexMap::from([("hp".to_string(), Value::Int(5))])),
            ),
        ]))
    );
    let meta = metas.get("Data").expect("table metadata");
    let stats = &meta.columns["hero"].sub_columns["stats"];
    assert_eq!(
        stats.sub_columns.keys().collect::<Vec<_>>(),
        ["hp"],
        "the deeper `##+` row nests under the deepest sub-column at that column"
    );
}

/// Case 5 — an optional sub-column with a declared default: the empty cell
/// stays `Null` in the assembled object while the default lands in the
/// metadata (the data plane does not apply defaults; the schema layer does).
#[test]
fn test_luban_optional_sub_column_with_default_stays_null() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "equip", ""][..],
            &["##+", "", "slot", "level"][..],
            &["##type", "Int32", "String", "Int32"][..],
            &["##default", "", "", "1"][..],
            &["", "1", "weapon", ""][..],
        ],
        &["C1:D1"],
        "equip.xlsx",
    );
    let (doc, metas) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("optional sub-column decodes");
    let table = doc.tables.get("Data").expect("one data table");
    assert_eq!(
        table.rows[0].fields["equip"].value,
        Value::Object(IndexMap::from([
            ("slot".to_string(), Value::String("weapon".to_string())),
            ("level".to_string(), Value::Null),
        ]))
    );
    let meta = metas.get("Data").expect("table metadata");
    assert_eq!(
        meta.columns["equip"].sub_columns["level"].default,
        Some(serde_json::json!(1)),
        "the default is declared on the sub-column"
    );
}

// ---- Luban multi-row fixtures (design §51 Q6, 回流-5) ------------------
//
// A top-level `*name` in the `##var` row folds contiguous data rows into
// one record: the first row carries the root fields, continuation rows
// carry only the listed sub-columns, and each such row appends one list
// element to the named column.

/// Case 1 — `*waves` multi-row list (dungeon stage): root fields on the
/// first row, continuation rows append elements; the named column is a
/// `Value::Array` of the sub-column objects.
#[test]
fn test_luban_multi_row_folds_contiguous_rows_into_an_array() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "name", "*waves", "*waves", "*waves"][..],
            &["##+", "", "", "index", "groups", "rewards"][..],
            &["##type", "Int32", "String", "Int32", "String", "String"][..],
            &["", "1", "stage1", "1", "wolf,skeleton", "sword"][..],
            &["", "", "", "2", "goblin", "potion"][..],
            &["", "2", "stage2", "1", "dragon", "gem"][..],
        ],
        &["D1:F1"],
        "dungeon.xlsx",
    );
    let (doc, metas) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("multi-row records decode");
    let table = doc.tables.get("Data").expect("one data table");
    assert_eq!(table.rows.len(), 2, "three data rows fold into two records");

    let first = &table.rows[0];
    assert_eq!(first.fields["id"].value, Value::Int(1));
    assert_eq!(first.fields["name"].value, Value::String("stage1".into()));
    assert_eq!(
        first.fields["waves"].value,
        Value::Array(vec![
            Value::Object(IndexMap::from([
                ("index".to_string(), Value::Int(1)),
                ("groups".to_string(), Value::String("wolf,skeleton".into())),
                ("rewards".to_string(), Value::String("sword".into())),
            ])),
            Value::Object(IndexMap::from([
                ("index".to_string(), Value::Int(2)),
                ("groups".to_string(), Value::String("goblin".into())),
                ("rewards".to_string(), Value::String("potion".into())),
            ])),
        ]),
        "continuation rows append to the waves array"
    );
    assert_eq!(first.primary_key, vec![Value::Int(1)]);

    let second = &table.rows[1];
    assert_eq!(second.fields["id"].value, Value::Int(2));
    assert_eq!(second.fields["name"].value, Value::String("stage2".into()));
    assert_eq!(
        second.fields["waves"].value,
        Value::Array(vec![Value::Object(IndexMap::from([
            ("index".to_string(), Value::Int(1)),
            ("groups".to_string(), Value::String("dragon".into())),
            ("rewards".to_string(), Value::String("gem".into())),
        ]))]),
        "a fresh record starts when a root field is present"
    );

    let meta = metas.get("Data").expect("table metadata");
    assert!(
        meta.columns["waves"].multi_row,
        "the * header marks the column multi-row"
    );
    assert_eq!(
        meta.columns["waves"].sub_columns.keys().collect::<Vec<_>>(),
        ["index", "groups", "rewards"],
        "list element shape from the ##+ row"
    );
}

/// Case 2 — a multi-row list beside a nested bean and flat fields: the
/// bean folds into one array while the nested cost column and root
/// scalars stay per-record. The list column keeps declaration order.
#[test]
fn test_luban_multi_row_beside_nested_bean_and_flat_fields() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "name", "cost", "", "*effects", ""][..],
            &["##+", "", "", "mp", "cd", "trigger", "damage"][..],
            &[
                "##type", "Int32", "String", "Int32", "Int32", "String", "Int32",
            ][..],
            &["", "1", "skill1", "10", "5", "crit", "100"][..],
            &["", "", "", "", "", "heal", "60"][..],
            &["", "2", "skill2", "20", "8", "miss", "0"][..],
        ],
        &["D1:E1", "F1:G1"],
        "skill.xlsx",
    );
    let (doc, _) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("multi-row beside a bean decodes");
    let table = doc.tables.get("Data").expect("one data table");
    assert_eq!(table.rows.len(), 2);

    let first = &table.rows[0];
    assert_eq!(
        first.fields["cost"].value,
        Value::Object(IndexMap::from([
            ("mp".to_string(), Value::Int(10)),
            ("cd".to_string(), Value::Int(5)),
        ])),
        "the flat record's nested bean is untouched"
    );
    assert_eq!(
        first.fields["effects"].value,
        Value::Array(vec![
            Value::Object(IndexMap::from([
                ("trigger".to_string(), Value::String("crit".into())),
                ("damage".to_string(), Value::Int(100)),
            ])),
            Value::Object(IndexMap::from([
                ("trigger".to_string(), Value::String("heal".into())),
                ("damage".to_string(), Value::Int(60)),
            ])),
        ])
    );
    let fields: Vec<&str> = first.fields.keys().map(String::as_str).collect();
    assert_eq!(fields, ["id", "name", "cost", "effects"], "header order");
}

/// Case 3 — a single-row multi-row record produces a one-element array,
/// and the record's trailing row closes the pending array.
#[test]
fn test_luban_multi_row_single_element_and_flush_at_end() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "*waves", "*waves"][..],
            &["##+", "", "index", "note"][..],
            &["##type", "Int32", "Int32", "String"][..],
            &["", "7", "3", "a"][..],
        ],
        &["C1:D1"],
        "stage.xlsx",
    );
    let (doc, _) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("single-element multi-row decodes");
    let table = doc.tables.get("Data").expect("one data table");
    assert_eq!(table.rows.len(), 1);
    assert_eq!(
        table.rows[0].fields["waves"].value,
        Value::Array(vec![Value::Object(IndexMap::from([
            ("index".to_string(), Value::Int(3)),
            ("note".to_string(), Value::String("a".into())),
        ]))]),
        "the pending record flushes at end of data"
    );
    assert_eq!(table.rows[0].fields["id"].value, Value::Int(7));
}

/// A blank row between two multi-row records is a separator: it is
/// skipped (no bogus list element) and the records fold independently.
#[test]
fn test_luban_multi_row_blank_row_is_a_separator_not_an_element() {
    let wb = luban_workbook(
        &[
            &["##var", "id", "*waves", "*waves"][..],
            &["##+", "", "index", "note"][..],
            &["##type", "Int32", "Int32", "String"][..],
            &["", "1", "10", "a"][..],
            &["", "", "", ""][..],
            &["", "2", "20", "b"][..],
        ],
        &["C1:D1"],
        "stages.xlsx",
    );
    let (doc, _) = luban_adapter()
        .parse_file_with_meta(wb.path())
        .expect("blank separator decodes");
    let table = doc.tables.get("Data").expect("one data table");
    assert_eq!(table.rows.len(), 2, "the blank row is skipped");
    assert_eq!(
        table.rows[0].fields["waves"].value,
        Value::Array(vec![Value::Object(IndexMap::from([
            ("index".to_string(), Value::Int(10)),
            ("note".to_string(), Value::String("a".into())),
        ]))]),
        "no bogus element from the separator"
    );
    assert_eq!(table.rows[1].fields["id"].value, Value::Int(2));
    assert_eq!(
        table.rows[1].fields["waves"].value,
        Value::Array(vec![Value::Object(IndexMap::from([
            ("index".to_string(), Value::Int(20)),
            ("note".to_string(), Value::String("b".into())),
        ]))])
    );
}
/// Preset A: only the contiguous non-empty run immediately above the
/// header row is read — a blank separator row ends the block.
#[test]
fn test_header_block_preset_stops_at_the_blank_row() {
    let sheet = sheet_xml(
        &[
            &["##description", "序号说明", "名称说明"][..],
            &["", "", ""][..],
            &["##type Float64", "Float64", ""][..],
            &["id", "name", "note"][..],
            &["1", "x", ""][..],
        ],
        1,
        "A1:C5",
    );
    let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "item.xlsx");
    let adapter = ExcelSourceAdapter {
        header_row: 3,
        convention: Some(ExcelConvention {
            preset: Preset::HeaderBlock,
            ..Default::default()
        }),
        ..Default::default()
    };

    let (doc, metas) = adapter
        .parse_file_with_meta(wb.path())
        .expect("convention decodes");

    let meta = metas.get("Data").expect("metadata for the table");
    let id = meta.columns.get("id").expect("id declared");
    assert_eq!(
        id.field_type,
        Some(FieldType::Float64),
        "the block directly above the header"
    );
    assert_eq!(
        id.description, None,
        "the description row past the blank row is not in the block"
    );
    let table = doc.tables.get("Data").expect("data table");
    assert_eq!(table.rows.len(), 1);
}

/// Preset C: the sibling `<sheet>.meta` sheet carries the declarations and
/// is never parsed as a table.
#[test]
fn test_meta_sheet_preset_reads_the_sibling_and_skips_it() {
    let data = sheet_xml(
        &[&["id", "name", "rarity"][..], &["1", "sword", "common"][..]],
        1,
        "A1:C2",
    );
    let meta_sheet = sheet_xml(
        &[
            &["id", "name", "rarity"][..],
            &["##type Int32", "String", "Enum<Rarity>"][..],
            &["##description", "武器名", "稀有度"][..],
            &["##enum", "", "common,rare,epic"][..],
        ],
        1,
        "A1:C4",
    );
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Skill" sheetId="1" r:id="rId1"/><sheet name="Skill.meta" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    let wb = TempWorkbook::new(
        &minimal_xlsx_with_workbook(
            workbook,
            &[("sheet1.xml", &data), ("sheet2.xml", &meta_sheet)],
        ),
        "meta.xlsx",
    );
    let adapter = ExcelSourceAdapter {
        convention: Some(ExcelConvention {
            preset: Preset::MetaSheet,
            ..Default::default()
        }),
        ..Default::default()
    };

    let (doc, metas) = adapter
        .parse_file_with_meta(wb.path())
        .expect("convention decodes");

    assert!(doc.tables.contains_key("Skill"));
    assert!(
        !doc.tables.contains_key("Skill.meta"),
        "the meta sheet is a carrier, not a table"
    );
    let meta = metas.get("Skill").expect("metadata for the table");
    let id = meta.columns.get("id").expect("id declared");
    assert_eq!(id.field_type, Some(FieldType::Int32));
    let name = meta.columns.get("name").expect("name declared");
    assert_eq!(name.description.as_deref(), Some("武器名"));
    let rarity = meta.columns.get("rarity").expect("rarity declared");
    assert_eq!(
        rarity.enum_values.as_deref(),
        Some(["common", "rare", "epic"].map(String::from).as_slice())
    );
}

/// The meta sheet's name row must agree with the data header row.
#[test]
fn test_meta_sheet_name_row_mismatch_is_e2207() {
    let data = sheet_xml(&[&["id", "name"][..], &["1", "x"][..]], 1, "A1:B2");
    let meta_sheet = sheet_xml(
        &[
            &["id", "name", "ghost"][..],
            &["##type Int32", "String", "Float64"][..],
        ],
        1,
        "A1:C2",
    );
    let workbook = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Skill" sheetId="1" r:id="rId1"/><sheet name="Skill.meta" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    let wb = TempWorkbook::new(
        &minimal_xlsx_with_workbook(
            workbook,
            &[("sheet1.xml", &data), ("sheet2.xml", &meta_sheet)],
        ),
        "mismatch.xlsx",
    );
    let adapter = ExcelSourceAdapter {
        convention: Some(ExcelConvention {
            preset: Preset::MetaSheet,
            ..Default::default()
        }),
        ..Default::default()
    };

    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert!(diags.errors().iter().any(|d| d.code == E2207));
    assert!(
        diags.errors().iter().any(
            |d| d.message.contains("ghost") && d.message.contains("not in the data header row")
        ),
        "{:?}",
        diags
            .errors()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );
}

/// A declaration cell is reported where the author put it: cell anchor in
/// the message and in the location.
#[test]
fn test_convention_violation_is_anchored_to_the_cell() {
    let sheet = sheet_xml(
        &[
            &["##type Int32", "String"][..],
            &["name", "##description", "x"][..],
            &["id", "name"][..],
        ],
        1,
        "A1:C3",
    );
    let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "broken.xlsx");
    let adapter = ExcelSourceAdapter {
        header_row: 2,
        convention: Some(ExcelConvention {
            preset: Preset::MarkerRows,
            ..Default::default()
        }),
        ..Default::default()
    };

    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert!(diags.errors().iter().all(|d| d.code == E2201));
    let errors = diags.errors();
    let hit = errors
        .iter()
        .find(|d| d.message.contains("A2"))
        .expect("anchored error");
    assert!(
        hit.message.contains("before any keyword"),
        "{:?}",
        hit.message
    );
    let location = hit.location.as_ref().expect("cell location");
    assert_eq!(location.sheet.as_deref(), Some("Data"));
    assert_eq!(location.column.as_deref(), Some("A"));
    assert_eq!(location.row, Some(2));
    assert!(hit.source.contains("broken.xlsx"));
}

// ---- Script adapter (X1, §51 Q2) ---------------------------------------
//
// The subprocess contract, exercised with a real script. The script is a
// repository file with a shebang, so nothing looks anything up on $PATH.
// Windows cannot exec a shebang script; the same tests stand behind
// `cfg(unix)` and a Windows project invokes a `.cmd`/`.bat` launcher,
// which is a repository file too (contract §"script").

/// Where python is; the script fixture uses `#!/usr/bin/env python3`.
#[cfg(unix)]
fn python() -> Option<String> {
    for candidate in ["python3", "python"] {
        if std::process::Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Some(candidate.to_string());
        }
    }
    None
}

/// A throwaway project root: `tools/` for scripts, a workbook inside it,
/// so a declared script path stays inside the root (§51 Q2).
#[cfg(unix)]
struct TempRoot(PathBuf);

#[cfg(unix)]
impl TempRoot {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "cage_convention_{}_{}_{}",
            std::process::id(),
            nanos,
            tag
        ));
        std::fs::create_dir_all(path.join("tools")).expect("tools dir");
        Self(path)
    }

    fn write_script(&self, rel: &str, body: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::write(&path, body).expect("script write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut perms = std::fs::metadata(&path)
                .expect("script metadata")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).expect("script chmod");
        }
        path
    }

    /// One data sheet: a header row and one row.
    fn workbook(&self, name: &str) -> PathBuf {
        let sheet = sheet_xml(&[&["id", "name"][..], &["1", "x"][..]], 1, "A1:B2");
        let path = self.0.join(name);
        std::fs::write(&path, minimal_xlsx(Some(&sheet))).expect("workbook write");
        path
    }

    fn adapter(&self, script: &str, timeout: u64) -> ExcelSourceAdapter {
        ExcelSourceAdapter {
            header_row: 0,
            convention: Some(ExcelConvention {
                script: Some(script.to_string()),
                script_timeout_secs: timeout,
                ..Default::default()
            }),
            project_root: Some(self.0.clone()),
            ..Default::default()
        }
    }
}

#[cfg(unix)]
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The script the contract tests point at: it reads the request, checks
/// the contract version, and answers two columns.
#[cfg(unix)]
const META_SCRIPT: &str = r#"#!/usr/bin/env python3
import json, sys
request = json.load(sys.stdin)
assert request["contract"] == "cage.excel-meta/1", request["contract"]
response = {
    "contract": "cage.excel-meta/1",
    "table": request["sheet"],
    "header_row": request["header_row"],
    "columns": {
        "id": {"cell": "A1", "type": {"kind": "Int32"}, "required": True},
        "name": {"cell": "B1", "type": {"kind": "String"}, "description": "武器名"},
    },
}
print(json.dumps(response, ensure_ascii=False))
"#;

#[cfg(unix)]
#[test]
fn test_script_contract_delivers_metadata() {
    if python().is_none() {
        eprintln!("skip: no python interpreter");
        return;
    }

    let root = TempRoot::new("ok");
    let path = root.write_script("tools/meta.py", META_SCRIPT);
    let wb = root.workbook("Data.xlsx");

    let (doc, metas) = root
        .adapter("tools/meta.py", 10)
        .parse_file_with_meta(&wb)
        .expect("the contract round-trips");

    assert!(doc.tables.contains_key("Data"));
    let meta = metas.get("Data").expect("metadata for the table");
    let id = meta.columns.get("id").expect("id declared");
    assert_eq!(id.cell, "A1");
    assert_eq!(id.field_type, Some(FieldType::Int32));
    assert_eq!(id.required, Some(true));
    let name = meta.columns.get("name").expect("name declared");
    assert_eq!(name.field_type, Some(FieldType::String));
    assert_eq!(name.description.as_deref(), Some("武器名"));
    assert!(
        path.exists(),
        "the fixture still stands until it is dropped"
    );
}

#[cfg(unix)]
#[test]
fn test_script_will_not_start_is_e2203() {
    for missing in [
        "tools/absent.py",
        "/absent/meta.py",
        "tools/../tools/meta.py",
    ] {
        let root = TempRoot::new(&format!("missing_{}", root_tag(missing)));
        let wb = root.workbook("Data.xlsx");
        let diags = root.adapter(missing, 10).parse_file(&wb).unwrap_err();
        assert!(diags.errors().iter().any(|d| d.code == E2203), "{missing}");
    }
}

/// A built-in preset cannot work without a header row to key columns by:
/// workbook-level E2201, before any cell is read.
#[test]
fn test_preset_without_a_header_row_is_e2201() {
    let wb = TempWorkbook::new(&minimal_xlsx(None), "headerless.xlsx");
    let adapter = ExcelSourceAdapter {
        has_header: false,
        convention: Some(ExcelConvention {
            preset: Preset::HeaderBlock,
            ..Default::default()
        }),
        ..Default::default()
    };
    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert_eq!(diags.errors().len(), 1, "{:?}", diags.errors());
    let d = &diags.errors()[0];
    assert_eq!(d.code, E2201);
    assert!(d.message.contains("has_header = false"), "{}", d.message);
}

/// An incoherent convention declaration (a marker that is not a `#`
/// word) is refused once per workbook, before any sheet is opened.
#[test]
fn test_incoherent_convention_is_refused_before_any_sheet_is_read() {
    let wb = skill_workbook();
    let adapter = ExcelSourceAdapter {
        convention: Some(ExcelConvention {
            preset: Preset::MarkerRows,
            marker: "x".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert_eq!(diags.errors().len(), 1, "{:?}", diags.errors());
    let d = &diags.errors()[0];
    assert_eq!(d.code, E2201);
    assert!(
        d.message.contains("marker must start with '#'"),
        "{}",
        d.message
    );
}

/// The meta-sheet preset needs the sibling `<sheet>.meta` part; a
/// workbook that names the convention but ships only the data sheet is
/// a workbook-level E2201 naming the missing sibling.
#[test]
fn test_meta_sheet_preset_without_the_sibling_is_e2201() {
    let sheet = sheet_xml(&[&["id", "name"][..], &["1", "sword"][..]], 1, "A1:B2");
    let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "lonely.xlsx");
    let adapter = ExcelSourceAdapter {
        convention: Some(ExcelConvention {
            preset: Preset::MetaSheet,
            ..Default::default()
        }),
        ..Default::default()
    };
    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert_eq!(diags.errors().len(), 1, "{:?}", diags.errors());
    let d = &diags.errors()[0];
    assert_eq!(d.code, E2201);
    assert!(
        d.message.contains("Data.meta") && d.message.contains("could not be read"),
        "{}",
        d.message
    );
}

/// A script adapter without the project root cannot resolve the script
/// path, so the sheet is refused with E2203 instead of silently
/// falling back to a built-in preset.
#[test]
fn test_script_convention_without_project_root_is_e2203() {
    let wb = skill_workbook();
    let adapter = ExcelSourceAdapter {
        convention: Some(ExcelConvention {
            script: Some("tools/contract.py".to_string()),
            ..Default::default()
        }),
        project_root: None,
        ..Default::default()
    };
    let diags = adapter.parse_file(wb.path()).unwrap_err();
    assert_eq!(diags.errors().len(), 1, "{:?}", diags.errors());
    let d = &diags.errors()[0];
    assert_eq!(d.code, E2203);
    assert!(d.message.contains("project root"), "{}", d.message);
}

/// The contract cell mapping keeps raw values without type inference:
/// non-finite floats and sheet errors render as their text form, dates
/// keep the data plane's rendering.
#[test]
fn test_contract_value_maps_exotic_cells_without_panic() {
    use calamine::{ExcelDateTime, ExcelDateTimeType};
    assert_eq!(
        ExcelSourceAdapter::cell_to_contract_value(&Data::Empty),
        CellValue::Null
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_contract_value(&Data::Bool(true)),
        CellValue::Bool(true)
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_contract_value(&Data::Int(-7)),
        CellValue::Int(-7)
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_contract_value(&Data::Float(2.5)),
        CellValue::Float(2.5)
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_contract_value(&Data::Float(f64::NAN)),
        CellValue::Str("NaN".to_string())
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_contract_value(&Data::Float(f64::INFINITY)),
        CellValue::Str("inf".to_string())
    );
    assert_eq!(
        ExcelSourceAdapter::cell_to_contract_value(&Data::String("sword".into())),
        CellValue::Str("sword".into())
    );
    let dt = ExcelDateTime::new(44927.0, ExcelDateTimeType::DateTime, true);
    assert_eq!(
        ExcelSourceAdapter::cell_to_contract_value(&Data::DateTime(dt)),
        CellValue::Str(format!("{dt:?}"))
    );
}

/// A data row wider than the `##var` declaration row: the stray cell
/// lands in a generated `col<n>` named for its absolute column, exactly
/// like a blank header — it is neither dropped nor a crash.
#[test]
fn test_data_row_wider_than_the_declaration_gets_a_col_fallback() {
    let sheet = sheet_xml(
        &[
            &["##var", "id", "name"][..],
            &["##type", "Int32", "String"][..],
            &["", "1", "sword", "stray"][..],
        ],
        1,
        "A1:D3",
    );
    let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "wide.xlsx");
    let adapter = ExcelSourceAdapter {
        convention: Some(ExcelConvention {
            preset: Preset::MarkerRows,
            ..Default::default()
        }),
        ..Default::default()
    };
    let (doc, _) = adapter
        .parse_file_with_meta(wb.path())
        .expect("extra trailing cells are tolerated");
    let table = doc.tables.get("Data").expect("table decoded");
    assert_eq!(table.rows.len(), 1);
    let row = &table.rows[0];
    assert_eq!(
        row.fields.keys().map(String::as_str).collect::<Vec<_>>(),
        ["id", "name", "col3"],
        "the stray cell keeps a generated absolute-column name"
    );
    assert_eq!(row.fields["id"].value, Value::Int(1));
    assert_eq!(row.fields["name"].value, Value::String("sword".into()));
    assert_eq!(row.fields["col3"].value, Value::String("stray".into()));
}

/// A declared header row beyond the sheet's height means every visible
/// row is a declaration row without a keyword — a per-cell E2201, not a
/// silent empty table.
#[test]
fn test_header_row_beyond_the_sheet_reports_keywordless_rows() {
    let sheet = sheet_xml(&[&["id", "name"][..], &["1", "sword"][..]], 1, "A1:B2");
    let wb = TempWorkbook::new(&minimal_xlsx(Some(&sheet)), "short.xlsx");
    let adapter = ExcelSourceAdapter {
        header_row: 10,
        convention: Some(ExcelConvention {
            preset: Preset::MarkerRows,
            ..Default::default()
        }),
        ..Default::default()
    };
    let diags = adapter.parse_file(wb.path()).unwrap_err();
    for d in diags.errors() {
        assert_eq!(d.code, E2201, "{}", d.message);
        assert!(
            d.message.contains("appears before any keyword cell"),
            "{}",
            d.message
        );
    }
    assert_eq!(diags.errors().len(), 4, "one per non-empty cell");
}

/// A declared script name that is not a file inside the root is refused.
#[cfg(unix)]
fn root_tag(missing: &str) -> String {
    missing
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

#[cfg(unix)]
#[test]
fn test_script_timeout_is_e2204() {
    if python().is_none() {
        eprintln!("skip: no python interpreter");
        return;
    }

    let root = TempRoot::new("timeout");
    root.write_script(
        "tools/slow.py",
        "#!/usr/bin/env python3\nimport time\ntime.sleep(30)\nprint('{}')\n",
    );
    let wb = root.workbook("Data.xlsx");

    let diags = root
        .adapter("tools/slow.py", 1)
        .parse_file(&wb)
        .unwrap_err();
    assert!(diags.errors().iter().any(|d| d.code == E2204), "{diags:?}");
    assert!(
        diags
            .errors()
            .iter()
            .any(|d| d.message.contains("timed out")),
        "{:?}",
        diags
            .errors()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );
}

#[cfg(unix)]
#[test]
fn test_script_failure_is_e2205_with_its_stderr() {
    if python().is_none() {
        eprintln!("skip: no python interpreter");
        return;
    }

    let root = TempRoot::new("crash");
    root.write_script(
            "tools/crash.py",
            "#!/usr/bin/env python3\nimport sys\nprint('authoring went wrong', file=sys.stderr)\nsys.exit(3)\n",
        );
    let wb = root.workbook("Data.xlsx");

    let diags = root
        .adapter("tools/crash.py", 10)
        .parse_file(&wb)
        .unwrap_err();
    assert!(diags.errors().iter().any(|d| d.code == E2205), "{diags:?}");
    assert!(
        diags
            .errors()
            .iter()
            .any(|d| d.message.contains("authoring went wrong")),
        "{:?}",
        diags
            .errors()
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );
}

/// Regression: >400 bytes of multi-byte stderr (the byte at offset 400
/// falls inside a CJK character) must not panic in the E2205 truncation
/// — the diagnostic path truncates at a char boundary.
#[cfg(unix)]
#[test]
fn test_script_failure_truncates_cjk_stderr_at_char_boundary() {
    if python().is_none() {
        eprintln!("skip: no python interpreter");
        return;
    }

    let root = TempRoot::new("cjk-stderr");
    root.write_script(
        "tools/cjk.py",
        "#!/usr/bin/env python3\nimport sys\nsys.stderr.write('爆' * 300)\nsys.exit(1)\n",
    );
    let wb = root.workbook("Data.xlsx");

    let diags = root
        .adapter("tools/cjk.py", 10)
        .parse_file(&wb)
        .unwrap_err();
    assert!(diags.errors().iter().any(|d| d.code == E2205), "{diags:?}");
    for d in diags.errors() {
        if d.code == E2205 {
            // A panic would have crashed the test. The stderr portion is
            // truncated to <=400 bytes at a char boundary, still valid
            // UTF-8, and carries the CJK payload.
            // The message is `script <path> exited with <status>: <stderr>`
            // with the stderr at the very end. Count the trailing CJK run
            // (each '爆' is 3 bytes) and assert the segment is <=400 bytes
            // — a precise cap assertion, unlike a loose whole-message bound
            // that would not catch a cap relaxed to 500 bytes.
            let run = d.message.chars().rev().take_while(|c| *c == '爆').count();
            assert!(run > 0, "lost the stderr payload: {:?}", d.message);
            let stderr_bytes = run * '爆'.len_utf8();
            assert!(
                stderr_bytes <= 400,
                "stderr segment {stderr_bytes}B exceeds the 400B cap: {:?}",
                d.message
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn test_script_bad_output_is_e2206() {
    if python().is_none() {
        eprintln!("skip: no python interpreter");
        return;
    }

    for (tag, code, script) in [
            (
                "garbage",
                E2206,
                "#!/usr/bin/env python3\nprint('not json at all')\n",
            ),
            (
                "contract",
                E2206,
                "#!/usr/bin/env python3\nimport json, sys\njson.load(sys.stdin)\nprint('{\"contract\": \"cage.excel-meta/9\", \"table\": \"Data\", \"header_row\": 0, \"columns\": {}}')\n",
            ),
            (
                "table",
                E2207,
                "#!/usr/bin/env python3\nimport json, sys\njson.load(sys.stdin)\nprint('{\"contract\": \"cage.excel-meta/1\", \"table\": \"Other\", \"header_row\": 0, \"columns\": {\"id\": {\"cell\": \"A1\", \"type\": {\"kind\": \"Int32\"}}}}')\n",
            ),
        ] {
            let root = TempRoot::new(tag);
            root.write_script("tools/case.py", script);
            let wb = root.workbook("Data.xlsx");

            let diags = root.adapter("tools/case.py", 10).parse_file(&wb).unwrap_err();
            assert!(
                diags.errors().iter().any(|d| d.code == code),
                "{tag}: {diags:?}"
            );
        }
}

/// The response is gated by the Schema vocabulary, not by a whitelist.
#[cfg(unix)]
#[test]
fn test_script_invented_keyword_is_e2202() {
    if python().is_none() {
        eprintln!("skip: no python interpreter");
        return;
    }

    let root = TempRoot::new("vocabulary");
    root.write_script(
            "tools/vocab.py",
            "#!/usr/bin/env python3\nimport json, sys\njson.load(sys.stdin)\nprint('{\"contract\": \"cage.excel-meta/1\", \"table\": \"Data\", \"header_row\": 0, \"columns\": {\"id\": {\"cell\": \"A1\", \"type\": {\"kind\": \"Int32\"}, \"note\": \"invented\"}}}')\n",
        );
    let wb = root.workbook("Data.xlsx");

    let diags = root
        .adapter("tools/vocab.py", 10)
        .parse_file(&wb)
        .unwrap_err();
    assert!(diags.errors().iter().any(|d| d.code == E2202), "{diags:?}");
}
