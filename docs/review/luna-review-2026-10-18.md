[0m
[0m✱ [0mGlob "**/*" [90m100 matches[0m
[0m$ [0mgit status --short && git ls-files | wc -l && find . -maxdepth 2 -type f | sort | sed -n '1,240p'
235
./Cargo.lock
./Cargo.toml
./codecov.yml
./docs/architecture.en.md
./docs/architecture.md
./docs/assets.en.md
./docs/assets.md
./docs/audit-doc-consistency.md
./docs/build.en.md
./docs/build.md
./docs/cli.en.md
./docs/cli.md
./docs/design.en.md
./docs/design.md
./docs/design-prototypes.en.md
./docs/design-prototypes.md
./docs/example.en.md
./docs/example.md
./docs/index.en.md
./docs/index.md
./docs/lifecycle.en.md
./docs/lifecycle.md
./docs/requirements.md
./docs/schema.en.md
./docs/schema.md
./docs/source.en.md
./docs/source.md
./docs/target.en.md
./docs/target.md
./docs/ui.en.md
./docs/ui.md
./docs/validation.en.md
./docs/validation.md
./docs/web.en.md
./docs/web.md
./docs/审计-文档一致性.md
./docs/需求整理.md
./examples/asset-workspace.sh
./examples/run.sh
./examples/web-smoke.sh
./.git/COMMIT_EDITMSG
./.git/config
./.git/description
./.git/FETCH_HEAD
./.git/HEAD
./.gitignore
./.git/index
./.git/ORIG_HEAD
./.git/packed-refs
./LICENSE
./node_modules/.package-lock.json
./package.json
./package-lock.json
./README.md
./README.zh.md
./scripts/install.ps1
./scripts/install.sh
./target/CACHEDIR.TAG
./target/.rustc_info.json
./todo.md
我会按仓库现状独立核查，不改文件、不提交。先梳理模块与公开承诺，再对核心实现、边界处理和测试逐项取证；结论只列有代码/文档行号支撑的项目，不把 todo 状态当作质量证据。
[0m$ [0mfind . -path './.git' -prune -o -path './node_modules' -prune -o -path './editors/vscode/node_modules' -prune -o -path './target' -prune -o -type f -print | sort
/dist/assets/chunks/gitGraph-VSP46ZUC.CWzEkEqn.js
./docs/.vitepress/dist/assets/chunks/graphlib.DS17s2tU.js
./docs/.vitepress/dist/assets/chunks/infoDiagram-5W2HQ5XZ.C3X_50l1.js
./docs/.vitepress/dist/assets/chunks/info-OHQRW6UA.BOGnc3Ok.js
./docs/.vitepress/dist/assets/chunks/init.C-OQMol4.js
./docs/.vitepress/dist/assets/chunks/ishikawaDiagram-K3B6WC7H.5YbInJlC.js
./docs/.vitepress/dist/assets/chunks/journeyDiagram-COXZFDF6.C2GqEdWf.js
./docs/.vitepress/dist/assets/chunks/kanban-definition-P3RFRI5V.BxsLF0Gr.js
./docs/.vitepress/dist/assets/chunks/katex.7vI-Mmbq.js
./docs/.vitepress/dist/assets/chunks/linear.CQTqr1oD.js
./docs/.vitepress/dist/assets/chunks/line.B-eYHO6F.js
./docs/.vitepress/dist/assets/chunks/mermaid.core.-bYljk4o.js
./docs/.vitepress/dist/assets/chunks/mermaid-parser.core.Dyeu1UsY.js
./docs/.vitepress/dist/assets/chunks/metadata.abbfebc9.js
./docs/.vitepress/dist/assets/chunks/mindmap-definition-LZFPQGKD.v6gQwYQI.js
./docs/.vitepress/dist/assets/chunks/ordinal.BAxfPAgN.js
./docs/.vitepress/dist/assets/chunks/packet-JDAUHWVQ.Caal_rUP.js
./docs/.vitepress/dist/assets/chunks/path.fybaL0A-.js
./docs/.vitepress/dist/assets/chunks/pegDiagram-BGZESJAR.D7zvlaPy.js
./docs/.vitepress/dist/assets/chunks/pieDiagram-CAPJLFHJ.GZEbHN6l.js
./docs/.vitepress/dist/assets/chunks/pie-XZMESJXO.CKgXUXUg.js
./docs/.vitepress/dist/assets/chunks/quadrantDiagram-GDMTTRAM.BwmofZeg.js
./docs/.vitepress/dist/assets/chunks/radar-ABXABTNO.CW-mdB_D.js
./docs/.vitepress/dist/assets/chunks/railroad-abnf-LNEFI6M7.B8GUKPfK.js
./docs/.vitepress/dist/assets/chunks/railroadDiagram-SM67HX2B.UPuv12Va.js
./docs/.vitepress/dist/assets/chunks/railroad-ebnf-DWYD2UWJ.CK7lChSQ.js
./docs/.vitepress/dist/assets/chunks/railroad-I3PHUGI6.C2-UPPs9.js
./docs/.vitepress/dist/assets/chunks/railroad-peg-JR7BVNR7.Djj9enTF.js
./docs/.vitepress/dist/assets/chunks/requirementDiagram-X7JNWC4A.DSYoNJn2.js
./docs/.vitepress/dist/assets/chunks/rough.esm.Dy-Kn_BL.js
./docs/.vitepress/dist/assets/chunks/sankeyDiagram-UM26HJRW.B0UQX1qz.js
./docs/.vitepress/dist/assets/chunks/sequenceDiagram-ZO4K6R2Y.RlWuYNKO.js
./docs/.vitepress/dist/assets/chunks/sizeCapture-INFHLROL.C-RahmM2.js
./docs/.vitepress/dist/assets/chunks/src.D83aMQU4.js
./docs/.vitepress/dist/assets/chunks/stateDiagram-v2-TBUQTH76.Cwi7xCvO.js
./docs/.vitepress/dist/assets/chunks/swimlanesDiagram-K3J5GTZL.BOSkl-hT.js
./docs/.vitepress/dist/assets/chunks/swimlanes-N4OXWK64.1-bhYtiS.js
./docs/.vitepress/dist/assets/chunks/theme.DeQTwRxZ.js
./docs/.vitepress/dist/assets/chunks/timeline-definition-YOQAKGHF.C-SU_wX6.js
./docs/.vitepress/dist/assets/chunks/treemap-SAJKECNS.BGFFpMDg.js
./docs/.vitepress/dist/assets/chunks/treeView-D4JQ5SDB.B-eiEiv0.js
./docs/.vitepress/dist/assets/chunks/usecaseDiagram-VIAY4XPW.D8ROhpEF.js
./docs/.vitepress/dist/assets/chunks/vennDiagram-BLWOH2XV.BkRhYLub.js
./docs/.vitepress/dist/assets/chunks/wardley-7MLQ67FV.9deTuSTt.js
./docs/.vitepress/dist/assets/chunks/wardleyDiagram-YMQ3BMBF.Ca9_Vacb.js
./docs/.vitepress/dist/assets/chunks/xychartDiagram-TAQBALBS.CaX9Gzso.js
./docs/.vitepress/dist/assets/cli.md.BCo2JxDg.js
./docs/.vitepress/dist/assets/cli.md.BCo2JxDg.lean.js
./docs/.vitepress/dist/assets/design-prototypes.md.Pi1_3E91.js
./docs/.vitepress/dist/assets/design-prototypes.md.Pi1_3E91.lean.js
./docs/.vitepress/dist/assets/en_architecture.md.BLD3TjCH.js
./docs/.vitepress/dist/assets/en_architecture.md.BLD3TjCH.lean.js
./docs/.vitepress/dist/assets/en_assets.md.DIs5bzXF.js
./docs/.vitepress/dist/assets/en_assets.md.DIs5bzXF.lean.js
./docs/.vitepress/dist/assets/en_build.md.D5Cn484F.js
./docs/.vitepress/dist/assets/en_build.md.D5Cn484F.lean.js
./docs/.vitepress/dist/assets/en_cli.md.7HOnX8Pr.js
./docs/.vitepress/dist/assets/en_cli.md.7HOnX8Pr.lean.js
./docs/.vitepress/dist/assets/en_design-prototypes.md.CzwDpqrL.js
./docs/.vitepress/dist/assets/en_design-prototypes.md.CzwDpqrL.lean.js
./docs/.vitepress/dist/assets/en_example.md.Dk8J7E0N.js
./docs/.vitepress/dist/assets/en_example.md.Dk8J7E0N.lean.js
./docs/.vitepress/dist/assets/en_index.md.SiZ7KK-k.js
./docs/.vitepress/dist/assets/en_index.md.SiZ7KK-k.lean.js
./docs/.vitepress/dist/assets/en_lifecycle.md.98Ywgwrh.js
./docs/.vitepress/dist/assets/en_lifecycle.md.98Ywgwrh.lean.js
./docs/.vitepress/dist/assets/en_schema.md.NfjPcSfK.js
./docs/.vitepress/dist/assets/en_schema.md.NfjPcSfK.lean.js
./docs/.vitepress/dist/assets/en_source.md.auLQZoO5.js
./docs/.vitepress/dist/assets/en_source.md.auLQZoO5.lean.js
./docs/.vitepress/dist/assets/en_target.md.C6d295fh.js
./docs/.vitepress/dist/assets/en_target.md.C6d295fh.lean.js
./docs/.vitepress/dist/assets/en_ui.md.Dn3RWivr.js
./docs/.vitepress/dist/assets/en_ui.md.Dn3RWivr.lean.js
./docs/.vitepress/dist/assets/en_validation.md.BH7b9_KP.js
./docs/.vitepress/dist/assets/en_validation.md.BH7b9_KP.lean.js
./docs/.vitepress/dist/assets/en_web.md.C0klJwtY.js
./docs/.vitepress/dist/assets/en_web.md.C0klJwtY.lean.js
./docs/.vitepress/dist/assets/example.md.EraeKICs.js
./docs/.vitepress/dist/assets/example.md.EraeKICs.lean.js
./docs/.vitepress/dist/assets.html
./docs/.vitepress/dist/assets/index.md.CXPqgKDf.js
./docs/.vitepress/dist/assets/index.md.CXPqgKDf.lean.js
./docs/.vitepress/dist/assets/init-wizard.Dp-h0iFL.png
./docs/.vitepress/dist/assets/inter-italic-cyrillic.CR0FzlVp.woff2
./docs/.vitepress/dist/assets/inter-italic-cyrillic-ext._yYvJxaO.woff2
./docs/.vitepress/dist/assets/inter-italic-greek.BeK1HKg-.woff2
./docs/.vitepress/dist/assets/inter-italic-greek-ext.DngHnpzC.woff2
./docs/.vitepress/dist/assets/inter-italic-latin.Duvr4T3O.woff2
./docs/.vitepress/dist/assets/inter-italic-latin-ext.Dm0n7MN1.woff2
./docs/.vitepress/dist/assets/inter-italic-symbols.DUl242AY.woff2
./docs/.vitepress/dist/assets/inter-italic-vietnamese.CNdtrOX_.woff2
./docs/.vitepress/dist/assets/inter-roman-cyrillic-ext.BIKsGeDD.woff2
./docs/.vitepress/dist/assets/inter-roman-cyrillic.R72SAhgh.woff2
./docs/.vitepress/dist/assets/inter-roman-greek.brO7ZQXV.woff2
./docs/.vitepress/dist/assets/inter-roman-greek-ext.CXd8zwEg.woff2
./docs/.vitepress/dist/assets/inter-roman-latin-ext.Cjca3GLw.woff2
./docs/.vitepress/dist/assets/inter-roman-latin.q5rAVC0E.woff2
./docs/.vitepress/dist/assets/inter-roman-symbols.CQZtw9ew.woff2
./docs/.vitepress/dist/assets/inter-roman-vietnamese.DaEDpkt8.woff2
./docs/.vitepress/dist/assets/lifecycle.md.XoPlhGsA.js
./docs/.vitepress/dist/assets/lifecycle.md.XoPlhGsA.lean.js
./docs/.vitepress/dist/assets/schema.md.Dzx-DKjg.js
./docs/.vitepress/dist/assets/schema.md.Dzx-DKjg.lean.js
./docs/.vitepress/dist/assets/source.md.C-yeD5If.js
./docs/.vitepress/dist/assets/source.md.C-yeD5If.lean.js
./docs/.vitepress/dist/assets/style.B1ka8l8C.css
./docs/.vitepress/dist/assets/target.md.DEMxUoWj.js
./docs/.vitepress/dist/assets/target.md.DEMxUoWj.lean.js
./docs/.vitepress/dist/assets/ui.md.D2qD-k6N.js
./docs/.vitepress/dist/assets/ui.md.D2qD-k6N.lean.js
./docs/.vitepress/dist/assets/validation.md.YY5Q_ikc.js
./docs/.vitepress/dist/assets/validation.md.YY5Q_ikc.lean.js
./docs/.vitepress/dist/assets/validation-result.dJzk4R7Q.png
./docs/.vitepress/dist/assets/validation-result-env.B7hPL6lc.png
./docs/.vitepress/dist/assets/validation-result-tree.yerUp0Rp.png
./docs/.vitepress/dist/assets/vp-icons.139f4833.css
./docs/.vitepress/dist/assets/web.md.C5RX5zbt.js
./docs/.vitepress/dist/assets/web.md.C5RX5zbt.lean.js
./docs/.vitepress/dist/assets/审计-文档一致性.md.-9uNMrLE.js
./docs/.vitepress/dist/assets/审计-文档一致性.md.-9uNMrLE.lean.js
./docs/.vitepress/dist/audit-doc-consistency.html
./docs/.vitepress/dist/build.html
./docs/.vitepress/dist/cli.html
./docs/.vitepress/dist/design-prototypes.html
./docs/.vitepress/dist/editor/app.css
./docs/.vitepress/dist/editor/app.js
./docs/.vitepress/dist/editor/index.html
./docs/.vitepress/dist/en/architecture.html
./docs/.vitepress/dist/en/assets.html
./docs/.vitepress/dist/en/build.html
./docs/.vitepress/dist/en/cli.html
./docs/.vitepress/dist/en/design-prototypes.html
./docs/.vitepress/dist/en/example.html
./docs/.vitepress/dist/en/index.html
./docs/.vitepress/dist/en/lifecycle.html
./docs/.vitepress/dist/en/schema.html
./docs/.vitepress/dist/en/source.html
./docs/.vitepress/dist/en/target.html
./docs/.vitepress/dist/en/ui.html
./docs/.vitepress/dist/en/validation.html
./docs/.vitepress/dist/en/web.html
./docs/.vitepress/dist/example.html
./docs/.vitepress/dist/favicon.svg
./docs/.vitepress/dist/hashmap.json
./docs/.vitepress/dist/index.html
./docs/.vitepress/dist/lifecycle.html
./docs/.vitepress/dist/logo.svg
./docs/.vitepress/dist/schema.html
./docs/.vitepress/dist/source.html
./docs/.vitepress/dist/status/app.css
./docs/.vitepress/dist/status/app.js
./docs/.vitepress/dist/status/index.html
./docs/.vitepress/dist/target.html
./docs/.vitepress/dist/ui.html
./docs/.vitepress/dist/validation.html
./docs/.vitepress/dist/web.html
./docs/.vitepress/dist/审计-文档一致性.html
./docs/.vitepress/theme/custom.css
./docs/.vitepress/theme/index.js
./docs/.vitepress/theme/Mermaid.vue
./docs/web.en.md
./docs/web.md
./docs/审计-文档一致性.md
./docs/需求整理.md
./editors/vscode/cage-0.1.0.vsix
./editors/vscode/LICENSE
./editors/vscode/out/extension.js
./editors/vscode/package.json
./editors/vscode/package-lock.json
./editors/vscode/README.md
./editors/vscode/src/extension.ts
./editors/vscode/test/extension.test.mjs
./editors/vscode/test/handshake.test.mjs
./editors/vscode/test/package.test.mjs
./editors/vscode/tsconfig.json
./editors/vscode/.vscodeignore
./examples/asset-workspace/assets.json
./examples/asset-workspace/build/client/csv/Character.csv
./examples/asset-workspace/build/client/csv/Item.csv
./examples/asset-workspace/build/client/json/Character.json
./examples/asset-workspace/build/client/json/Item.json
./examples/asset-workspace/build/client/msgpack/Character.msgpack
./examples/asset-workspace/build/client/msgpack/Item.msgpack
./examples/asset-workspace/build/client/py/cage_enums.py
./examples/asset-workspace/build/client/py/Character.py
./examples/asset-workspace/build/client/py/Item.py
./examples/asset-workspace/build/manifest.json
./examples/asset-workspace/build/server/cs/CageEnums.cs
./examples/asset-workspace/build/server/cs/Character.cs
./examples/asset-workspace/build/server/cs/Item.cs
./examples/asset-workspace/build/server/json/Character.json
./examples/asset-workspace/build/server/json/Item.json
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/data/client/csv/Character.csv
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/data/client/csv/Item.csv
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/data/client/json/Character.json
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/data/client/json/Item.json
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/data/client/msgpack/Character.msgpack
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/data/client/msgpack/Item.msgpack
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/generated/client/py/cage_enums.py
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/generated/client/py/Character.py
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/generated/client/py/Item.py
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/HASHES.json
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/manifest.json
./examples/asset-workspace/build/snapshot/client-c43f2f0cfe82/schema.json
./examples/asset-workspace/cage.toml
./examples/asset-workspace/migrations/0001_rename_cost.yaml
./examples/asset-workspace/README.md
./examples/asset-workspace/schemas/character.yaml
./examples/asset-workspace/schemas/item.yaml
./examples/asset-workspace.sh
./examples/asset-workspace/sources/Character.json
./examples/asset-workspace/sources/Item.csv
./examples/game-config/bad/cage.toml
./examples/game-config/bad/config/Character.csv
./examples/game-config/bad/config/Item.yaml
./examples/game-config/bad/config/Quest.xlsx
./examples/game-config/bad/config/Shop.msgpack
./examples/game-config/bad/config/Stage.json
./examples/game-config/build/client/cpp/cage_enums.h
./examples/game-config/build/client/cpp/Character.h
./examples/game-config/build/client/cpp/Item.h
./examples/game-config/build/client/cpp/Quest.h
./examples/game-config/build/client/cpp/Shop.h
./examples/game-config/build/client/cpp/Stage.h
./examples/game-config/build/client/cs/CageEnums.cs
./examples/game-config/build/client/cs/Character.cs
./examples/game-config/build/client/cs/Item.cs
./examples/game-config/build/client/cs/Quest.cs
./examples/game-config/build/client/cs/Shop.cs
./examples/game-config/build/client/cs/Stage.cs
./examples/game-config/build/client/csv/Character.csv
./examples/game-config/build/client/csv/Item.csv
./examples/game-config/build/client/csv/Quest.csv
./examples/game-config/build/client/csv/Shop.csv
./examples/game-config/build/client/csv/Stage.csv
./examples/game-config/build/client/fbs/cage_enums.fbs
./examples/game-config/build/client/fbs/Character.fbs
./examples/game-config/build/client/fbs/Item.fbs
./examples/game-config/build/client/fbs/Quest.fbs
./examples/game-config/build/client/fbs/Shop.fbs
./examples/game-config/build/client/fbs/Stage.fbs
./examples/game-config/build/client/go/cage_enums.go
./examples/game-config/build/client/go/Character.go
./examples/game-config/build/client/go/Item.go
./examples/game-config/build/client/go/Quest.go
./examples/game-config/build/client/go/Shop.go
./examples/game-config/build/client/go/Stage.go
./examples/game-config/build/client/java/CageEnums.java
./examples/game-config/build/client/java/Character.java
./examples/game-config/build/client/java/Item.java
./examples/game-config/build/client/java/Quest.java
./examples/game-config/build/client/java/Shop.java
./examples/game-config/build/client/java/Stage.java
./examples/game-config/build/client/js/cage_enums.d.ts
./examples/game-config/build/client/js/cage_enums.js
./examples/game-config/build/client/js/Character.d.ts
./examples/game-config/build/client/js/Character.js
./examples/game-config/build/client/js/Item.d.ts
./examples/game-config/build/client/js/Item.js
./examples/game-config/build/client/json/Character.json
./examples/game-config/build/client/json/Item.json
./examples/game-config/build/client/json/Quest.json
./examples/game-config/build/client/jsonschema/Character.schema.json
./examples/game-config/build/client/jsonschema/Item.schema.json
./examples/game-config/build/client/jsonschema/Quest.schema.json
./examples/game-config/build/client/jsonschema/Shop.schema.json
./examples/game-config/build/client/jsonschema/Stage.schema.json
./examples/game-config/build/client/json/Shop.json
./examples/game-config/build/client/json/Stage.json
./examples/game-config/build/client/js/Quest.d.ts
./examples/game-config/build/client/js/Quest.js
./examples/game-config/build/client/js/Shop.d.ts
./examples/game-config/build/client/js/Shop.js
./examples/game-config/build/client/js/Stage.d.ts
./examples/game-config/build/client/js/Stage.js
./examples/game-config/build/client/lua/cage_enums.lua
./examples/game-config/build/client/lua/Character.lua
./examples/game-config/build/client/lua/Item.lua
./examples/game-config/build/client/lua/Quest.lua
./examples/game-config/build/client/lua/Shop.lua
./examples/game-config/build/client/lua/Stage.lua
./examples/game-config/build/client/msgpack/Character.msgpack
./examples/game-config/build/client/msgpack/Item.msgpack
./examples/game-config/build/client/msgpack/Quest.msgpack
./examples/game-config/build/client/msgpack/Shop.msgpack
./examples/game-config/build/client/msgpack/Stage.msgpack
./examples/game-config/build/client/proto/cage_enums.proto
./examples/game-config/build/client/proto/Character.proto
./examples/game-config/build/client/proto/Item.proto
./examples/game-config/build/client/proto/Quest.proto
./examples/game-config/build/client/proto/Shop.proto
./examples/game-config/build/client/proto/Stage.proto
./examples/game-config/build/client/py/cage_enums.py
./examples/game-config/build/client/py/Character.py
./examples/game-config/build/client/py/Item.py
./examples/game-config/build/client/py/Quest.py
./examples/game-config/build/client/py/Shop.py
./examples/game-config/build/client/py/Stage.py
./examples/game-config/build/client/ts/cage_enums.ts
./examples/game-config/build/client/ts/Character.ts
./examples/game-config/build/client/ts/Item.ts
./examples/game-config/build/client/ts/Quest.ts
./examples/game-config/build/client/ts/Shop.ts
./examples/game-config/build/client/ts/Stage.ts
./examples/game-config/build/manifest.json
./examples/game-config/build/server/cs/CageEnums.cs
./examples/game-config/build/server/cs/Character.cs
./examples/game-config/build/server/cs/Item.cs
./examples/game-config/build/server/cs/Quest.cs
./examples/game-config/build/server/cs/Shop.cs
./examples/game-config/build/server/cs/Stage.cs
./examples/game-config/build/server/csv/Character.csv
./examples/game-config/build/server/csv/Item.csv
./examples/game-config/build/server/csv/Quest.csv
./examples/game-config/build/server/csv/Shop.csv
./examples/game-config/build/server/csv/Stage.csv
./examples/game-config/build/server/json/Character.json
./examples/game-config/build/server/json/Item.json
./examples/game-config/build/server/json/Quest.json
./examples/game-config/build/server/json/Shop.json
./examples/game-config/build/server/json/Stage.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/csv/Character.csv
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/csv/Item.csv
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/csv/Quest.csv
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/csv/Shop.csv
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/csv/Stage.csv
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/json/Character.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/json/Item.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/json/Quest.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/json/Shop.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/json/Stage.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/msgpack/Character.msgpack
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/msgpack/Item.msgpack
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/msgpack/Quest.msgpack
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/msgpack/Shop.msgpack
./examples/game-config/build/snapshot/client-850ac5e6ac32/data/client/msgpack/Stage.msgpack
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cpp/cage_enums.h
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cpp/Character.h
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cpp/Item.h
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cpp/Quest.h
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cpp/Shop.h
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cpp/Stage.h
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cs/CageEnums.cs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cs/Character.cs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cs/Item.cs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cs/Quest.cs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cs/Shop.cs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/cs/Stage.cs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/fbs/cage_enums.fbs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/fbs/Character.fbs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/fbs/Item.fbs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/fbs/Quest.fbs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/fbs/Shop.fbs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/fbs/Stage.fbs
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/go/cage_enums.go
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/go/Character.go
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/go/Item.go
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/go/Quest.go
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/go/Shop.go
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/go/Stage.go
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/java/CageEnums.java
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/java/Character.java
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/java/Item.java
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/java/Quest.java
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/java/Shop.java
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/java/Stage.java
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/cage_enums.d.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/cage_enums.js
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Character.d.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Character.js
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Item.d.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Item.js
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/jsonschema/Character.schema.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/jsonschema/Item.schema.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/jsonschema/Quest.schema.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/jsonschema/Shop.schema.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/jsonschema/Stage.schema.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Quest.d.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Quest.js
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Shop.d.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Shop.js
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Stage.d.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/js/Stage.js
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/lua/cage_enums.lua
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/lua/Character.lua
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/lua/Item.lua
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/lua/Quest.lua
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/lua/Shop.lua
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/lua/Stage.lua
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/proto/cage_enums.proto
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/proto/Character.proto
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/proto/Item.proto
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/proto/Quest.proto
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/proto/Shop.proto
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/proto/Stage.proto
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/py/cage_enums.py
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/py/Character.py
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/py/Item.py
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/py/Quest.py
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/py/Shop.py
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/py/Stage.py
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/ts/cage_enums.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/ts/Character.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/ts/Item.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/ts/Quest.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/ts/Shop.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/generated/client/ts/Stage.ts
./examples/game-config/build/snapshot/client-850ac5e6ac32/HASHES.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/manifest.json
./examples/game-config/build/snapshot/client-850ac5e6ac32/schema.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/csv/Character.csv
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/csv/Item.csv
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/csv/Quest.csv
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/csv/Shop.csv
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/csv/Stage.csv
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/json/Character.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/json/Item.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/json/Quest.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/json/Shop.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/json/Stage.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/msgpack/Character.msgpack
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/msgpack/Item.msgpack
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/msgpack/Quest.msgpack
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/msgpack/Shop.msgpack
./examples/game-config/build/snapshot/client-df808dfaa1cd/data/client/msgpack/Stage.msgpack
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cpp/cage_enums.h
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cpp/Character.h
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cpp/Item.h
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cpp/Quest.h
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cpp/Shop.h
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cpp/Stage.h
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cs/CageEnums.cs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cs/Character.cs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cs/Item.cs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cs/Quest.cs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cs/Shop.cs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/cs/Stage.cs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/fbs/cage_enums.fbs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/fbs/Character.fbs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/fbs/Item.fbs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/fbs/Quest.fbs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/fbs/Shop.fbs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/fbs/Stage.fbs
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/go/cage_enums.go
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/go/Character.go
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/go/Item.go
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/go/Quest.go
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/go/Shop.go
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/go/Stage.go
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/java/CageEnums.java
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/java/Character.java
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/java/Item.java
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/java/Quest.java
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/java/Shop.java
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/java/Stage.java
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/cage_enums.d.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/cage_enums.js
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Character.d.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Character.js
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Item.d.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Item.js
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/jsonschema/Character.schema.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/jsonschema/Item.schema.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/jsonschema/Quest.schema.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/jsonschema/Shop.schema.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/jsonschema/Stage.schema.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Quest.d.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Quest.js
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Shop.d.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Shop.js
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Stage.d.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/js/Stage.js
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/lua/cage_enums.lua
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/lua/Character.lua
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/lua/Item.lua
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/lua/Quest.lua
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/lua/Shop.lua
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/lua/Stage.lua
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/proto/cage_enums.proto
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/proto/Character.proto
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/proto/Item.proto
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/proto/Quest.proto
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/proto/Shop.proto
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/proto/Stage.proto
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/py/cage_enums.py
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/py/Character.py
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/py/Item.py
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/py/Quest.py
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/py/Shop.py
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/py/Stage.py
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/ts/cage_enums.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/ts/Character.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/ts/Item.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/ts/Quest.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/ts/Shop.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/generated/client/ts/Stage.ts
./examples/game-config/build/snapshot/client-df808dfaa1cd/HASHES.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/manifest.json
./examples/game-config/build/snapshot/client-df808dfaa1cd/schema.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/csv/Character.csv
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/csv/Item.csv
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/csv/Quest.csv
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/csv/Shop.csv
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/csv/Stage.csv
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/json/Character.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/json/Item.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/json/Quest.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/json/Shop.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/json/Stage.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/msgpack/Character.msgpack
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/msgpack/Item.msgpack
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/msgpack/Quest.msgpack
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/msgpack/Shop.msgpack
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/data/client/msgpack/Stage.msgpack
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cpp/cage_enums.h
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cpp/Character.h
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cpp/Item.h
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cpp/Quest.h
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cpp/Shop.h
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cpp/Stage.h
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cs/CageEnums.cs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cs/Character.cs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cs/Item.cs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cs/Quest.cs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cs/Shop.cs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/cs/Stage.cs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/fbs/cage_enums.fbs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/fbs/Character.fbs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/fbs/Item.fbs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/fbs/Quest.fbs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/fbs/Shop.fbs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/fbs/Stage.fbs
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/go/cage_enums.go
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/go/Character.go
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/go/Item.go
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/go/Quest.go
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/go/Shop.go
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/go/Stage.go
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/java/CageEnums.java
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/java/Character.java
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/java/Item.java
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/java/Quest.java
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/java/Shop.java
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/java/Stage.java
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/cage_enums.d.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/cage_enums.js
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Character.d.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Character.js
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Item.d.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Item.js
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/jsonschema/Character.schema.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/jsonschema/Item.schema.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/jsonschema/Quest.schema.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/jsonschema/Shop.schema.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/jsonschema/Stage.schema.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Quest.d.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Quest.js
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Shop.d.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Shop.js
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Stage.d.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/js/Stage.js
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/lua/cage_enums.lua
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/lua/Character.lua
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/lua/Item.lua
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/lua/Quest.lua
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/lua/Shop.lua
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/lua/Stage.lua
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/proto/cage_enums.proto
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/proto/Character.proto
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/proto/Item.proto
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/proto/Quest.proto
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/proto/Shop.proto
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/proto/Stage.proto
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/py/cage_enums.py
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/py/Character.py
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/py/Item.py
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/py/Quest.py
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/py/Shop.py
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/py/Stage.py
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/ts/cage_enums.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/ts/Character.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/ts/Item.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/ts/Quest.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/ts/Shop.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/generated/client/ts/Stage.ts
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/HASHES.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/manifest.json
./examples/game-config/build/snapshot/client-prod-55dae1a8e145/schema.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/csv/Character.csv
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/csv/Item.csv
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/csv/Quest.csv
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/csv/Shop.csv
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/csv/Stage.csv
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/json/Character.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/json/Item.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/json/Quest.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/json/Shop.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/json/Stage.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/msgpack/Character.msgpack
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/msgpack/Item.msgpack
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/msgpack/Quest.msgpack
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/msgpack/Shop.msgpack
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/data/client/msgpack/Stage.msgpack
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cpp/cage_enums.h
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cpp/Character.h
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cpp/Item.h
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cpp/Quest.h
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cpp/Shop.h
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cpp/Stage.h
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cs/CageEnums.cs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cs/Character.cs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cs/Item.cs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cs/Quest.cs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cs/Shop.cs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/cs/Stage.cs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/fbs/cage_enums.fbs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/fbs/Character.fbs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/fbs/Item.fbs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/fbs/Quest.fbs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/fbs/Shop.fbs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/fbs/Stage.fbs
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/go/cage_enums.go
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/go/Character.go
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/go/Item.go
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/go/Quest.go
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/go/Shop.go
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/go/Stage.go
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/java/CageEnums.java
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/java/Character.java
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/java/Item.java
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/java/Quest.java
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/java/Shop.java
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/java/Stage.java
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/cage_enums.d.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/cage_enums.js
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Character.d.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Character.js
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Item.d.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Item.js
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/jsonschema/Character.schema.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/jsonschema/Item.schema.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/jsonschema/Quest.schema.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/jsonschema/Shop.schema.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/jsonschema/Stage.schema.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Quest.d.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Quest.js
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Shop.d.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Shop.js
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Stage.d.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/js/Stage.js
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/lua/cage_enums.lua
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/lua/Character.lua
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/lua/Item.lua
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/lua/Quest.lua
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/lua/Shop.lua
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/lua/Stage.lua
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/proto/cage_enums.proto
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/proto/Character.proto
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/proto/Item.proto
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/proto/Quest.proto
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/proto/Shop.proto
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/proto/Stage.proto
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/py/cage_enums.py
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/py/Character.py
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/py/Item.py
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/py/Quest.py
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/py/Shop.py
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/py/Stage.py
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/ts/cage_enums.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/ts/Character.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/ts/Item.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/ts/Quest.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/ts/Shop.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/generated/client/ts/Stage.ts
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/HASHES.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/manifest.json
./examples/game-config/build/snapshot/client-prod-b926aded2ac8/schema.json
./examples/game-config/cage.toml
./examples/game-config/ci/github-actions.yml
./examples/game-config/ci/gitlab-ci.yml
./examples/game-config/ci/Jenkinsfile
./examples/game-config/config/Character.csv
./examples/game-config/config/Item.yaml
./examples/game-config/config/Quest.xlsx
./examples/game-config/config/Shop.msgpack
./examples/game-config/config/Stage.json
./examples/game-config/migrations/0001_rename_desc.yaml
./examples/game-config/README.md
./examples/game-config/schemas/character.yaml
./examples/game-config/schemas/item.yaml
./examples/game-config/schemas/quest.yaml
./examples/game-config/schemas/shop.yaml
./examples/game-config/schemas/stage.yaml
./examples/run.sh
./examples/web-demo/cage.toml
./examples/web-demo/config/Item.json
./examples/web-demo/schema.yaml
./examples/web-smoke.sh
./.github/workflows/ci.yml
./.github/workflows/daily.yml
./.github/workflows/deploy-docs.yml
./.github/workflows/nightly.yml
./.gitignore
./LICENSE
./package.json
./package-lock.json
./README.md
./README.zh.md
./scripts/install.ps1
./scripts/install.sh
./todo.md


[showing lines 235-946 of 946; full output saved to /home/cui/.local/share/opencode/shell/5c5b7fc368d18b4d9c5cf962a08a52154ffcde08/sh_14ef7f4d1001YMsfze5aZBl135.out]
[0m→ [0mRead Cargo.toml 
[0m→ [0mRead todo.md 
[0m→ [0mRead README.md [90m[limit=260][0m
[0m→ [0mRead README.zh.md [90m[limit=260][0m
[0m$ [0mfind crates -maxdepth 3 -type f -not -path '*/target/*' | sort
crates/cage-cli/Cargo.toml
crates/cage-cli/src/init.rs
crates/cage-cli/src/lsp.rs
crates/cage-cli/src/main.rs
crates/cage-cli/src/mcp.rs
crates/cage-cli/src/remote.rs
crates/cage-cli/src/web.rs
crates/cage-cli/tests/assets.rs
crates/cage-cli/tests/cli_paths.rs
crates/cage-cli/tests/cs_target.rs
crates/cage-cli/tests/db_source.rs
crates/cage-cli/tests/env_packaging.rs
crates/cage-cli/tests/env_validation.rs
crates/cage-cli/tests/gamerule.rs
crates/cage-cli/tests/gen.rs
crates/cage-cli/tests/golden.rs
crates/cage-cli/tests/incremental.rs
crates/cage-cli/tests/init_scaffold.rs
crates/cage-cli/tests/lsp.rs
crates/cage-cli/tests/mcp_server.rs
crates/cage-cli/tests/migrate_draft.rs
crates/cage-cli/tests/migrate.rs
crates/cage-cli/tests/registry_push.rs
crates/cage-cli/tests/registry_remote.rs
crates/cage-cli/tests/registry.rs
crates/cage-cli/tests/registry_signing.rs
crates/cage-cli/tests/remote_source.rs
crates/cage-cli/tests/schema_draft.rs
crates/cage-cli/tests/sheets_source.rs
crates/cage-cli/tests/template_target.rs
crates/cage-cli/tests/web.rs
crates/cage-core/Cargo.toml
crates/cage-core/src/lib.rs
crates/cage-core/src/registry.rs
crates/cage-core/src/remote.rs
crates/cage-source-csv/Cargo.toml
crates/cage-source-csv/src/lib.rs
crates/cage-source-db/Cargo.toml
crates/cage-source-db/src/draft.rs
crates/cage-source-db/src/lib.rs
crates/cage-source-db/src/mysql.rs
crates/cage-source-db/src/pg.rs
crates/cage-source-db/src/pool.rs
crates/cage-source-excel/Cargo.toml
crates/cage-source-excel/src/lib.rs
crates/cage-source-http/Cargo.toml
crates/cage-source-http/src/lib.rs
crates/cage-source-json/Cargo.toml
crates/cage-source-json/src/lib.rs
crates/cage-source-msgpack/Cargo.toml
crates/cage-source-msgpack/src/lib.rs
crates/cage-source-sheets/Cargo.toml
crates/cage-source-sheets/src/lib.rs
crates/cage-source-yaml/Cargo.toml
crates/cage-source-yaml/src/lib.rs
crates/cage-target-cpp/Cargo.toml
crates/cage-target-cpp/src/lib.rs
crates/cage-target-cpp/templates/enums.h.tera
crates/cage-target-cpp/templates/table.h.tera
crates/cage-target-cs/Cargo.toml
crates/cage-target-cs/src/lib.rs
crates/cage-target-cs/templates/enums.cs.tera
crates/cage-target-cs/templates/table.cs.tera
crates/cage-target-csv/Cargo.toml
crates/cage-target-csv/src/lib.rs
crates/cage-target-flatbuffers/Cargo.toml
crates/cage-target-flatbuffers/src/lib.rs
crates/cage-target-go/Cargo.toml
crates/cage-target-go/src/lib.rs
crates/cage-target-go/templates/enums.go.tera
crates/cage-target-go/templates/table.go.tera
crates/cage-target-java/Cargo.toml
crates/cage-target-java/src/lib.rs
crates/cage-target-java/templates/enums.java.tera
crates/cage-target-java/templates/table.java.tera
crates/cage-target-json/Cargo.toml
crates/cage-target-jsonschema/Cargo.toml
crates/cage-target-jsonschema/src/lib.rs
crates/cage-target-json/src/lib.rs
crates/cage-target-lua/Cargo.toml
crates/cage-target-lua/src/lib.rs
crates/cage-target-lua/templates/enums.lua.tera
crates/cage-target-lua/templates/table.lua.tera
crates/cage-target-msgpack/Cargo.toml
crates/cage-target-msgpack/src/lib.rs
crates/cage-target-proto/Cargo.toml
crates/cage-target-proto/src/lib.rs
crates/cage-target-py/Cargo.toml
crates/cage-target-py/src/lib.rs
crates/cage-target-py/templates/enums.py.tera
crates/cage-target-py/templates/table.py.tera
crates/cage-target-template/Cargo.toml
crates/cage-target-template/src/lib.rs
crates/cage-target-ts/Cargo.toml
crates/cage-target-ts/src/lib.rs
crates/cage-target-ts/templates/enums.d.ts.tera
crates/cage-target-ts/templates/enums.js.tera
crates/cage-target-ts/templates/enums.ts.tera
crates/cage-target-ts/templates/table.d.ts.tera
crates/cage-target-ts/templates/table.js.tera
crates/cage-target-ts/templates/table.ts.tera
[0m$ [0mfind docs -maxdepth 1 -type f -name '*.md' -print | sort && find .github -type f -maxdepth 3 -print | sort
docs/architecture.en.md
docs/architecture.md
docs/assets.en.md
docs/assets.md
docs/audit-doc-consistency.md
docs/build.en.md
docs/build.md
docs/cli.en.md
docs/cli.md
docs/design.en.md
docs/design.md
docs/design-prototypes.en.md
docs/design-prototypes.md
docs/example.en.md
docs/example.md
docs/index.en.md
docs/index.md
docs/lifecycle.en.md
docs/lifecycle.md
docs/requirements.md
docs/schema.en.md
docs/schema.md
docs/source.en.md
docs/source.md
docs/target.en.md
docs/target.md
docs/ui.en.md
docs/ui.md
docs/validation.en.md
docs/validation.md
docs/web.en.md
docs/web.md
docs/审计-文档一致性.md
docs/需求整理.md
.github/workflows/ci.yml
.github/workflows/daily.yml
.github/workflows/deploy-docs.yml
.github/workflows/nightly.yml
[0m✱ [0mGrep "TODO|FIXME|todo!\(|unimplemented!\(|panic!\(" [90min crates · 64 matches[0m
[0m→ [0mRead docs/audit-doc-consistency.md 
[0m→ [0mRead docs/审计-文档一致性.md 
[0m$ [0mfind crates/cage-core/src -type f | sort; wc -l crates/cage-cli/src/*.rs crates/cage-core/src/**/*.rs crates/cage-core/src/*.rs
crates/cage-core/src/assets/mod.rs
crates/cage-core/src/assets/tests.rs
crates/cage-core/src/diagnostics/mod.rs
crates/cage-core/src/edit/mod.rs
crates/cage-core/src/error/codes.rs
crates/cage-core/src/error/mod.rs
crates/cage-core/src/lib.rs
crates/cage-core/src/manifest/mod.rs
crates/cage-core/src/migrate/diff.rs
crates/cage-core/src/migrate/mod.rs
crates/cage-core/src/normalize/mod.rs
crates/cage-core/src/reference/mod.rs
crates/cage-core/src/registry.rs
crates/cage-core/src/remote.rs
crates/cage-core/src/schema/mod.rs
crates/cage-core/src/snapshot/mod.rs
crates/cage-core/src/validation/expr.rs
crates/cage-core/src/validation/mod.rs
crates/cage-core/src/validation/rules.rs
crates/cage-core/src/value/mod.rs
   1135 crates/cage-cli/src/init.rs
    594 crates/cage-cli/src/lsp.rs
   7176 crates/cage-cli/src/main.rs
   1206 crates/cage-cli/src/mcp.rs
    268 crates/cage-cli/src/remote.rs
   1983 crates/cage-cli/src/web.rs
    254 crates/cage-core/src/assets/mod.rs
    165 crates/cage-core/src/assets/tests.rs
    857 crates/cage-core/src/diagnostics/mod.rs
    329 crates/cage-core/src/edit/mod.rs
    484 crates/cage-core/src/error/codes.rs
      6 crates/cage-core/src/error/mod.rs
   1499 crates/cage-core/src/manifest/mod.rs
   1220 crates/cage-core/src/migrate/diff.rs
   2267 crates/cage-core/src/migrate/mod.rs
   1021 crates/cage-core/src/normalize/mod.rs
   1239 crates/cage-core/src/reference/mod.rs
   1538 crates/cage-core/src/schema/mod.rs
    378 crates/cage-core/src/snapshot/mod.rs
    523 crates/cage-core/src/validation/expr.rs
   3937 crates/cage-core/src/validation/mod.rs
    294 crates/cage-core/src/validation/rules.rs
    719 crates/cage-core/src/value/mod.rs
     91 crates/cage-core/src/lib.rs
   4193 crates/cage-core/src/registry.rs
    697 crates/cage-core/src/remote.rs
  34073 total
[0m$ [0mcargo test --workspace --locked
Command moved to the background (shell ID: sh_14ef835e4001v0UKx2zFC40TF1).
Output is streaming to: /home/cui/.local/share/opencode/shell/5c5b7fc368d18b4d9c5cf962a08a52154ffcde08/sh_14ef835e4001v0UKx2zFC40TF1.out
[0m→ [0mRead docs/需求整理.md [90m[limit=240][0m
[0m→ [0mRead docs/architecture.md [90m[limit=340][0m
[0m→ [0mRead .github/workflows/ci.yml 
[0m✱ [0mGrep "pub fn (extract|import|export|verify|load|resolve|publish|gc|remove|push|apply|write)|fn (extract|import|export|verify|load|resolve|publish|gc|remove|push|apply|write)" [90min crates/cage-core/src · 67 matches[0m
[0m→ [0mRead docs/cli.md [90m[limit=300][0m
[0m✱ [0mGrep "security|unsafe|TODO|not implemented|reserved|未实现|留待|不支持|只读|权限" [90min docs · 168 matches[0m
[0m✱ [0mGrep "read_to_string|write\(|create_dir_all|remove_file|remove_dir_all|rename\(|canonicalize|strip_prefix|\.join\(" [90min crates/cage-cli/src · 250 matches[0m
[0m→ [0mRead docs/web.md [90m[limit=220][0m
[0m→ [0mRead docs/validation.md [90m[offset=460, limit=32][0m
[0m→ [0mRead docs/architecture.en.md [90m[offset=190, limit=32][0m
[0m→ [0mRead docs/review/gpt6-luna-review-2026-10-18.md [90m[limit=220][0m
[0m→ [0mRead docs/cli.md [90m[offset=420, limit=40][0m
[0m$ [0mgit status --short
(no output)
[0m→ [0mRead docs/review/gpt6-luna-review-2026-10-18.md [90m[offset=220, limit=300][0m
[0m✱ [0mGrep "##|缺陷|问题|差异|建议|Table|表一|P0|P1|Table 1|FINDING" [90min docs/review/gpt6-luna-review-2026-10-18.md · 6 matches[0m
[0m→ [0mRead docs/review/gpt6-luna-review-2026-10-18.md [90m[offset=1090, limit=100][0m
[0m→ [0mRead crates/cage-cli/src/lsp.rs [90m[limit=100][0m
[0m→ [0mRead crates/cage-cli/src/web.rs [90m[offset=50, limit=130][0m
[0m→ [0mRead crates/cage-source-db/src/pg.rs [90m[offset=125, limit=90][0m
[0m→ [0mRead crates/cage-cli/src/web.rs [90m[offset=330, limit=80][0m
[0m→ [0mRead crates/cage-core/src/snapshot/mod.rs [90m[offset=90, limit=100][0m
[0m→ [0mRead docs/lifecycle.md [90m[offset=124, limit=22][0m
[0m→ [0mRead docs/requirements.md [90m[offset=1, limit=35][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=280, limit=60][0m
[0m→ [0mRead crates/cage-core/src/validation/expr.rs [90m[offset=1, limit=80][0m
[0m→ [0mRead crates/cage-core/src/validation/mod.rs [90m[offset=860, limit=150][0m
[0m→ [0mRead docs/lifecycle.en.md [90m[offset=124, limit=18][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=1280, limit=360][0m
[0m→ [0mRead README.md [90m[offset=33, limit=12][0m
[0m→ [0mRead README.zh.md [90m[offset=35, limit=12][0m
[0m→ [0mRead crates/cage-core/src/validation/mod.rs [90m[offset=1020, limit=105][0m
[0m→ [0mRead docs/lifecycle.md [90m[offset=129, limit=10][0m
[0m→ [0mRead crates/cage-core/src/snapshot/mod.rs [90m[offset=177, limit=70][0m
[0m→ [0mRead docs/requirements.md [90m[offset=14, limit=14][0m
[0m→ [0mRead crates/cage-core/src/registry.rs [90m[offset=880, limit=210][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=1630, limit=130][0m
[0m→ [0mRead docs/cli.md [90m[offset=365, limit=50][0m
[0m→ [0mRead crates/cage-core/src/validation/mod.rs [90m[offset=1145, limit=90][0m
[0m→ [0mRead docs/source.md [90m[offset=16, limit=20][0m
[0m→ [0mRead docs/architecture.en.md [90m[offset=202, limit=8][0m
[0m→ [0mRead docs/architecture.md [90m[offset=205, limit=12][0m
[0m✱ [0mGrep "lint_env_overrides|environment.*env|env: Option|env_override|--env" [90min crates/cage-core/src/schema/mod.rs · 38 matches[0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=340, limit=100][0m
[0m✱ [0mGrep "let mut warnings|lint_env_overrides|warning:.*env override|lint_env" [90min crates/cage-cli/src/main.rs · 1 match[0m
[0m✱ [0mGrep "snapshot.*environment|--env|environment.*registry|publish.*env" [90min crates/cage-cli/tests/env_packaging.rs · 20 matches[0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=120, limit=60][0m
[0m✱ [0mGrep "fn validate_type|type mismatch|E1101|value_matches_type" [90min crates/cage-core/src/validation/mod.rs · 30 matches[0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=525, limit=28][0m
[0m→ [0mRead crates/cage-core/src/schema/mod.rs [90m[offset=35, limit=60][0m
[0m→ [0mRead docs/cli.en.md [90m[offset=800, limit=22][0m
[0m→ [0mRead crates/cage-core/src/validation/mod.rs [90m[offset=445, limit=85][0m
[0m→ [0mRead crates/cage-core/src/snapshot/mod.rs [90m[offset=20, limit=74][0m
[0m✱ [0mGrep "Snapshot \{|pub enum|enum Commands|env: Option<String>|Run snapshot|snapshot.*env" [90min crates/cage-cli/src/main.rs · 19 matches[0m
[0m✱ [0mGrep "Lsp \{|Lsp,|Commands::Lsp|pub fn run_lsp|Language Server" [90min crates/cage-cli/src · 5 matches[0m
[0m→ [0mRead docs/design.md [90m[offset=3128, limit=24][0m
[0m→ [0mRead docs/cli.md [90m[offset=415, limit=12][0m
[0m→ [0mRead crates/cage-core/src/registry.rs [90m[offset=1820, limit=108][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=630, limit=32][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=2258, limit=76][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=2140, limit=55][0m
[0m✱ [0mGrep "regenerate_prefixes|output_dir.*format|target_output|target_records" [90min crates/cage-cli/src/main.rs · 13 matches[0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=2058, limit=80][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=1975, limit=65][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=2438, limit=35][0m
[0m✱ [0mGrep "fn write_artifact_files|struct ArtifactItem|artifact.path" [90min crates/cage-cli/src/main.rs · 4 matches[0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=3478, limit=42][0m
[0m→ [0mRead crates/cage-cli/src/main.rs [90m[offset=4748, limit=40][0m
[0m→ [0mRead editors/vscode/src/extension.ts [90m[limit=80][0m
### 表一 缺陷

| 问题 | file:line | 为什么是问题 | 建议动作 |
|---|---|---|---|
| **高｜增量构建可能删除输出目录外的文件** | `crates/cage-cli/src/main.rs:2005-2007,2079-2084,2168-2177` | 增量构建直接读取旧 manifest；目标移除或变更时，用字符串前缀筛选旧产物路径，再执行 `remove_file(path.join(rel))`。恶意或损坏的 manifest 可用 `build/.../../../..` 一类路径通过前缀判断，造成项目目录外文件被删除。 | 读取 manifest 后校验产物路径与 target 输出路径都是规范化相对路径；实际读写/删除前再校验规范路径仍位于输出根目录内。 |
| **高｜本地 Web 服务缺少请求边界保护** | `crates/cage-cli/src/web.rs:60,73-75,86-105,119-145` | 服务只绑定 loopback，但对 POST 请求无上限地读完整 body，且单线程处理；大请求可耗尽内存或阻塞其他请求。`/api/schema` 可写文件，`/api/build`、`/api/snapshot` 可触发写操作。跨站请求是否能成功触发受浏览器策略影响，**未核实**。 | 限制请求体大小和读取时间；校验 `Host`/`Origin`，为写操作增加 CSRF 防护或一次性本地 token。 |
| **高｜快照账本不能认证发布者** | `crates/cage-core/src/snapshot/mod.rs:46-47,101-128,149-174` | `HASHES.json` 与被校验文件放在同一快照中，是校验信任根；同时替换数据和账本仍可通过 `verify_snapshot`。因此它能发现单独损坏，不足以证明快照来自可信发布者。 | 快照加载支持基于外部可信公钥的强制验签；或明确把快照校验限定为内容一致性检查。 |
| **中高｜迁移写回不是事务性的** | `crates/cage-cli/src/main.rs:1633-1634,1662-1671` | 文件按序逐个直接 `fs::write`。写到后续文件失败或进程中断时，前面的源文件已迁移、后面的仍是旧数据；当前文件也可能被截断，留下不完整内容。 | 先将全部结果写入临时文件并完成校验，再原子替换；提供失败回滚或备份恢复机制。 |

### 表二 文档与实现不一致

| 文档说法 | 文档位置 | 代码实际 | 代码位置 |
|---|---|---|---|
| 引用谓词校验“预留”。 | `README.zh.md:40`；`README.md:38` | 已解析谓词并在目标行上求值；不满足时报告 `E1410`。 | `crates/cage-core/src/validation/mod.rs:1195-1217` |
| `schema-draft --db ...`、`migrate-draft . --out ...` 是迁移流程命令。 | `docs/lifecycle.md:132-133` | `schema-draft` 要求项目路径和 `mysql:`/`pg:` spec；`migrate-draft` 要求两个 schema 文件以及 `--from`、`--to`。示例命令不符合 CLI 参数定义。 | `crates/cage-cli/src/main.rs:292-328` |
| 未选中环境的坏覆盖 lint 需要新错误码；环境化 snapshot / registry 出包尚未实现。 | `docs/architecture.md:211`；`docs/architecture.en.md:207` | 基线环境已逐条输出 lint warning；snapshot 和 registry publish 均接受 `--env`。 | `crates/cage-core/src/schema/mod.rs:39-55`；`crates/cage-cli/src/main.rs:537-548,2266-2296,2312-2315,340-347` |
| IDE 插件留待后续接入。 | `docs/validation.md:479` | 仓库已有 `cage lsp` 和 VSCode 扩展，扩展会启动该 LSP。 | `crates/cage-cli/src/main.rs:257-264`；`editors/vscode/src/extension.ts:1-5,29-55` |
| CSV 类型推断后由 Schema 在 L2“严格校准”。 | `docs/需求整理.md:18`；`docs/requirements.md:18` | L2 对值与字段类型族做严格匹配，不匹配时报 `E1101`；不会在此处自动转换类型。 | `crates/cage-core/src/validation/mod.rs:450-468,474-519` |
| 快照篡改会被暴露。 | `README.zh.md:44`；`README.md:42` | 校验使用快照内的 `HASHES.json` 作为信任根；同时改文件与账本时，不能据此识别恶意替换。 | `crates/cage-core/src/snapshot/mod.rs:46-47,101-128` |

### 表三 值得补的能力

| 能力 | 为什么现在没有 | 价值高低 |
|---|---|---|
| 快照签名与强制验签 | 快照目前只用同包账本核对文件哈希；验签能力只覆盖 bundle，不覆盖运行时快照加载。`crates/cage-core/src/snapshot/mod.rs:101-128`；`crates/cage-core/src/registry.rs:1871-1886` | 高 |
| LSP 基于未保存缓冲区的行列诊断、补全与跳转 | 当前 LSP 明确从磁盘校验，并把诊断映射为文件级零宽位置；补全等功能明确不做。`crates/cage-cli/src/lsp.rs:14-23`；`docs/cli.md:379-395` | 中高 |
| 数据库源的行数/字节预算与 PostgreSQL 流式读取 | PostgreSQL 当前先缓冲完整查询结果，再转换为内部行集；设计文档说明该路径维持缓冲，未见资源上限。`crates/cage-source-db/src/pg.rs:160-170,174-190`；`docs/design.md:3135-3143` | 中 |
