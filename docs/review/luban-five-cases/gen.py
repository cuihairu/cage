#!/usr/bin/env python3
"""Generate the Luban vs Cage five-case comparison projects.

One logical game-config dataset, emitted twice:
  luban/  — Luban 5.1 project (Defines/schema.xml + Data/*.xlsx)
  cage/   — Cage project (cage.toml + schemas/*.yaml + config/*.xlsx)

Deterministic (fixed seed) so both sides always describe the same rows.
"""
import os
import random

from openpyxl import Workbook

ROOT = os.path.dirname(os.path.abspath(__file__))
random.seed(42)

# ---------------------------------------------------------------- dataset

N_HERO = 300
N_SKILL = 200
N_STAGE = 150
N_GAMEPLAY = 250
N_SHOP = 200

ELEMENTS = ["fire", "water", "earth", "wind"]
EVENTS = ["on_cast", "on_hit", "on_kill", "on_death"]
GROUPS = ["goblin", "wolf", "skeleton", "bat", "ogre", "slime"]


def heroes():
    rows = []
    for i in range(1, N_HERO + 1):
        # title: text on even rows (not %5), explicit "" on %5 rows, null on odd
        if i % 2 == 0:
            title = "" if i % 5 == 0 else f"Hero Title {i}"
        else:
            title = None
        # level: empty cell (default 1 fills) on every third row, explicit otherwise
        level = None if i % 3 == 0 else 1 + (i % 5)
        # nickname: text on even rows, null on odd
        nickname = f"nick_{i}" if i % 2 == 0 else None
        rows.append((i, f"hero_{i}", title, level, 100 + i, nickname))
    return rows


def skills():
    """(skill_row, effect_rows) — effects is a polymorphic list."""
    tables = []
    for i in range(1, N_SKILL + 1):
        effects = []
        n = 1 + (i % 3)
        for k in range(n):
            is_damage = (i + k) % 2 == 0
            effect = {
                "type": "DamageEffect" if is_damage else "HealEffect",
                "event": EVENTS[(i + k) % len(EVENTS)],
                "chance": round(0.1 + ((i * 7 + k * 3) % 9) / 10.0, 1),
                "damage": 10 + ((i * 13 + k * 29) % 500) if is_damage else None,
                "heal": 5 + ((i * 11 + k * 17) % 300) if not is_damage else None,
            }
            effects.append(effect)
        tables.append((
            # cd kept fractional: cage Float32 rejects whole-number cells
            (i, f"skill_{i}", 10 + (i % 90), round(0.25 + (i % 60) / 10.0, 2)),
            effects,
        ))
    return tables


def stages():
    """(stage_row, wave_rows) — each wave: groups (csv) + rewards (item|count pairs)."""
    tables = []
    for i in range(1, N_STAGE + 1):
        waves = []
        n = 2 + (i % 3)
        for k in range(n):
            g1, g2 = GROUPS[i % 6], GROUPS[(i + k + 1) % 6]
            r1 = 1 + ((i + k) % 50)
            r2 = 1 + ((i * 3 + k) % 50)
            waves.append((
                k + 1,                      # wave index
                f"{g1},{g2}",               # monster_groups (csv)
                f"{r1}|{1 + k % 3};{r2}|1", # rewards (sep ; between, | inside)
            ))
        tables.append(((i, f"stage_{i}"), waves))
    return tables


def gameplays():
    rows = []
    for i in range(1, N_GAMEPLAY + 1):
        kind = i % 3
        if kind == 0:
            payload = f"Damage|{10 + i % 400}|{ELEMENTS[i % 4]}"
        elif kind == 1:
            payload = f"Heal|{5 + i % 200}"
        else:
            # duration kept fractional: cage Float32 rejects whole-number cells
            payload = f"Buff|buff_{i % 30}|{round(0.25 + (i % 50) / 10.0, 2)}"
        rows.append((i, payload))
    return rows


def shop_items():
    rows = []
    for i in range(1, N_SHOP + 1):
        rows.append((i, f"shop_{i}", 10 + i * 2, 4 + i, 6 + i * 2, f"icon_{i}.png"))
    return rows


# ---------------------------------------------------------------- xlsx util


def write_sheet(wb, title, header_row, plus_row, type_row, data_rows, merges=()):
    """Write one Luban sheet: ##var / ##+ / ##type + data. Header cells start at B."""
    ws = wb.create_sheet(title)
    ws.append(header_row)
    if plus_row is not None:
        ws.append(plus_row)
    ws.append(type_row)
    for row in data_rows:
        ws.append([None] + list(row))
    for merge in merges:
        ws.merge_cells(merge)
    return ws


