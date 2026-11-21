// Unit tests for the nine built-in rule templates (rule_templates.js).
// Zero dependencies, run with:
//   node --test docs/public/editor/rule_templates.test.mjs
import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  RULE_TEMPLATES,
  EXPRESSIVENESS_LIMITS,
  templateById,
  planTemplate,
  boundaryGuidance,
} from './rule_templates.js'

const DOC = {
  tables: {
    Monster: {
      name: 'Monster',
      primary_key: ['id'],
      fields: {
        id: { name: 'id', type: { kind: 'Int32' }, required: true },
        level: { name: 'level', type: { kind: 'Int32' } },
        attack: { name: 'attack', type: { kind: 'Int32' } },
        tier: { name: 'tier', type: { kind: 'String' } },
        best_item: { name: 'best_item', type: { kind: 'Int32' } },
      },
    },
    Item: {
      name: 'Item',
      primary_key: ['id'],
      fields: { id: { name: 'id', type: { kind: 'Int32' } } },
    },
  },
  enums: { Rarity: { name: 'Rarity', values: [] } },
}

test('the nine templates carry the review table’s id, engine and boundary flag', () => {
  assert.equal(RULE_TEMPLATES.length, 9)
  assert.deepEqual(RULE_TEMPLATES.map((t) => t.id), [
    'primary_key', 'range', 'enum', 'reference', 'field_compare',
    'conditional_required', 'growth', 'cross_table_sum', 'reachability',
  ])
  // The five expressible ones are the only ones that may touch the document.
  assert.deepEqual(
    RULE_TEMPLATES.filter((t) => t.expressible).map((t) => t.id),
    ['primary_key', 'range', 'enum', 'reference', 'field_compare']
  )
  // The four unexpressible ones each carry planner guidance.
  for (const t of RULE_TEMPLATES.filter((x) => !x.expressible)) {
    assert.ok(t.guidance.length > 20, `${t.id}: ${t.guidance}`)
  }
  assert.equal(templateById('ghost'), undefined)
})

test('primary_key appends the field to the existing key', () => {
  const r = planTemplate('primary_key', { table: 'Monster', field: 'level' }, DOC)
  assert.equal(r.kind, 'patches')
  assert.deepEqual(r.patches, [{
    path: ['tables', 'Monster', 'primary_key'], value: ['id', 'level'],
  }])
  assert.equal(planTemplate('primary_key', { table: 'Monster', field: 'id' }, DOC).kind, 'error')
})

test('range sets inclusive min and max on the field', () => {
  const r = planTemplate('range', { table: 'Monster', field: 'level', min: 1, max: 20 }, DOC)
  assert.equal(r.kind, 'patches')
  assert.equal(r.patches.length, 2)
  assert.deepEqual(r.patches[0], { path: ['tables', 'Monster', 'fields', 'level', 'min'], value: 1 })
  assert.deepEqual(r.patches[1], { path: ['tables', 'Monster', 'fields', 'level', 'max'], value: 20 })
  assert.equal(planTemplate('range', { table: 'Monster', field: 'level' }, DOC).kind, 'error')
})

test('enum switches the field type onto an existing enum', () => {
  const r = planTemplate('enum', { table: 'Monster', field: 'tier', enumName: 'Rarity' }, DOC)
  assert.deepEqual(r.patches, [{
    path: ['tables', 'Monster', 'fields', 'tier', 'type'],
    value: { kind: 'Enum', value: 'Rarity' },
  }])
  assert.equal(planTemplate('enum', { table: 'Monster', field: 'tier' }, DOC).kind, 'error')
})

test('reference sets a cross-table reference', () => {
  const r = planTemplate('reference', {
    table: 'Monster', field: 'best_item', refTable: 'Item', refField: 'id',
  }, DOC)
  assert.deepEqual(r.patches, [{
    path: ['tables', 'Monster', 'fields', 'best_item', 'reference'],
    value: { table: 'Item', field: 'id' },
  }])
  assert.equal(planTemplate('reference', {
    table: 'Monster', field: 'best_item',
  }, DOC).kind, 'error')
})

test('field_compare yields an AST (the frontend never writes an assert)', () => {
  const r = planTemplate('field_compare', {
    table: 'Monster', field: 'attack', op: '<=', otherField: 'level',
  }, DOC)
  assert.equal(r.kind, 'compile')
  assert.deepEqual(r.ast, {
    kind: 'Compare',
    field: 'attack',
    op: '<=',
    rhs: { kind: 'Field', name: 'level' },
  })
  assert.equal(r.meta.warning_only, false)
  assert.equal(planTemplate('field_compare', {
    table: 'Monster', field: 'attack', otherField: 'attack',
  }, DOC).kind, 'error')
})

test('the four unexpressible templates refuse to auto-convert', () => {
  for (const id of ['conditional_required', 'growth', 'cross_table_sum', 'reachability']) {
    const r = planTemplate(id, { table: 'Monster', field: 'level' }, DOC)
    assert.equal(r.kind, 'boundary', id)
    assert.ok(r.guidance.includes('Python'), `${id}: ${r.guidance}`)
  }
})

test('unknown fields and unknown templates error out', () => {
  assert.equal(planTemplate('range', { table: 'Monster', field: 'ghost', min: 1, max: 2 }, DOC).kind, 'error')
  assert.equal(planTemplate('range', { table: 'Ghost', field: 'id', min: 1, max: 2 }, DOC).kind, 'error')
  assert.equal(planTemplate('ghost', {}, DOC).kind, 'error')
})

test('boundaryGuidance names the concrete refusal and never fakes a conversion', () => {
  const g = boundaryGuidance('字符串同时含两种引号')
  assert.ok(g.heading.includes('当前规则引擎表达不了'), g.heading)
  assert.ok(g.heading.includes('字符串同时含两种引号'), g.heading)
  assert.equal(g.limits, EXPRESSIVENESS_LIMITS)
  assert.ok(g.limits.length >= 4)
  assert.equal(g.routes.length, 2)
  assert.ok(g.routes[0].includes('规则模板'), g.routes[0])
  assert.ok(g.routes[1].includes('Python') && g.routes[1].includes('P2-2'), g.routes[1])
  // A missing reason still reads sensibly.
  assert.ok(boundaryGuidance().heading.includes('该条件'))
})

test('EXPRESSIVENESS_LIMITS is the stable honest list of engine limits', () => {
  assert.deepEqual(EXPRESSIVENESS_LIMITS, [
    '单条件比较：字段 OP 字面量或同行字段——无 AND/OR/NOT 逻辑组合',
    '无算术与函数（如「攻击力 ≤ 等级 × 100」）',
    '无跨行聚合（如「一组奖励权重总和为 100」）',
    '无图上可达性/环检查（如「技能升级链无环」）',
  ])
})
