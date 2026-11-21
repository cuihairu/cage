// rule_templates.js — P1-3 九项内置规则模板（纯逻辑，node --test 可测，
// 零依赖、无 DOM）。
//
// 每条模板是「策划想表达的一条业务规则」的高层入口，落在规则模式。它
// 们只把规则落到 Schema 的既有表示上，绝不引入新的规则格式：
//   - 直接改 Schema 的模板（主键唯一/数值范围/枚举选择/外键关系）产出
//     path 补丁，由表单层 setPathRoot 应用；
//   - 字段比较产出 AST，经 /api/rule/compile（P1-2）编译成 ExpressionRule
//     写进 `field.rules`——前端仍不手写 assert；
//   - 表达式引擎表达不了的模板（条件必填/数值成长/跨表总量/配置可达性）
//     不假装能自动转换（评审 141 行）：返回 boundary 引导，指向 Python
//     扩展（P2-2）或专门 Rust 校验器（P3-2），P1-5 使该引导更醒目。

export const RULE_TEMPLATES = [
  {
    id: "primary_key",
    name: "主键唯一",
    engine: "Rust Schema",
    expressible: true,
    hint: "把字段加入表主键（primary_key）",
  },
  {
    id: "range",
    name: "数值范围",
    engine: "Rust Schema",
    expressible: true,
    hint: "设字段 min/max（含端点）",
  },
  {
    id: "enum",
    name: "枚举选择",
    engine: "Rust Schema",
    expressible: true,
    hint: "把字段类型设为既有枚举",
  },
  {
    id: "reference",
    name: "外键关系",
    engine: "Rust 引用校验",
    expressible: true,
    hint: "设字段 reference（目标表/字段）",
  },
  {
    id: "field_compare",
    name: "字段比较",
    engine: "L6 表达式",
    expressible: true,
    hint: "字段与同行另一字段比较（经 AST 编译）",
  },
  {
    id: "conditional_required",
    name: "条件必填",
    engine: "扩展规则 AST",
    expressible: false,
    guidance:
      "条件必填（如「物品类型为装备时必须有装备部位」）是蕴含式规则，"
      + "当前表达式引擎只支持单条件比较，无法表达；请选择 Python 扩展"
      + "（P2-2）或专门的 Rust 校验器（P3-2）实现，本界面不会假装能自动转换。",
  },
  {
    id: "growth",
    name: "数值成长",
    engine: "规则模板或 Python",
    expressible: false,
    guidance:
      "数值成长（如「攻击力随等级满足公式」）需要算术/函数，当前表达式"
      + "引擎没有数学运算；请用 Python 扩展（P2-2）或规则包（P3-2）表达。",
  },
  {
    id: "cross_table_sum",
    name: "跨表总量",
    engine: "Rust 内置扩展或 Python",
    expressible: false,
    guidance:
      "跨表总量（如「一组奖励权重总和为 100」）是对多行做聚合，超出单条"
      + "表达式；请用 Python 扩展（P2-2）或专门的 Rust 校验器（P3-2）。",
  },
  {
    id: "reachability",
    name: "配置可达性",
    engine: "Python 或专门校验器",
    expressible: false,
    guidance:
      "配置可达性（如「技能升级链不能出现环」）是图上的可达性检查，超出"
      + "表达式引擎；请用 Python 扩展（P2-2）或专门的 Rust 校验器（P3-2）。",
  },
];

export function templateById(id) {
  return RULE_TEMPLATES.find((t) => t.id === id);
}

/// 字段是否存在（供模板参数校验与错误提示）。
function getField(doc, table, field) {
  return doc && doc.tables && doc.tables[table] && doc.tables[table].fields
    && doc.tables[table].fields[field];
}

/// 规划一次模板应用。返回：
///   {kind:"patches", patches:[{path, value}], summary}
///   {kind:"compile", ast, meta, summary}     —— 字段比较，交 /api/rule/compile
///   {kind:"boundary", guidance}              —— 引擎表达不了，只给引导
///   {kind:"error", error}
export function planTemplate(tplId, params, doc) {
  const tpl = templateById(tplId);
  if (!tpl) return { kind: "error", error: `未知模板 ${tplId}` };
  if (!tpl.expressible) return { kind: "boundary", guidance: tpl.guidance };

  const table = params.table;
  const field = params.field;
  if (!table || !field) return { kind: "error", error: "模板需要表和字段" };
  if (!getField(doc, table, field)) {
    return { kind: "error", error: `未知字段 ${table}.${field}` };
  }
  const fieldPath = ["tables", table, "fields", field];

  switch (tplId) {
    case "primary_key": {
      const t = doc.tables[table];
      const pk = Array.isArray(t.primary_key) ? t.primary_key : [];
      if (pk.includes(field)) {
        return { kind: "error", error: `${field} 已在主键中` };
      }
      return {
        kind: "patches",
        patches: [{ path: ["tables", table, "primary_key"], value: [...pk, field] }],
        summary: `把 ${field} 加入主键（${[...pk, field].join(", ")}）`,
      };
    }
    case "range": {
      const min = params.min, max = params.max;
      if (min === undefined || min === "" || max === undefined || max === "") {
        return { kind: "error", error: "范围模板需要 min 与 max" };
      }
      return {
        kind: "patches",
        patches: [
          { path: fieldPath.concat("min"), value: Number(min) },
          { path: fieldPath.concat("max"), value: Number(max) },
        ],
        summary: `${field} ∈ [${min}, ${max}]`,
      };
    }
    case "enum": {
      const en = params.enumName;
      if (!en) return { kind: "error", error: "枚举模板需要选择枚举" };
      return {
        kind: "patches",
        patches: [{ path: fieldPath.concat("type"), value: { kind: "Enum", value: en } }],
        summary: `${field} 类型改为枚举 ${en}`,
      };
    }
    case "reference": {
      const rt = params.refTable, rf = params.refField;
      if (!rt || !rf) return { kind: "error", error: "外键模板需要目标表与字段" };
      return {
        kind: "patches",
        patches: [
          { path: fieldPath.concat("reference"), value: { table: rt, field: rf } },
        ],
        summary: `${field} 引用 ${rt}.${rf}`,
      };
    }
    case "field_compare": {
      const other = params.otherField;
      const op = params.op || "==";
      if (!other) return { kind: "error", error: "字段比较模板需要另一字段" };
      if (other === field) return { kind: "error", error: "不能与自身比较" };
      return {
        kind: "compile",
        ast: {
          kind: "Compare",
          field,
          op,
          rhs: { kind: "Field", name: other },
        },
        meta: { name: `${field}_${op}_${other}`, message: null, warning_only: false },
        summary: `${field} ${op} ${other}`,
      };
    }
    default:
      return { kind: "error", error: `模板 ${tplId} 未实现` };
  }
}
