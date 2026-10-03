#!/usr/bin/env python3
"""Generate merged_cells.xlsx — T2.4 合并单元格夹具（纯标准库，可复现）。

布局刻意打乱两处，逼出真实寻址/填充逻辑：
- 有数据区从 B2 起（used range 不落在 A1）：行列定位必须按 sheet 网格报，
  而不是按 range 内偏移；
- D3:D4 纵向合并（D4 在 XML 里无值，与 Excel 存盘一致）：D4 所在行
  必须取锚点 D3 的值。

列：B=id C=name D=type E=price F=note（F 列仅在 F5 有值，保证 F 在 used range 内）。
"""
import zipfile
from pathlib import Path

OUT = Path(__file__).parent / "merged_cells.xlsx"

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"""

WORKBOOK = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Items" sheetId="1" r:id="rId1"/></sheets>
</workbook>"""

WORKBOOK_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"""

SHEET = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<dimension ref="B2:F5"/>
<sheetData>
<row r="2"><c r="B2" t="inlineStr"><is><t>id</t></is></c><c r="C2" t="inlineStr"><is><t>name</t></is></c><c r="D2" t="inlineStr"><is><t>type</t></is></c><c r="E2" t="inlineStr"><is><t>price</t></is></c><c r="F2" t="inlineStr"><is><t>note</t></is></c></row>
<row r="3"><c r="B3"><v>1001</v></c><c r="C3" t="inlineStr"><is><t>Sword</t></is></c><c r="D3" t="inlineStr"><is><t>weapon</t></is></c><c r="E3"><v>99</v></c></row>
<row r="4"><c r="B4"><v>1002</v></c><c r="C4" t="inlineStr"><is><t>Shield</t></is></c><c r="E4"><v>250</v></c></row>
<row r="5"><c r="B5"><v>1003</v></c><c r="C5" t="inlineStr"><is><t>Potion</t></is></c><c r="D5" t="inlineStr"><is><t>material</t></is></c><c r="E5"><v>30</v></c><c r="F5" t="inlineStr"><is><t>rare</t></is></c></row>
</sheetData>
<mergeCells count="1"><mergeCell ref="D3:D4"/></mergeCells>
</worksheet>"""


def main() -> None:
    # 固定时间戳：同一脚本产出字节级一致的夹具
    with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED) as zf:
        for name, content in (
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/worksheets/sheet1.xml", SHEET),
        ):
            info = zipfile.ZipInfo(name, date_time=(2026, 10, 3, 0, 0, 0))
            zf.writestr(info, content, compress_type=zipfile.ZIP_DEFLATED)
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