def save(wb, path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    wb.save(path)


# ---------------------------------------------------------------- luban side


def gen_luban():
    luban = os.path.join(ROOT, "luban")

    schema = """<module name="">
    <!-- case 4: optional / default / null -->
    <bean name="Hero">
        <var name="id" type="int"/>
        <var name="name" type="string"/>
        <var name="title" type="string?"/>
        <var name="level" type="int"/>
        <var name="hp" type="int"/>
        <var name="nickname" type="string?"/>
    </bean>
    <table name="TbHero" value="Hero" input="Hero@Hero.xlsx"/>

    <!-- case 1: skill — nested bean + polymorphic list (inheritance) -->
    <bean name="SkillCost">
        <var name="mp" type="int"/>
        <var name="cd" type="float"/>
    </bean>
    <bean name="SkillTrigger">
        <var name="event" type="string"/>
        <var name="chance" type="float"/>
    </bean>
    <bean name="SkillEffect">
        <var name="trigger" type="SkillTrigger"/>
        <bean name="DamageEffect">
            <var name="damage" type="int"/>
        </bean>
        <bean name="HealEffect">
            <var name="heal" type="int"/>
        </bean>
    </bean>
    <bean name="Skill">
        <var name="id" type="int"/>
        <var name="name" type="string"/>
        <var name="cost" type="SkillCost"/>
        <var name="effects" type="(list),SkillEffect"/>
    </bean>
    <table name="TbSkill" value="Skill" input="Skill@Skill.xlsx"/>

    <!-- case 2: dungeon stage — nested waves with csv groups + reward pairs -->
    <bean name="StageWave">
        <var name="index" type="int"/>
        <var name="monster_groups" type="(list#sep=,),string"/>
        <var name="rewards" type="(list#sep=;),string"/>
    </bean>
    <bean name="DungeonStage">
        <var name="id" type="int"/>
        <var name="name" type="string"/>
        <var name="waves" type="(list),StageWave"/>
    </bean>
    <table name="TbDungeonStage" value="DungeonStage" input="DungeonStage@DungeonStage.xlsx"/>

    <!-- case 3: polymorphic gameplay (type-tagged union, one cell) -->
    <bean name="Gameplay" sep="|">
        <bean name="Damage">
            <var name="amount" type="int"/>
            <var name="element" type="string"/>
        </bean>
        <bean name="Heal">
            <var name="amount" type="int"/>
        </bean>
        <bean name="Buff">
            <var name="buff_id" type="string"/>
            <var name="duration" type="float"/>
        </bean>
    </bean>
    <bean name="GameplayRow">
        <var name="id" type="int"/>
        <var name="gameplay" type="Gameplay"/>
    </bean>
    <table name="TbGameplay" value="GameplayRow" input="Gameplay@Gameplay.xlsx"/>

    <!-- case 5: client/server projection via groups -->
    <bean name="ShopItem">
        <var name="id" type="int"/>
        <var name="name" type="string"/>
        <var name="price" type="int" group="c"/>
        <var name="cost" type="int" group="s"/>
        <var name="margin" type="int" group="s"/>
        <var name="icon" type="string" group="c"/>
    </bean>
    <table name="TbShopItem" value="ShopItem" input="ShopItem@ShopItem.xlsx"/>
</module>
"""
    with open(os.path.join(luban, "Defines", "schema.xml"), "w") as f:
        f.write(schema)

    conf = """{
  "groups": [
    { "names": ["c"], "default": true },
    { "names": ["s"], "default": true }
  ],
  "schemaFiles": [ { "fileName": "Defines", "type": "" } ],
  "dataDir": "Data",
  "targets": [
    { "name": "client", "manager": "Tables", "groups": ["c"], "topModule": "cfg" },
    { "name": "server", "manager": "Tables", "groups": ["s"], "topModule": "cfg" },
    { "name": "all",    "manager": "Tables", "groups": ["c", "s"], "topModule": "cfg" }
  ],
  "xargs": []
}
"""
    with open(os.path.join(luban, "luban.conf"), "w") as f:
        f.write(conf)

    # Hero — flat; title/nickname nullable; level default via title tag
    wb = Workbook()
    ws = wb.active
    ws.title = "Hero"
    ws.append(["##var", "id", "name", "title", "level#default=1", "hp", "nickname"])
    ws.append(["##type", "int", "string", "string?", "int", "int", "string?"])
    for row in heroes():
        ws.append([None] + list(row))
    save(wb, os.path.join(luban, "Data", "Hero.xlsx"))

    # Skill — nested cost (merged), *effects polymorphic list (multi-row)
    wb = Workbook()
    ws = wb.active
    ws.title = "Skill"
    ws.append(["##var", "id", "name", "cost", None, "*effects", None, None, None, None])
    ws.merge_cells("D1:E1")
    ws.merge_cells("F1:J1")
    ws.append(["##+", None, None, "mp", "cd", "$type", "trigger", None, "damage", "heal"])
    ws.merge_cells("G2:H2")
    ws.append(["##type", "int", "string", "int", "float", "string", None, None, None, None])
    for skill_row, effects in skills():
        i, name, mp, cd = skill_row
        first = True
        for e in effects:
            if first:
                ws.append([None, i, name, mp, cd, e["type"], e["event"], e["chance"], e["damage"], e["heal"]])
                first = False
            else:
                ws.append([None, None, None, None, None, e["type"], e["event"], e["chance"], e["damage"], e["heal"]])
    save(wb, os.path.join(luban, "Data", "Skill.xlsx"))

    # DungeonStage — *waves multi-row list; groups csv; rewards "item|count;..."
    wb = Workbook()
    ws = wb.active
    ws.title = "DungeonStage"
    ws.append(["##var", "id", "name", "*waves", None, None, None])
    ws.merge_cells("D1:G1")
    ws.append(["##+", None, None, "index", "monster_groups", "rewards"])
    ws.append(["##type", "int", "string", "int", "list,string", "list,string"])
    for stage_row, waves in stages():
        (i, name) = stage_row
        first = True
        for w_index, groups, rewards in waves:
            if first:
                ws.append([None, i, name, w_index, groups, rewards])
                first = False
            else:
                ws.append([None, None, None, w_index, groups, rewards])
    save(wb, os.path.join(luban, "Data", "DungeonStage.xlsx"))

    # Gameplay — polymorphic single cell (sep |)
    wb = Workbook()
    ws = wb.active
    ws.title = "Gameplay"
    ws.append(["##var", "id", "gameplay"])
    ws.append(["##type", "int", "Gameplay"])
    for row in gameplays():
        ws.append([None] + list(row))
    save(wb, os.path.join(luban, "Data", "Gameplay.xlsx"))

    # ShopItem — flat; groups split columns
    wb = Workbook()
    ws = wb.active
    ws.title = "ShopItem"
    ws.append(["##var", "id", "name", "price", "cost", "margin", "icon"])
    ws.append(["##type", "int", "string", "int", "int", "int", "string"])
    for row in shop_items():
        ws.append([None] + list(row))
    save(wb, os.path.join(luban, "Data", "ShopItem.xlsx"))


# ---------------------------------------------------------------- cage side


CAGE_SCHEMA_COMMON = """\
metadata:
  version: "0.1.0"
  description: Luban five-case comparison — cage side (flat decomposition)
"""


def schema_yaml():
    """Cage schema: the same logical config, flat-decomposed."""
    skill_effects = []
    for _, effects in skills():
        skill_effects.extend(effects)
    effect_rows = [(i + 1, 1 + i % N_SKILL) for i in range(len(skill_effects))]

    hero = """tables:
  Hero:
    name: Hero
    description: case 4 — optional fields, defaults, null (empty cell)
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true, min: 1 }
      name: { name: name, type: { kind: String }, required: true }
      title:
        name: title
        type: { kind: Any }
        description: >-
          optional nullable string; cage has no nullable typed field, so the
          null-accepting equivalent is Any (luban uses string?)
      level:
        name: level
        type: { kind: Any }
        description: >-
          empty cells become null and cage's default only fills absent values,
          not excel-null; luban #default=1 fills the cell (case-4 finding)
      hp: { name: hp, type: { kind: Int32 }, required: true }
      nickname:
        name: nickname
        type: { kind: Any }
        description: optional nullable string; empty cell -> null (see title)
"""
    skill = """  Skill:
    name: Skill
    description: >-
      case 1 (flat form) — the nested bean became two columns; the polymorphic
      list became a referenced SkillEffect table (cage excel has no nested /
      polymorphic single-table form)
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true, min: 1 }
      name: { name: name, type: { kind: String }, required: true }
      cost_mp: { name: cost_mp, type: { kind: Int32 }, required: true, min: 0 }
      cost_cd:
        name: cost_cd
        type: { kind: Float32 }
        required: true
        min: 0
  SkillEffect:
    name: SkillEffect
    description: >-
      case 1 (flat form) — kind discriminates the union; trigger flattened to
      two columns; skill_id references Skill
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true, min: 1 }
      skill_id:
        name: skill_id
        type: { kind: Int32 }
        required: true
        reference: { table: Skill, field: id }
      kind:
        name: kind
        type: { kind: Enum, value: EffectKind }
        required: true
      trigger_event: { name: trigger_event, type: { kind: String }, required: true }
      trigger_chance: { name: trigger_chance, type: { kind: Float32 }, required: true, min: 0, max: 1 }
      damage:
        name: damage
        type: { kind: Any }
        description: >-
          DamageEffect only; null on HealEffect rows. cage has no nullable
          typed field, Any is the null-accepting equivalent (luban int?)
      heal:
        name: heal
        type: { kind: Any }
        description: HealEffect only; null on DamageEffect rows (see damage)
"""
    stage = """  DungeonStage:
    name: DungeonStage
    description: case 2 (flat form) — waves became a referenced DungeonWave table
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true, min: 1 }
      name: { name: name, type: { kind: String }, required: true }
  DungeonWave:
    name: DungeonWave
    description: case 2 (flat form) — one row per wave; groups/rewards arrays
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true, min: 1 }
      stage_id:
        name: stage_id
        type: { kind: Int32 }
        required: true
        reference: { table: DungeonStage, field: id }
      wave_index: { name: wave_index, type: { kind: Int32 }, required: true, min: 1 }
      monster_groups:
        name: monster_groups
        type: { kind: String }
        required: true
        description: >-
          csv "g1,g2"; cage excel cells cannot produce arrays, so the raw
          string is carried (luban (list#sep=,) parses it into an array)
      rewards:
        name: rewards
        type: { kind: String }
        required: true
        description: 'item|count pairs joined by ;, e.g. "12|3;4|1" (luban (list#sep=;))'
"""
    gameplay = """  Gameplay:
    name: Gameplay
    description: >-
      case 3 (flat form) — cage has no tagged union; the workaround is a kind
      enum with per-kind optional payload fields (type safety is lost)
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true, min: 1 }
      kind:
        name: kind
        type: { kind: Enum, value: GameplayKind }
        required: true
      amount:
        name: amount
        type: { kind: Any }
        description: Damage/Heal only; null on Buff rows (cage Any, luban int?)
      element:
        name: element
        type: { kind: Any }
        description: Damage only; null otherwise (see amount)
      buff_id:
        name: buff_id
        type: { kind: Any }
        description: Buff only; null otherwise (see amount)
      duration:
        name: duration
        type: { kind: Any }
        description: Buff only; null otherwise (see amount)
"""
    shop = """  ShopItem:
    name: ShopItem
    description: case 5 — client/server field projection via targets
    primary_key: [id]
    fields:
      id: { name: id, type: { kind: Int32 }, required: true, min: 1 }
      name: { name: name, type: { kind: String }, required: true }
      price:
        name: price
        type: { kind: Int32 }
        required: true
        min: 0
        default: 0
        targets: ["client"]
        description: client-only; default so the server profile can drop it
      cost:
        name: cost
        type: { kind: Int32 }
        required: true
        min: 0
        default: 0
        targets: ["server"]
        description: >-
          required server-only field; the default makes it droppable from the
          client profile (cage E9006 refuses required server-only otherwise)
      margin:
        name: margin
        type: { kind: Int32 }
        required: true
        min: 0
        default: 0
        targets: ["server"]
        description: server-only; default so the client profile can drop it
      icon:
        name: icon
        type: { kind: String }
        required: true
        default: "icon.png"
        targets: ["client"]
        description: client-only; default so the server profile can drop it
"""
    enums = """enums:
  EffectKind:
    name: EffectKind
    values:
      - { name: Damage, value: 1 }
      - { name: Heal, value: 2 }
  GameplayKind:
    name: GameplayKind
    values:
      - { name: Damage, value: 1 }
      - { name: Heal, value: 2 }
      - { name: Buff, value: 3 }
"""
    return CAGE_SCHEMA_COMMON + hero + skill + stage + gameplay + shop + enums, effect_rows


def cage_toml():
    return """output_dir = "build"
schema_path = "schemas"

[project]
name = "luban-five-cases-cage"
version = "0.1.0"
description = "Luban five-case comparison — cage side"

[source_roots]
main = "config"

[profiles.client]
name = "client"
targets = [
  { format = "json", output_dir = "build/client/json" },
]

[profiles.server]
name = "server"
targets = [
  { format = "json", output_dir = "build/server/json" },
]
"""


def cage_effect_rows():
    """Flatten the polymorphic effect list into cage's SkillEffect rows."""
    rows = []
    eid = 0
    for skill_row, effects in skills():
        skill_id = skill_row[0]
        for e in effects:
            eid += 1
            kind = "Damage" if e["type"] == "DamageEffect" else "Heal"
            rows.append((
                eid, skill_id, kind, e["event"], e["chance"], e["damage"], e["heal"],
            ))
    return rows


def cage_wave_rows():
    """Same raw cell text as luban's (the cell IS the string on the cage side)."""
    rows = []
    wid = 0
    for stage_row, waves in stages():
        stage_id = stage_row[0]
        for w_index, groups, rewards in waves:
            wid += 1
            rows.append((wid, stage_id, w_index, groups, rewards))
    return rows


def cage_gameplay_rows():
    rows = []
    for i, payload in gameplays():
        parts = payload.split("|")
        kind = parts[0]
        row = {
            "id": i,
            "kind": kind,  # enum cells are name strings on the cage side
        }
        if kind in ("Damage", "Heal"):
            row["amount"] = int(parts[1])
        if kind == "Damage":
            row["element"] = parts[2]
        if kind == "Buff":
            row["buff_id"] = parts[1]
            row["duration"] = float(parts[2])
        rows.append(row)
    return rows


def cage_xlsx_cell(v):
    """Arrays render as csv strings in the flat grid (cage infers strings)."""
    if isinstance(v, list):
        return ",".join(str(x) for x in v)
    return v


def gen_cage():
    cage = os.path.join(ROOT, "cage")
    os.makedirs(os.path.join(cage, "schemas"), exist_ok=True)
    os.makedirs(os.path.join(cage, "config"), exist_ok=True)

    with open(os.path.join(cage, "cage.toml"), "w") as f:
        f.write(cage_toml())

    schema, _ = schema_yaml()
    with open(os.path.join(cage, "schemas", "main.yaml"), "w") as f:
        f.write(schema)

    # Hero.xlsx
    wb = Workbook()
    ws = wb.active
    ws.title = "Hero"
    ws.append(["id", "name", "title", "level", "hp", "nickname"])
    for row in heroes():
        ws.append(list(row))
    save(wb, os.path.join(cage, "config", "Hero.xlsx"))

    # Skill.xlsx + SkillEffect.xlsx
    wb = Workbook()
    ws = wb.active
    ws.title = "Skill"
    ws.append(["id", "name", "cost_mp", "cost_cd"])
    for (i, name, mp, cd), _ in skills():
        ws.append([i, name, mp, cd])
    save(wb, os.path.join(cage, "config", "Skill.xlsx"))

    wb = Workbook()
    ws = wb.active
    ws.title = "SkillEffect"
    ws.append(["id", "skill_id", "kind", "trigger_event", "trigger_chance", "damage", "heal"])
    for row in cage_effect_rows():
        ws.append(list(row))
    save(wb, os.path.join(cage, "config", "SkillEffect.xlsx"))

    # DungeonStage.xlsx + DungeonWave.xlsx
    wb = Workbook()
    ws = wb.active
    ws.title = "DungeonStage"
    ws.append(["id", "name"])
    for (i, name), _ in stages():
        ws.append([i, name])
    save(wb, os.path.join(cage, "config", "DungeonStage.xlsx"))

    wb = Workbook()
    ws = wb.active
    ws.title = "DungeonWave"
    ws.append(["id", "stage_id", "wave_index", "monster_groups", "rewards"])
    for wid, stage_id, w_index, groups, rewards in cage_wave_rows():
        ws.append([wid, stage_id, w_index, groups, rewards])
    save(wb, os.path.join(cage, "config", "DungeonWave.xlsx"))

    # Gameplay.xlsx
    wb = Workbook()
    ws = wb.active
    ws.title = "Gameplay"
    ws.append(["id", "kind", "amount", "element", "buff_id", "duration"])
    for row in cage_gameplay_rows():
        ws.append([row.get("id"), row.get("kind"), row.get("amount"),
                   row.get("element"), row.get("buff_id"), row.get("duration")])
    save(wb, os.path.join(cage, "config", "Gameplay.xlsx"))

    # ShopItem.xlsx
    wb = Workbook()
    ws = wb.active
    ws.title = "ShopItem"
    ws.append(["id", "name", "price", "cost", "margin", "icon"])
    for row in shop_items():
        ws.append(list(row))
    save(wb, os.path.join(cage, "config", "ShopItem.xlsx"))


if __name__ == "__main__":
    os.makedirs(os.path.join(ROOT, "luban", "Defines"), exist_ok=True)
    os.makedirs(os.path.join(ROOT, "luban", "Data"), exist_ok=True)
    gen_luban()
    gen_cage()
    print("generated:", ROOT)
